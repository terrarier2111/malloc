use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use libc::{_SC_PAGESIZE, c_int, MAP_ANON, MREMAP_MAYMOVE, munmap, off64_t, PROT_READ, PROT_WRITE, size_t, sysconf};
use crate::linux::alloc_ref::AllocRef;
use crate::linux::chunk_ref::ChunkRef;
use crate::util::{align_unaligned_ptr_to, round_up_to};

// FIXME: reuse allocations and maybe use sbrk

const NOT_PRESENT: usize = 0;

static PAGE_SIZE: AtomicUsize = AtomicUsize::new(NOT_PRESENT);

static FREE_CHUNK_ROOT: AtomicPtr<()> = AtomicPtr::new(null_mut()); // this is the root for the implicit RB-tree

/// `chunk_start` indicates the start of the chunk (including metadata)
/// `size` indicates the number of bytes available
fn push_free_chunk(chunk_start: *mut u8, size: usize) {
    let curr_start_chunk = FREE_CHUNK_ROOT.load(Ordering::Acquire).cast::<u8>();
    
}

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
    let page_size = get_page_size();

    let full_size = round_up_to(size + ALLOC_FULL_INITIAL_METADATA_SIZE + ALLOC_FULL_INITIAL_METADATA_PADDING, page_size);

    let alloc_ptr = map_memory(full_size);
    let mut alloc = AllocRef::new_start(alloc_ptr);
    alloc.setup(full_size, size + CHUNK_METADATA_SIZE);
    let chunk_start = unsafe { alloc_chunk_start(alloc_ptr) };
    let mut chunk = ChunkRef::new_start(chunk_start);
    chunk.setup(size + CHUNK_METADATA_SIZE, true, false);
    if full_size > size + ALLOC_FULL_INITIAL_METADATA_SIZE + ALLOC_FULL_INITIAL_METADATA_PADDING + CHUNK_METADATA_SIZE {
        // FIXME: setup other chunk!
        let mut last_chunk = ChunkRef::new_start(chunk.into_end(size + CHUNK_METADATA_SIZE).into_raw());
        last_chunk.setup(full_size - (size + CHUNK_METADATA_SIZE), false, true);
    } else {
        chunk.set_last(true);
    }
    chunk.into_content_start()
}

#[inline]
pub(crate) fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    let page_size = get_page_size();
    let full_size = round_up_to(size * 2 + ALLOC_FULL_INITIAL_METADATA_SIZE, page_size);
    let alloc_ptr = map_memory(full_size);
    let mut alloc = AllocRef::new_start(alloc_ptr);
    alloc.setup(full_size, size);
    let mut desired_chunk_start = unsafe { align_unaligned_ptr_to::<ALLOC_METADATA_SIZE_ONE_SIDE>(alloc_ptr, full_size - ALLOC_METADATA_SIZE_ONE_SIDE, align).sub(ALLOC_METADATA_SIZE_ONE_SIDE) };
    if (desired_chunk_start as usize - alloc_ptr as usize) < ALLOC_METADATA_SIZE_ONE_SIDE + CHUNK_METADATA_SIZE {
        desired_chunk_start = unsafe { desired_chunk_start.add(size) };
    }
    let mut chunk = ChunkRef::new_start(desired_chunk_start);
    chunk.setup(size, false, false);
    let mut first_chunk = ChunkRef::new_start(unsafe { alloc_ptr.add(ALLOC_METADATA_SIZE_ONE_SIDE) });
    let first_chunk_size = first_chunk.read_size();
    first_chunk.setup(first_chunk_size, true, false);
    first_chunk.set_free(true);
    let last_chunk_start = chunk.into_end(size).into_raw();
    let mut last_chunk = ChunkRef::new_start(last_chunk_start);
    let last_chunk_size = last_chunk.read_size();
    last_chunk.setup(last_chunk_size, false, true);
    last_chunk.set_free(true);

    chunk.into_content_start()
}

#[inline]
pub(crate) fn dealloc(ptr: *mut u8) {
    let mut chunk = ChunkRef::new_start(unsafe { ptr.sub(CHUNK_METADATA_SIZE) });
    let chunk_size = chunk.read_size();

    if chunk.is_first() {
        if chunk.is_last() {
            unreachable!();
        }
        let alloc = AllocRef::new_start(unsafe { chunk.into_raw().sub(ALLOC_METADATA_SIZE_ONE_SIDE) });
        let alloc_size = alloc.read_size();
        let mut right_chunk = chunk.into_end(chunk_size);
        if !right_chunk.is_free() {
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        let overall_size = chunk_size + right_chunk.read_size();
        if overall_size + ALLOC_METADATA_SIZE == alloc_size {
            unmap_memory(alloc.into_raw(), alloc_size);
            return;
        }
        right_chunk.set_first(true); // FIXME: just update right metadata
        right_chunk.update_size(overall_size); // FIXME: just update right metadata
        chunk.update_size(overall_size); // FIXME: just update left metadata
        return;
    } else if chunk.is_last() {
        // FIXME: try merging with left chunk
        let alloc = AllocRef::new_end(unsafe { chunk.into_end(chunk_size).into_raw().add(ALLOC_METADATA_SIZE_ONE_SIDE) });
        let alloc_size = alloc.read_size();
        let mut left_chunk = ChunkRef::new_end(unsafe { alloc.into_raw().sub(ALLOC_METADATA_SIZE_ONE_SIDE) });
        if !left_chunk.is_free() {
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        let overall_size = chunk_size + left_chunk.read_size();
        if overall_size + ALLOC_METADATA_SIZE == alloc_size {
            unmap_memory(alloc.into_start(alloc_size).into_raw(), alloc_size);
            return;
        }
        left_chunk.set_last(true); // FIXME: just update left metadata
        left_chunk.update_size(overall_size); // FIXME: just update left metadata
        chunk.update_size(overall_size); // FIXME: just update right metadata
        return;
    } else {

    }
}

#[inline]
pub(crate) fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, _new_align: usize) -> *mut u8 {
    remap_memory(ptr, old_size, new_size)
}

#[inline]
fn map_memory(size: usize) -> *mut u8 {
    const MMAP_SYSCALL_ID: usize = 9;

    let ptr;

    unsafe {
        asm!(
            "syscall",
            in("rdi") null_mut::<c_void>(),
            in("rsi") size as size_t,
            in("rdx") PROT_READ | PROT_WRITE,
            in("r10") MAP_ANON,
            in("r8") -1 as c_int,
            in("r9") 0 as off64_t,
            inlateout("rax") MMAP_SYSCALL_ID => ptr
        );
    }
    ptr
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

const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE * 2;
const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>() * 2;
const CHUNK_METADATA_SIZE: usize = CHUNK_METADATA_SIZE_ONE_SIDE * 2;
const CHUNK_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;
const ALLOC_FULL_INITIAL_METADATA_PADDING: usize = MIN_ALIGN - (ALLOC_FULL_INITIAL_METADATA_SIZE % MIN_ALIGN);

#[inline]
unsafe fn alloc_chunk_start(alloc: *mut u8) -> *mut u8 {
    alloc.add(ALLOC_METADATA_SIZE)
}


mod alloc_ref {

    #[derive(Copy, Clone)]
    pub(crate) struct AllocRef<const START: bool>(*mut u8);

    impl<const START: bool> AllocRef<START> {

        #[inline]
        pub(crate) fn into_raw(self) -> *mut u8 {
            self.0
        }

    }

    impl AllocRef<true> {

        #[inline]
        pub(crate) fn new_start(alloc_start: *mut u8) -> Self {
            Self(alloc_start)
        }

        #[inline]
        pub(crate) fn setup(&mut self, size: usize, chunk_size: usize) {
            self.setup_own(size, chunk_size);
            self.into_end(size).setup_own(size, chunk_size);
        }

        #[inline]
        fn setup_own(&mut self, size: usize, chunk_size: usize) {
            self.write_size(size);
            self.write_max_chunk_size(chunk_size);
        }

        #[inline]
        pub(crate) fn read_size(&self) -> usize {
            unsafe { *self.0.cast::<usize>() }
        }

        #[inline]
        pub(crate) fn write_size(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>() = size; }
        }

        #[inline]
        pub(crate) fn read_max_chunk_size(&self) -> usize {
            unsafe { *self.0.cast::<usize>().add(1) }
        }

        #[inline]
        pub(crate) fn write_max_chunk_size(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>().add(1) = size; }
        }

        #[inline]
        pub(crate) fn into_end(self, size: usize) -> AllocRef<false> {
            AllocRef::new_end(unsafe { self.0.add(size) })
        }

    }

    impl AllocRef<false> {

        #[inline]
        pub(crate) fn new_end(alloc_end: *mut u8) -> Self {
            Self(alloc_end)
        }

        #[inline]
        pub(crate) fn setup(&mut self, size: usize, chunk_size: usize) {
            self.setup_own(size, chunk_size);
            self.into_start(size).setup_own(size, chunk_size);
        }

        #[inline]
        fn setup_own(&mut self, size: usize, chunk_size: usize) {
            self.write_size(size);
            self.write_max_chunk_size(chunk_size);
        }

        #[inline]
        pub(crate) fn read_size(&self) -> usize {
            unsafe { *self.0.cast::<usize>().sub(1) }
        }

        #[inline]
        pub(crate) fn write_size(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>().sub(1) = size; }
        }

        #[inline]
        pub(crate) fn read_max_chunk_size(&self) -> usize {
            unsafe { *self.0.cast::<usize>().sub(2) }
        }

        #[inline]
        pub(crate) fn write_max_chunk_size(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>().sub(2) = size; }
        }

        #[inline]
        pub(crate) fn into_start(self, size: usize) -> AllocRef<true> {
            AllocRef::new_start(unsafe { self.0.sub(size) })
        }

    }

}

mod chunk_ref {
    use crate::linux::CHUNK_METADATA_SIZE;

    // FIXME: assume that we have alignment >= 2
    const FIRST_CHUNK_FLAG: usize = 1 << 0;
    const LAST_CHUNK_FLAG: usize = 1 << 1;
    const FREE_CHUNK_FLAG: usize = 1 << 2;

    /// `START` indicates whether the stored reference is a reference to the chunk's start or end.
    #[derive(Copy, Clone)]
    pub(crate) struct ChunkRef<const START: bool>(*mut u8);

    impl<const START: bool> ChunkRef<START> {

        #[inline]
        pub(crate) fn into_raw(self) -> *mut u8 {
            self.0
        }

    }

    impl ChunkRef<true> {

        #[inline]
        pub(crate) fn new_start(chunk_start: *mut u8) -> Self {
            Self(chunk_start)
        }

        #[inline]
        pub(crate) fn setup(&mut self, size: usize, first_chunk: bool, last_chunk: bool) {
            self.setup_own(size, first_chunk, last_chunk);
            self.into_end(size).setup_own(size, first_chunk, last_chunk);
        }

        #[inline]
        fn setup_own(&mut self, size: usize, first_chunk: bool, last_chunk: bool) {
            let size = if first_chunk {
                size | FIRST_CHUNK_FLAG
            } else {
                size
            };
            let size = if last_chunk {
                size | LAST_CHUNK_FLAG
            } else {
                size
            };
            self.write_size_raw(size);
        }

        #[inline]
        pub(crate) fn read_size(&self) -> usize {
            (self.read_size_raw() & !FIRST_CHUNK_FLAG)
        }

        #[inline]
        fn read_size_raw(&self) -> usize {
            unsafe { *self.0.cast::<usize>() }
        }

        /// this doesn't modify the FIRST_CHUNK flag
        #[inline]
        pub(crate) fn update_size(&mut self, size: usize) {
            let first_chunk_flag = self.read_size_raw() & FIRST_CHUNK_FLAG;
            self.write_size_raw(size | first_chunk_flag);
        }

        #[inline]
        fn write_size_raw(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>() = size; }
        }

        #[inline]
        pub(crate) fn set_first(&mut self, first: bool) {
            if first {
                self.write_size_raw(self.read_size_raw() | FIRST_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !FIRST_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_first(&self) -> bool {
            self.read_size() & FIRST_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn set_last(&mut self, last: bool) {
            if last {
                self.write_size_raw(self.read_size_raw() | LAST_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !LAST_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_last(&self) -> bool {
            self.read_size() & LAST_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn set_free(&mut self, free: bool) {
            if free {
                self.write_size_raw(self.read_size_raw() | FREE_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !FREE_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_free(&self) -> bool {
            self.read_size() & FREE_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn into_end(self, size: usize) -> ChunkRef<false> {
            ChunkRef(unsafe { self.0.add(size) })
        }

        #[inline]
        pub(crate) fn into_content_start(self) -> *mut u8 {
            unsafe { self.0.add(CHUNK_METADATA_SIZE) }
        }

    }

    impl ChunkRef<false> {

        #[inline]
        pub(crate) fn new_end(alloc_end: *mut u8) -> Self {
            Self(alloc_end)
        }

        #[inline]
        pub(crate) fn setup(&mut self, size: usize, first_chunk: bool, last_chunk: bool) {
            self.setup_own(size, first_chunk, last_chunk);
            self.into_start(size).setup_own(size, first_chunk, last_chunk);
        }

        #[inline]
        fn setup_own(&mut self, size: usize, first_chunk: bool, last_chunk: bool) {
            let size = if first_chunk {
                size | FIRST_CHUNK_FLAG
            } else {
                size
            };
            let size = if last_chunk {
                size | LAST_CHUNK_FLAG
            } else {
                size
            };
            self.write_size_raw(size);
        }

        #[inline]
        pub(crate) fn read_size(&self) -> usize {
            (self.read_size_raw() & !FIRST_CHUNK_FLAG)
        }

        #[inline]
        fn read_size_raw(&self) -> usize {
            unsafe { *self.0.cast::<usize>().sub(1) }
        }

        #[inline]
        pub(crate) fn write_size<const FIRST_CHUNK: bool>(&mut self, size: usize) {

        }

        /// this doesn't modify the FIRST_CHUNK flag
        #[inline]
        pub(crate) fn update_size(&mut self, size: usize) {
            let first_chunk_flag = self.read_size_raw() & FIRST_CHUNK_FLAG;
            self.write_size_raw(size | first_chunk_flag);
        }

        #[inline]
        fn write_size_raw(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>().sub(1) = size; }
        }

        #[inline]
        pub(crate) fn set_first(&mut self, first: bool) {
            if first {
                self.write_size_raw(self.read_size_raw() | FIRST_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !FIRST_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_first(&self) -> bool {
            self.read_size() & FIRST_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn set_last(&mut self, last: bool) {
            if last {
                self.write_size_raw(self.read_size_raw() | LAST_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !LAST_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_last(&self) -> bool {
            self.read_size() & LAST_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn set_free(&mut self, free: bool) {
            if free {
                self.write_size_raw(self.read_size_raw() | FREE_CHUNK_FLAG);
            } else {
                self.write_size_raw(self.read_size_raw() & !FREE_CHUNK_FLAG);
            }
        }

        #[inline]
        pub(crate) fn is_free(&self) -> bool {
            self.read_size() & FREE_CHUNK_FLAG != 0
        }

        #[inline]
        pub(crate) fn into_start(self, size: usize) -> ChunkRef<true> {
            ChunkRef(unsafe { self.0.sub(size) })
        }

    }

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
