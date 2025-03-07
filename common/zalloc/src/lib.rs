#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! Implementation of a Zeroizing Allocator, enabling zeroizing memory on deallocation.
//! This can either be used with Box (requires nightly and the "allocator" feature) to provide the
//! functionality of zeroize on types which don't implement zeroize, or used as a wrapper around
//! the global allocator to ensure *all* memory is zeroized.

use core::{
  slice,
  alloc::{Layout, GlobalAlloc},
};

use zeroize::Zeroize;

/// An allocator wrapper which zeroizes its memory on dealloc.
pub struct ZeroizingAlloc<T>(pub T);

unsafe impl<T: GlobalAlloc> GlobalAlloc for ZeroizingAlloc<T> {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    self.0.alloc(layout)
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    slice::from_raw_parts_mut(ptr, layout.size()).zeroize();
    self.0.dealloc(ptr, layout);
  }
}
