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

pub fn alloc_promise_sized(size: usize) -> *mut u8 {
    // FIXME: use more efficient impl!
    sys::alloc(size)
}

// #[no_mangle]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    sys::alloc_aligned(size, align)
}

pub fn alloc_aligned_promise_sized(size: usize, align: usize) -> *mut u8 {
    // FIXME: use more efficient impl!
    sys::alloc_aligned(size, align)
}

/// This function may only be used with pointers returned from
/// `alloc` or `alloc_aligned`
/// otherwise calling this function will result in
/// undefined behavior.
pub fn free(ptr: *mut u8) {
    sys::free(ptr);
}

/// This function may only be used with pointers returned from
/// `alloc` or `alloc_aligned` otherwise calling this function will result in
/// undefined behavior.
/// Compared to `free` the additional `size` context may improve
/// dealloction performance.
pub fn free_ctx_sized(ptr: *mut u8, size: usize) {
    sys::free(ptr);
}

/// This function may only be used with pointers returned from
/// `alloc_promise_sized` or `alloc_aligned_promise_sized`
/// otherwise calling this function will result in
/// undefined behavior.
pub fn free_promise_sized(ptr: *mut u8, size: usize) {
    // FIXME: use more efficient impl!
    sys::free(ptr);
}

/// This function may only be used with pointers returned from
/// `alloc` or `alloc_aligned`
/// otherwise calling this function will result in
/// undefined behavior.
pub fn realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
    todo!()
}

/// This function may only be used with pointers returned from
/// `alloc_promise_sized` or `alloc_aligned_promise_sized`
/// otherwise calling this function will result in
/// undefined behavior.
pub fn realloc_promise_sized(ptr: *mut u8, old_size: usize, new_size: usize, new_align: usize) -> *mut u8 {
    sys::realloc(ptr, old_size, new_size, new_align)
}
