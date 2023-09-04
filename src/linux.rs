use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicUsize, Ordering};
use libc::{_SC_PAGESIZE, c_int, MAP_ANON, MREMAP_MAYMOVE, off64_t, PROT_READ, PROT_WRITE, size_t, sysconf};

// FIXME: reuse allocations and maybe use sbrk

const NOT_PRESENT: usize = 0;

static PAGE_SIZE: AtomicUsize = AtomicUsize::new(NOT_PRESENT);

#[inline]
fn get_page_size() -> usize {
    let cached = PAGE_SIZE.load(Ordering::Relaxed);
    if cached != NOT_PRESENT {
        return cached;
    }
    let resolved = unsafe { sysconf(_SC_PAGESIZE) as usize };
    PAGE_SIZE.store(resolved, Ordering::Relaxed);
    resolved
}

/// returns memory aligned to ptr size
#[inline]
pub(crate) fn alloc(size: usize) -> *mut u8 {
    if size % get_page_size() == 0 {
        return map_memory(size);
    }

    let full_size = size + ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

    let alloc = map_memory(full_size);
    unsafe { write_alloc_size(alloc, full_size); }
    // we don't support free chunks yet
    unsafe { write_alloc_max_chunk_size(alloc, 0); }
    let chunk_start = unsafe { chunk_start(alloc) };
    unsafe { write_chunk_size(chunk_start, size); }
    alloc
}

#[inline]
pub(crate) fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    let final_size = size * 2 + ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

}

#[inline]
pub(crate) fn dealloc(ptr: *mut u8) {
    let size = unsafe { read_chunk_size(ptr) };
    unmap_memory(ptr, size);
}

#[inline]
pub(crate) fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, _new_align: usize) -> *mut u8 {
    remap_memory(ptr, old_size, new_size)
}

fn map_memory(size: usize) -> *mut u8 {
    unsafe { libc::mmap64(null_mut(), size as size_t, PROT_READ | PROT_WRITE, MAP_ANON, -1 as c_int, 0 as off64_t) }.cast::<u8>() // FIXME: can we handle return value?
}

fn unmap_memory(ptr: *mut u8, size: usize) {
    let result = unsafe { libc::munmap(ptr.cast::<c_void>(), size as size_t) };
    if result != 0 {
        // we can't handle this error properly, so just abort the process
        core::intrinsics::abort();
    }
}

fn remap_memory(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    unsafe { libc::mremap(ptr.cast::<c_void>(), old_size, new_size, MREMAP_MAYMOVE) }.cast::<u8>() // FIXME: can we handle return value?
}

const ALLOC_METADATA_SIZE: usize = size_of::<usize>() * 2;
const CHUNK_METADATA_SIZE: usize = size_of::<usize>();

#[inline]
unsafe fn read_alloc_size(alloc: *mut u8) -> usize {
    unsafe { *alloc.cast::<usize>() }
}

#[inline]
unsafe fn read_alloc_max_chunk_size(alloc: *mut u8) -> usize {
    unsafe { *alloc.cast::<usize>().add(1) }
}

#[inline]
unsafe fn write_alloc_size(alloc: *mut u8, size: usize) {
    unsafe { *alloc.cast::<usize>() = size; }
}

#[inline]
unsafe fn write_alloc_max_chunk_size(alloc: *mut u8, max_chunk_size: usize) {
    unsafe { *alloc.cast::<usize>().add(1) = max_chunk_size; }
}

#[inline]
unsafe fn read_chunk_size(chunk: *mut u8) -> usize {
    unsafe { *chunk.cast::<usize>() }
}

#[inline]
unsafe fn write_chunk_size(chunk: *mut u8, size: usize) {
    unsafe { *chunk.cast::<usize>() = size; }
}

#[inline]
unsafe fn chunk_start(alloc: *mut u8) -> *mut u8 {
    alloc.add(ALLOC_METADATA_SIZE)
}

// The minimum alignment guaranteed by the architecture. This value is used to
// add fast paths for low alignment values.
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
