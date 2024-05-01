use transcript::Transcript;

use ciphersuite::{group::ff::PrimeField, Ciphersuite};

use crate::{
  *,
  gadgets::{CurveSpec, OnCurve, incomplete_add},
};

/// A representation of the divisor.
///
/// The coefficient for x**1 is explicitly excluded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Divisor {
  y: Variable,
  yx: Vec<Variable>,
  x_from_power_of_2: Vec<Variable>,
  zero: Variable,
}

impl<C: Ciphersuite> Circuit<C> {
  fn tuple_member_of_list<T: Transcript>(
    &mut self,
    transcript: &mut T,
    member: Vec<Variable>,
    list: Vec<Vec<Variable>>,
  ) {
    // Ensure this is being safely called
    for variable in member.iter().chain(list.iter().flatten()) {
      assert!(
        matches!(variable, Variable::C(_, _)),
        "tuple member of set requires all arguments belong to vector commitments"
      );
    }

    // Create challenges which we use to aggregate tuples into LinCombs
    let mut challenges = vec![];
    for _ in 0 .. member.len() {
      challenges
        .push(C::hash_to_F(b"fcmp", transcript.challenge(b"tuple_member_of_list").as_ref()));
    }

    // Aggregate the claimed member
    let member = {
      let mut res = LinComb::empty();
      for (i, member) in member.into_iter().enumerate() {
        res = res + &(LinComb::from(member) * challenges[i]);
      }
      res
    };

    // Aggregate the list members
    let list = {
      let mut res = vec![];
      for list_tuple in list {
        let mut item = LinComb::empty();
        for (i, list_member) in list_tuple.into_iter().enumerate() {
          item = item + &(LinComb::from(list_member) * challenges[i]);
        }
        res.push(item);
      }
      res
    };

    // Run traditional set membership
    self.member_of_list(member, list)
  }

  fn divisor_challenge_invert(&mut self, scalar: C::F) -> C::F {
    let res = Option::from(scalar.invert());
    // If somehow, we are trying to invert zero, push a constraint requiring -1 = 0
    // This will cause the proof to fail to verify
    // TODO: Properly propagate this error
    if res.is_none() {
      self.constraints.push(LinComb::empty().constant(C::F::ONE));
      return C::F::ONE;
    }
    res.unwrap()
  }

  fn divisor_challenge(
    &mut self,
    curve: &CurveSpec<C::F>,
    divisor: &Divisor,
    slope: C::F,
    c_x: C::F,
    c_y: C::F,
  ) -> Variable {
    let c_x_sq = c_x * c_x;
    let three_x_sq = c_x_sq + c_x_sq + c_x_sq;
    let three_x_sq_plus_a = three_x_sq + curve.a;
    let two_c_y = c_y + c_y;

    let p_0_n_0 = three_x_sq_plus_a * self.divisor_challenge_invert(two_c_y);

    // The evaluation of the divisor differentiated by y, further multiplied by p_0_n_0
    // Differentation drops everything without a y coefficient, and drops what remains by a power
    // of y
    // (y**1 -> y**0, yx**i -> x**i)
    let p_0_n_1 = {
      let mut p_0_n_1 = LinComb::empty().term(p_0_n_0, divisor.y);
      let mut c_x_eval = c_x;
      for var in &divisor.yx {
        p_0_n_1 = p_0_n_1.term(p_0_n_0 * c_x_eval, *var);
        c_x_eval *= c_x;
      }
      p_0_n_1
    };

    // The evaluation of the divisor differentiated by x
    let p_0_n_2 = {
      // The coefficient for x**1 is 1, so 1 becomes the new zero coefficient
      // Since the constant is subtracted, we use -1 for the constant (making it +1)
      let mut p_0_n_2 = LinComb::empty().constant(-C::F::ONE);

      // Handle the new y coefficient
      p_0_n_2 = p_0_n_2.term(c_y, divisor.yx[0]);

      // Handle the new yx coefficients
      let mut c_yx = c_y * c_x;
      for (j, yx) in divisor.yx.iter().enumerate().skip(1) {
        // For the power which was shifted down, we multiply this coefficient
        // 3 x**2 -> 2 * 3 x**1
        let original_power_of_x = j + 1;
        // Use incremental addition for this multiplication
        // For such a small weight, it's faster than any constant time operation
        let mut this_weight = c_yx;
        for _ in 1 .. original_power_of_x {
          this_weight += c_yx;
        }

        p_0_n_2 = p_0_n_2.term(this_weight, *yx);

        c_yx *= c_x;
      }

      // Handle the x coefficients
      let mut c_x_eval = c_x;
      // We don't skip the first one as `x_from_power_of_2` already omits x**1
      for (i, x) in divisor.x_from_power_of_2.iter().enumerate() {
        let original_power_of_x = i + 2;
        // Use incremental addition for this multiplication
        // For such a small weight, it's faster than any constant time operation
        let mut this_weight = c_x_eval;
        for _ in 1 .. original_power_of_x {
          this_weight += c_x_eval;
        }

        p_0_n_2 = p_0_n_2.term(this_weight, *x);

        c_x_eval *= c_x;
      }

      p_0_n_2
    };

    let p_0_n = p_0_n_1 + &p_0_n_2;

    let p_0_d = {
      let mut p_0_d = LinComb::empty().term(c_y, divisor.y);

      let mut c_yx = c_y * c_x;
      for var in &divisor.yx {
        p_0_d = p_0_d.term(c_yx, *var);
        c_yx *= c_x;
      }

      let mut c_x_eval = c_x * c_x;
      for var in &divisor.x_from_power_of_2 {
        p_0_d = p_0_d.term(c_x_eval, *var);
        c_x_eval *= c_x;
      }

      // We add -c_x to c so this evaluates as `+ c_x` (since c is subtracted from the linear
      // combination)
      // Adding c_x effectively adds a `1 x` term, ensuring the divisor isn't 0
      p_0_d.term(C::F::ONE, divisor.zero).constant(-c_x)
    };

    let p_1_n = two_c_y;
    let p_1_d = (-slope * p_1_n) + three_x_sq_plus_a;

    // Calculate the joint numerator
    let p_n = p_0_n * p_1_n;
    // Calculate the joint denominator
    let p_d = p_0_d * p_1_d;

    // We want `n / d = o`
    // `n / d = o` == `n = d * o`
    // This is a safe unwrap as it's solely done by the prover and should always be non-zero
    let witness =
      self.eval(&p_d).map(|p_d| (p_d, self.eval(&p_n).unwrap() * p_d.invert().unwrap()));
    let (_l, o, n_claim) = self.mul(Some(p_d), None, witness);
    self.equality(p_n, &n_claim.into());
    o
  }

  /// Prove the claimed discrete logarithm is a consistent representation of the discrete log
  /// for the specified point over the specified generator.
  ///
  /// The discrete log representation must be treated as a non-canonical, opaque black box. A
  /// discrete log has effectively infinite representations within this black box. The only
  /// guarantee is that the discrete log proven for is always equivalent to any other discrete log
  /// proven for with this exact representation.
  ///
  /// Ensures the point is on-curve.
  fn discrete_log<T: Transcript>(
    &mut self,
    transcript: &mut T,
    curve: &CurveSpec<C::F>,
    mut generator_x: C::F,
    mut generator_y: C::F,
    divisor: Divisor,
    dlog: Vec<Variable>,
    x: Variable,
    y: Variable,
  ) -> OnCurve {
    assert_eq!(dlog.len(), <C::F as PrimeField>::CAPACITY.try_into().unwrap());

    // Ensure this is being safely called
    for variable in dlog.iter().chain(core::iter::once(&x)).chain(core::iter::once(&y)) {
      assert!(
        matches!(variable, Variable::C(_, _)),
        "discrete log requires all arguments belong to vector commitments",
      );
    }

    let point = self.on_curve(curve, x, y);

    // TODO: Implement a proper hash to curve
    let (c0_x, c0_y) = loop {
      let c0_x = C::hash_to_F(b"fcmp", transcript.challenge(b"discrete_log_0").as_ref());
      let Some(c0_y) =
        Option::<C::F>::from(((c0_x * c0_x * c0_x) + (curve.a * c0_x) + curve.b).sqrt())
      else {
        continue;
      };
      break (c0_x, if bool::from(c0_y.is_odd()) { -c0_y } else { c0_y });
    };
    let (c1_x, c1_y) = loop {
      let c1_x = C::hash_to_F(b"fcmp", transcript.challenge(b"discrete_log_0").as_ref());
      if c0_x == c1_x {
        continue;
      }
      let Some(c1_y) =
        Option::<C::F>::from(((c1_x * c1_x * c1_x) + (curve.a * c1_x) + curve.b).sqrt())
      else {
        continue;
      };
      break (c1_x, if bool::from(c1_y.is_odd()) { -c1_y } else { c1_y });
    };

    // mdbl-2007-bl
    let double = |circuit: &mut Self, x: C::F, y: C::F| {
      let xx = x * x;
      let w = curve.a + (xx.double() + xx);
      let y1y1 = y * y;
      let r = y1y1.double();
      let y1r = y * r;
      let sss = y1r.double().double();
      let rr = r * r;
      let b = (x + r).square() - xx - rr;
      let h = (w * w) - b.double();
      let x3 = h.double() * y;
      let y3 = (w * (b - h)) - rr.double();
      let z3 = sss;

      let z3_inv = circuit.divisor_challenge_invert(z3);
      (x3 * z3_inv, y3 * z3_inv)
    };

    let (c2_x, c2_y) = incomplete_add::<C::F>(c0_x, c0_y, c1_x, c1_y)
      .expect("couldn't perform incomplete addition on two distinct, on curve points");

    let slope = (c1_y - c0_y) * self.divisor_challenge_invert(c1_x - c0_x);
    let intercept = c1_y - (slope * c1_x);

    // lhs from the paper, evaluating the divisor
    let mut eval = LinComb::from(self.divisor_challenge(curve, &divisor, slope, c0_x, c0_y)) +
      &LinComb::from(self.divisor_challenge(curve, &divisor, slope, c1_x, c1_y)) +
      &LinComb::from(self.divisor_challenge(curve, &divisor, slope, c2_x, c2_y));

    // Interpolate the powers of the generator
    for bit in dlog {
      let weight = self.divisor_challenge_invert(intercept - (generator_y - (slope * generator_x)));
      eval = eval.term(-weight, bit);
      // TODO: Take in a table as an argument
      (generator_x, generator_y) = double(self, generator_x, generator_y);
    }

    // Interpolate the output point
    // intercept - (y - (slope * x))
    // intercept - y + (slope * x))
    // -y + (slope * x)) + intercept
    // We use -intercept since constants are subtracted
    let output_interpolation =
      LinComb::empty().constant(-intercept).term(-C::F::ONE, y).term(slope, x);
    let output_interpolation_eval = self.eval(&output_interpolation);
    let (_output_interpolation, inverse) =
      self.inverse(Some(output_interpolation), output_interpolation_eval);
    eval = eval.term(-C::F::ONE, inverse);

    self.constraints.push(eval);

    point
  }

  /// Prove knowledge of the discrete logarithm for the specified point over the specified
  /// generator.
  ///
  /// The variable used as knowledge of the discrete log representation must be treated as a
  /// non-canonical, opaque black box which is inconsistent across uses (and accordingly unsafe to
  /// reuse).
  ///
  /// Ensures the point is on-curve.
  fn discrete_log_pok<T: Transcript>(
    &mut self,
    transcript: &mut T,
    curve: &CurveSpec<C::F>,
    generator_x: C::F,
    generator_y: C::F,
    divisor: Divisor,
    dlog_knowledge: Vec<Variable>,
    x: Variable,
    y: Variable,
  ) -> OnCurve {
    // For now, we use the more expensive Discrete Log instead of attempting any more optimized
    // versions of this gadget
    self.discrete_log(transcript, curve, generator_x, generator_y, divisor, dlog_knowledge, x, y)
  }
}
