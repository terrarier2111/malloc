// #![no_std]

#![feature(core_intrinsics)]

mod linux;
mod util;

// #[no_mangle]
pub fn alloc(size: usize) -> *mut u8 {
    if cfg!(target_os = "linux") {
        linux::alloc(size)
    } else {
        core::intrinsics::abort();
    }
}

// #[no_mangle]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    if cfg!(target_os = "linux") {
        linux::alloc_aligned(size, align)
    } else {
        core::intrinsics::abort();
    }
}

// #[no_mangle]
pub fn dealloc(ptr: *mut u8) {
    if ptr.is_null() {
        // this is a noop
        return;
    }
    if cfg!(target_os = "linux") {
        linux::dealloc(ptr);
    } else {
        core::intrinsics::abort();
    }
}

// #[no_mangle]
pub fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, new_align: usize) -> *mut u8 {
    if cfg!(target_os = "linux") {
        linux::realloc(ptr, old_size, new_size, new_align)
    } else {
        core::intrinsics::abort();
    }
}
