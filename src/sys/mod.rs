use std::mem::{align_of, size_of};

use crossbeam_utils::CachePadded;

mod linux;

pub(crate) const CACHE_LINE_SIZE: usize = align_of::<CachePadded<()>>();
pub(crate) const CACHE_LINE_WORD_SIZE: usize = CACHE_LINE_SIZE / size_of::<usize>();

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
pub fn free(ptr: *mut u8) {
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

pub(crate) fn get_page_size() -> usize {
    if cfg!(target_os = "linux") {
        linux::get_page_size()
    } else {
        core::intrinsics::abort();
    }
}

// The minimum alignment guaranteed by the architecture.
#[cfg(all(any(
target_arch = "x86",
target_arch = "arm",
target_arch = "mips",
target_arch = "powerpc",
target_arch = "powerpc64",
target_arch = "asmjs",
target_arch = "wasm32",
target_arch = "hexagon"
)))]
pub(crate) const MIN_ALIGN: usize = 8;
#[cfg(all(any(
target_arch = "x86_64",
target_arch = "aarch64",
target_arch = "mips64",
target_arch = "s390x",
target_arch = "sparc64",
target_arch = "riscv64"
)))]
pub(crate) const MIN_ALIGN: usize = 16;