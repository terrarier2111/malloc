// #![no_std]

#![feature(thread_local)]
#![feature(core_intrinsics)]
#![feature(strict_provenance)]
#![feature(ptr_mask)]
#![feature(int_roundings)]
#![feature(linkage)]

mod util;
mod sys;
mod alloc_ref;
mod chunk_ref;
mod implicit_rb_tree;

// #[no_mangle]
pub fn alloc(size: usize) -> *mut u8 {
    sys::alloc(size)
}

// #[no_mangle]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    sys::alloc_aligned(size, align)
}

// #[no_mangle]
pub fn free(ptr: *mut u8) {
    sys::free(ptr)
}

// #[no_mangle]
pub fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, new_align: usize) -> *mut u8 {
    sys::realloc(ptr, old_size, new_size, new_align)
}
