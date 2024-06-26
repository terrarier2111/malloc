use core::arch::asm;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::cell::UnsafeCell;
use std::mem::{align_of, size_of};
use std::ptr::{null_mut, NonNull};
use libc::{_SC_PAGESIZE, c_int, MAP_ANON, MAP_PRIVATE, off64_t, PROT_READ, PROT_WRITE, size_t, sysconf};
use crate::alloc_ref::{AllocRef, CHUNK_ALLOC_METADATA_SIZE_ONE_SIDE, BUCKET_METADATA_SIZE, CHUNK_ALLOC_METADATA_SIZE, FreeAlloc};
use crate::chunk_ref::meta::ChunkMeta;
use crate::chunk_ref::{CHUNK_METADATA_SIZE, ChunkRef, CHUNK_METADATA_SIZE_ONE_SIDE};
use crate::util::{align_unaligned_ptr_up_to, round_up_to_multiple_of, abort};

use self::bit_map_list::{BitMapListNode, BitMapList, BitMapEntry};
use self::global_free_list::GlobalFreeList;
use self::local_cache::ThreadLocalCache;

use super::CACHE_LINE_WORD_SIZE;

// FIXME: reuse allocations and maybe use sbrk

// FIXME: try avoiding atomic instructions if the freeing thread is the same as the original thread.
// FIXME: we could do this by having 2 bitsets (1 for non-atomic local frees and 1 for atomic non-local frees)

const NOT_PRESENT: usize = 0;

static GLOBAL_FREE_LIST: GlobalFreeList = GlobalFreeList::new(); // this is the root for the implicit global RB-tree

mod global_free_list {
    use std::{ptr::null_mut, sync::atomic::{AtomicPtr, Ordering}};


    pub(crate) struct GlobalFreeList {
        root: AtomicPtr<()>,
    }

    impl GlobalFreeList {

        #[inline]
        pub const fn new() -> Self {
            Self {
                root: AtomicPtr::new(null_mut()),
            }
        }

        #[inline]
        pub fn is_empty(&self) -> bool {
            self.root.load(Ordering::Acquire).is_null()
        }

        /// `chunk_start` indicates the start of the chunk (including metadata)
        /// `size` indicates the chunk size in pages
        pub fn push_free_alloc(&self, alloc_start: *mut u8, size: usize) {
            let mut curr_start_alloc = self.root.load(Ordering::Acquire);
            
        }

    }

}

fn free_global(addr: NonNull<u8>, size: usize) {
     // FIXME: cache the global memory!
    unmap_memory(addr.as_ptr(), size);
}

const LOCAL_CACHE_ENTRIES_MAX: usize = 32;

fn free_local(addr: NonNull<u8>, size: usize) {
    let local = LOCAL_CACHE.get();
    if unsafe { &mut *local }.free_chunk_root.len() >= LOCAL_CACHE_ENTRIES_MAX {
        free_global(addr, size);
        return;
    }
    unsafe { (&mut *local).push_free_chunk(addr.as_ptr().cast::<()>(), size); }
}

// Provides thread-local destructors without an associated "key", which
// can be more efficient.

// Since what appears to be glibc 2.18 this symbol has been shipped which
// GCC and clang both use to invoke destructors in thread_local globals, so
// let's do the same!
//
// Note, however, that we run on lots older linuxes, as well as cross
// compiling from a newer linux to an older linux, so we also have a
// fallback implementation to use as well.
pub(crate) unsafe fn register_dtor(t: *mut u8, dtor: unsafe extern "C" fn(*mut u8)) {
    use core::mem;

    extern "C" {
        #[linkage = "extern_weak"]
        static __dso_handle: *mut u8;
        #[linkage = "extern_weak"]
        static __cxa_thread_atexit_impl: *const libc::c_void;
    }
    if !__cxa_thread_atexit_impl.is_null() {
        type F = unsafe extern "C" fn(
            dtor: unsafe extern "C" fn(*mut u8),
            arg: *mut u8,
            dso_handle: *mut u8,
        ) -> libc::c_int;
        mem::transmute::<*const libc::c_void, F>(__cxa_thread_atexit_impl)(
            dtor,
            t,
            &__dso_handle as *const _ as *mut _,
        );
        return;
    }
    // we can't actually register a handler and we don't have a fallback impl just yet, so simply fail for now.
    abort();
}

#[thread_local]
static LOCAL_CACHE: UnsafeCell<ThreadLocalCache> = UnsafeCell::new(ThreadLocalCache::new());

fn cleanup_tls() {
    // we store the metadata about the size and next ptr in the allocations themselves
    // this means that we will trash the cache more and we have to fetch more things into
    // it but we don't have to use as much memory. We have to load 24 bytes (on 64 bit systems)
    // anyways as we have 2 ptrs to children and one key for traversal.
    // FIXME: is this tradeoff worth it?
    let mut chunk_ref = unsafe { (&mut *LOCAL_CACHE.get()).free_chunk_root.pop_front().cast::<FreeAlloc>() };
    while !chunk_ref.is_null() {
        let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(chunk_ref.cast::<u8>()) });
        let size = alloc.read_free().read_size()/* / PAGE_SIZE*/;
        let next = alloc.read_free().read_next();
        free_global(alloc.into_raw(), size);
        chunk_ref = next;
    }
}

mod local_cache {

    // static NOT_SETUP_SENTINEL: u8 = 0;

    use std::{mem::{MaybeUninit, transmute}, ptr::NonNull};

    use crate::{linked_list::LinkedList, alloc_ref::FreeAlloc};

    use super::bit_map_list::BitMapList;const BUCKETS: usize = 10;
    const BUCKET_ELEM_SIZES: [usize; BUCKETS] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];

    const fn construct_buckets() -> [BitMapList; BUCKETS] {
        // we have to use several nasty workarounds here as many const things are not stable yet.
        let mut buckets = unsafe { MaybeUninit::<[MaybeUninit<BitMapList>; BUCKETS]>::uninit().assume_init() };
        let mut i = 0;
        let base = BitMapList::new_empty();
        while i < BUCKETS {
            unsafe { core::ptr::copy(&base as *const BitMapList, buckets[i].as_ptr().cast_mut(), 1); }
            i += 1;
        }
        unsafe { transmute(buckets) }
    }

    #[repr(C)]
    pub(crate) struct ThreadLocalCache {
        pub(crate) free_chunk_root: LinkedList<usize>/*ImplicitRbTree*/, // this acts as a local cache for free chunks
        pub(crate) buckets: [BitMapList; BUCKETS],
    }

    unsafe impl Send for ThreadLocalCache {}
    unsafe impl Sync for ThreadLocalCache {}

    impl ThreadLocalCache {

        pub const fn new() -> Self {
            Self {
                buckets: construct_buckets(),
                free_chunk_root: LinkedList::new(),
            }
        }

        pub unsafe fn push_free_chunk(&mut self, chunk: *mut (), size: usize) {
            self.free_chunk_root.push(chunk, size);
        }

        pub unsafe fn pop_free_chunk(&mut self) -> Option<FreeAlloc> {
            let ret = self.free_chunk_root.pop_front();
            if ret.is_null() {
                None
            } else {
                Some(FreeAlloc(unsafe { NonNull::new_unchecked(ret.cast::<u8>()) }))
            }
        }

    }
}

const BUCKETS: usize = 12;
const BUCKET_ELEM_SIZES: [usize; BUCKETS] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];
const LARGEST_BUCKET: usize = BUCKET_ELEM_SIZES[BUCKETS - 1];
// the first array is for meta size and the second is for elem cnt
const BUCKET_META_AND_ELEM_CNTS: ([usize; BUCKETS], [usize; BUCKETS]) = {
    let mut ret = ([0; BUCKETS], [0; BUCKETS]);
    const META_BASE_WORD_SIZE: usize = (size_of::<usize>() + size_of::<BitMapListNode>()).div_ceil(size_of::<usize>());
    let mut i = 0;
    while i < BUCKETS {
        // calculate bucket size by first estimating the meta size and then
        // iteratively improving our estimate until we have the perfect values.
        let mut max_elems = PAGE_SIZE / BUCKET_ELEM_SIZES[i];
        // our metadata consists of a bitmap of used values and another atomic bitmap, which has to be aligned to cache line size.
        const BUCKET_META_WORD_SIZE: usize = BUCKET_METADATA_SIZE.div_ceil(size_of::<usize>());
        let mut meta_size = (max_elems.div_ceil(usize::BITS as usize) * 2 + BUCKET_META_WORD_SIZE + META_BASE_WORD_SIZE).next_multiple_of(CACHE_LINE_WORD_SIZE);
        loop {
            let prev = max_elems;
            max_elems = (PAGE_SIZE - meta_size * size_of::<usize>()) / BUCKET_ELEM_SIZES[i];
            // check if we can't improve our estimate, in which case we must already have the perfect values
            if prev == max_elems {
                break;
            }
            meta_size = (max_elems.div_ceil(usize::BITS as usize) * 2 + BUCKET_META_WORD_SIZE + META_BASE_WORD_SIZE).next_multiple_of(CACHE_LINE_WORD_SIZE);
        }
        ret.0[i] = meta_size - BUCKET_META_WORD_SIZE;
        ret.1[i] = max_elems;
        i += 1;
    }
    ret
};
const BUCKET_MAP_CNT: [usize; BUCKETS] = {
    let mut ret = [0; BUCKETS];
    let meta_base_word_size = (size_of::<usize>() + size_of::<BitMapListNode>()).div_ceil(size_of::<usize>());
    let mut i = 0;
    while i < BUCKETS {
        ret[i] = (BUCKET_META_AND_ELEM_CNTS.0[i] - meta_base_word_size) / 2;
        i += 1;
    }
    ret
};

// how many entries are used to store metadata
const META_ENTRIES_USED: [usize; BUCKETS] = {
    let mut ret = [0; BUCKETS];
    const META_BASE_WORD_SIZE: usize = (size_of::<usize>() + size_of::<BitMapListNode>()).div_ceil(size_of::<usize>());
    let mut i = 0;
    while i < BUCKETS {
        ret[i] = (BUCKET_META_AND_ELEM_CNTS.0[i] - META_BASE_WORD_SIZE).div_ceil(BUCKET_ELEM_SIZES[i].div_floor(size_of::<usize>()));
        // static_assertions::const_assert_eq!(ret[i], BUCKET_ELEM_SIZES[i] - BUCKET_META_AND_ELEM_CNTS.1[i]); // FIXME: assert this correctly!
        i += 1;
    }
    ret
};

/// we use an internal page size of 64KB this should be large enough for any
/// non-huge native page size.
const PAGE_SIZE: usize = 1024 * 64;

static OS_PAGE_SIZE: AtomicUsize = AtomicUsize::new(NOT_PRESENT);
static PAGE_OS_PAGE_CNT: AtomicUsize = AtomicUsize::new(NOT_PRESENT);

#[inline]
pub(crate) fn get_page_size() -> usize {
    let cached = OS_PAGE_SIZE.load(Ordering::Relaxed);
    if cached != NOT_PRESENT {
        return cached;
    }
    setup_page_size()
}

#[inline]
pub(crate) fn get_page_in_os_page_cnt() -> usize {
    let cached = PAGE_OS_PAGE_CNT.load(Ordering::Relaxed);
    if cached != NOT_PRESENT {
        return cached;
    }
    PAGE_SIZE.div_ceil(setup_page_size())
}

#[cold]
#[inline(never)]
fn setup_page_size() -> usize {
    let resolved = unsafe { sysconf(_SC_PAGESIZE) as usize };
    OS_PAGE_SIZE.store(resolved, Ordering::Relaxed);
    PAGE_OS_PAGE_CNT.store(PAGE_SIZE.div_ceil(resolved), Ordering::Relaxed);
    resolved
}

fn map_pages(pages: usize) -> *mut u8 {
    let (size, additional) = if pages == 1 {
        (2 * PAGE_SIZE, 1)
    } else {
        ((pages + 2) * PAGE_SIZE, 2)
    };
    let raw = map_memory(size);
    println!("mapped 0");
    // FIXME: is this fast path actually an improvement?
    if raw as usize % PAGE_SIZE == 0 {
        unsafe { (&mut *LOCAL_CACHE.get()).push_free_chunk(raw.add(pages * PAGE_SIZE).cast::<()>(), additional * PAGE_SIZE); }
        return raw;
    }
    let aligned = unsafe { align_unaligned_ptr_up_to(raw, size, PAGE_SIZE, pages * PAGE_SIZE) };
    let front = aligned as usize - raw as usize;
    if front != 0 {
        println!("unmapping {front}");
        unmap_memory(raw, front);
    }
    let back = additional * PAGE_SIZE - front;
    if back != 0 {
        println!("unmapping {back}");
        unmap_memory(unsafe { raw.add(front + pages * PAGE_SIZE) }, back);
    }
    println!("allocec raw: {:?} aligned: {:?}", raw as usize, aligned as usize);
    aligned
}

/// returns memory chunk aligned to word size
#[inline]
pub fn alloc(size: usize) -> *mut u8 {
    if size > LARGEST_BUCKET {
        return alloc_chunked(size);
    }
    let bin_idx = bin_idx(size);
    println!("bin: {}", bin_idx);
    alloc_free_entry(bin_idx)
}

#[inline]
fn bin_idx(size: usize) -> usize {
    // FIXME: support bins in between larger powers of two that are themselves non-power of two bins.
    let rounded_up = size.next_power_of_two();
    (rounded_up.trailing_zeros() - size_of::<usize>().trailing_zeros()) as usize
}

fn alloc_free_entry(bin_idx: usize) -> *mut u8 {
    let bucket = unsafe { &mut (&mut *LOCAL_CACHE.get()).buckets[bin_idx] };
    if let Some(entry) = bucket.try_alloc_free_entry() {
        println!("quickly got entry");
        return entry.into_raw().cast::<u8>().as_ptr();
    }
    // FIXME: reuse cached pages!

    println!("allocating bucket");
    let alloc_0 = alloc_bucket(bin_idx);
    println!("alloced new bucket {}", unsafe { *alloc_0.cast::<usize>() });
    if alloc_0.is_null() {
        return alloc_0;
    }
    assert!(unsafe { AllocRef::new_start(NonNull::new_unchecked(alloc_0)).is_bucket() });
    let alloc = unsafe { AllocRef::new_start(NonNull::new_unchecked(alloc_0)).read_bucket().into_start() };
    let node = BitMapListNode::create(alloc.cast::<()>(), BUCKET_ELEM_SIZES[bin_idx], BUCKET_MAP_CNT[bin_idx], META_ENTRIES_USED[bin_idx]);
    bucket.insert_node(node);
    println!("inserted node {}", node.as_ptr() as usize);
    let ret = bucket.try_alloc_free_entry().map(|entry| entry.into_raw().as_ptr().cast::<u8>()).unwrap_or(null_mut());
    assert!(unsafe { AllocRef::new_start(NonNull::new_unchecked(alloc_0)).is_bucket() });
    println!("ret: {:?}", ret);
    ret
}

fn alloc_chunked(size: usize) -> *mut u8 {
    let size = round_up_to_multiple_of(size, align_of::<usize>());

    let full_size = round_up_to_multiple_of(size + CHUNK_ALLOC_METADATA_SIZE, PAGE_SIZE);

    println!("pre alloc {}", full_size);
    let alloc_ptr = map_memory(full_size);
    println!("alloced {:?}", alloc_ptr);
    if alloc_ptr.is_null() {
        return alloc_ptr;
    }
    let mut alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(alloc_ptr) });
    alloc.read_chunked().setup(ChunkMeta::empty().set_size(full_size).set_first(true).set_free(false));
    let chunk_start = unsafe { alloc.read_chunked().into_start() };
    let mut chunk = ChunkRef::new_start(chunk_start);
    let mut chunk_meta = ChunkMeta::empty().set_size(size + CHUNK_METADATA_SIZE).set_first(true).set_free(false);
    if full_size > size + CHUNK_ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE {
        let mut last_chunk = ChunkRef::new_start(chunk.into_end(size + CHUNK_METADATA_SIZE).into_raw());
        last_chunk.setup(ChunkMeta::empty().set_size(full_size - (size + CHUNK_ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE_ONE_SIDE)).set_first(false).set_last(true).set_free(true));
    } else {
        chunk_meta = chunk_meta.set_last(true);
    }
    chunk.setup(chunk_meta);
    chunk.into_content_start()
}

#[inline]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    let size = round_up_to_multiple_of(size, align_of::<usize>());
    if align <= BUCKET_ELEM_SIZES[BUCKETS - 1] && size <= BUCKET_ELEM_SIZES[BUCKETS - 1] {
        let bucket = bin_idx(size.max(align));
        return alloc_free_entry(bucket);
    }
    alloc_large_alignment(size, align)
}

fn alloc_large_alignment(align: usize, size: usize) -> *mut u8 {
    let os_page_size = get_page_size();
    let base_size = size.max(align);
    // allocate the space we need plus enough space to ensure we can align our allocation to PAGE_SIZE and additionally have 
    let full_size = round_up_to_multiple_of(base_size, PAGE_SIZE) + 2 * PAGE_SIZE + 2 * os_page_size;
    // keep multiples of PAGE_SIZE around so we can reuse these pages later on
    let alloc_size = round_up_to_multiple_of(size, PAGE_SIZE);
    let alloc_ptr = map_memory(full_size);
    // handle mapping errors
    if alloc_ptr.is_null() {
        return null_mut();
    }
    let aligned = unsafe { align_unaligned_ptr_up_to(alloc_ptr, full_size, PAGE_SIZE, alloc_size) };
    let front = (aligned as usize - os_page_size) - alloc_ptr as usize;
    let trailing = full_size - (alloc_size + front);
    if front != 0 {
        // FIXME: currently we are unmapping 1 page too much!
        unsafe { unmap_memory(alloc_ptr, front); }
    }
    if trailing != 0 {
        unsafe { unmap_memory(aligned.add(alloc_size), trailing); }
    }
    let mut alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(aligned.sub(CHUNK_ALLOC_METADATA_SIZE_ONE_SIDE)) });
     // FIXME: if we want to retain parts of the allocation we might have to set last to false
    alloc.read_chunked().setup(ChunkMeta::empty().set_size(full_size).set_free(false).set_first(true).set_last(true));
    let mut chunk = ChunkRef::new_start(aligned);
    chunk.setup(ChunkMeta::new(size, false, false, false));
    let mut first_chunk = ChunkRef::new_start(unsafe { alloc_ptr.add(CHUNK_ALLOC_METADATA_SIZE_ONE_SIDE) });
    let first_chunk_size = first_chunk.read_size();
    first_chunk.setup(ChunkMeta::new(first_chunk_size, true, false, true));
    let last_chunk_start = chunk.into_end(size).into_raw();
    let mut last_chunk = ChunkRef::new_start(last_chunk_start);
    let last_chunk_size = last_chunk.read_size();
    last_chunk.setup(ChunkMeta::new(last_chunk_size, false, true, true));

    chunk.into_content_start()
}

#[inline]
fn alloc_bucket(idx: usize) -> *mut u8 {
    // allocate a new page aligned to PAGE_SIZE
    let alloc = map_pages(1);
    if alloc.is_null() {
        return alloc;
    }
    println!("raw: {} mapped {} idx {}", alloc as usize, alloc as usize % PAGE_SIZE, idx);
    AllocRef::new_start(unsafe { NonNull::new_unchecked(alloc) }).setup_bucket(idx);
    alloc
}

#[inline]
pub fn dealloc(ptr: *mut u8) {
    println!("start dealloc {:?}", ptr);
    // check if we have a large or overaligned allocation
    if ptr as usize % PAGE_SIZE == 0 {
        dealloc_large(ptr);
        return;
    }
    println!("val: {}, {}", unsafe { *(((ptr as usize) - ((ptr as usize) % PAGE_SIZE)) as *mut u8) }, ((ptr as usize) - ((ptr as usize) % PAGE_SIZE)) % PAGE_SIZE);
    let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(((ptr as usize) - ((ptr as usize) % PAGE_SIZE)) as *mut u8) });
    let bucket = alloc.read_bucket();
    println!("bucket: {}", bucket.read_bucket_idx());
    let offset = ptr as usize % PAGE_SIZE; // FIXME: this offset is to small as it doesn't respect the map metadata
    let idx = (offset - BUCKET_META_AND_ELEM_CNTS.0[bucket.read_bucket_idx()]) / BUCKET_ELEM_SIZES[bucket.read_bucket_idx()];
    let bucket_index = idx / PAGE_SIZE;
    let bucket_inner_idx = idx % PAGE_SIZE;
    println!("offset: {} idx: {} bucket_idx: {} bucket_inner_idx: {}", offset, idx, bucket_index, bucket_inner_idx);
    unsafe { bucket.into_start().cast::<AtomicUsize>().add(bucket_index).as_ref().unwrap_unchecked().fetch_or(1 << bucket_inner_idx, Ordering::Release); }
    let elems = bucket.increase_remaining_elem_cnt();
    if elems.is_free() && elems.elem_cnt() == BUCKET_META_AND_ELEM_CNTS.1[bucket.read_bucket_idx()] {
        free_local(alloc.into_raw(), PAGE_SIZE);
    }
}

#[inline]
fn dealloc_large(ptr: *mut u8) {
    let ptr = unsafe { ptr.sub(CHUNK_ALLOC_METADATA_SIZE_ONE_SIDE) };
    dealloc_chunked(ptr);
}

fn dealloc_chunked(ptr: *mut u8) {   
    // FIXME: should we support multiple chunks in the same allocation?
    let mut chunk = ChunkRef::new_start(unsafe { ptr.sub(CHUNK_METADATA_SIZE_ONE_SIDE) });
    let chunk_size = chunk.read_size();
    println!("chunk size: {} chunk {:?} free {}", chunk_size, unsafe { ptr.sub(CHUNK_METADATA_SIZE_ONE_SIDE) }, chunk.is_last());

    if chunk.is_first() {
        println!("first chunk!");
        if chunk.is_last() {
            // we are the only chunk, so just cleanup
            println!("unmap {:?}", chunk.into_raw());
            free_local(unsafe { NonNull::new_unchecked(chunk.into_raw()) }, chunk.read_size());
            return;
        }
        let mut right_chunk = ChunkRef::new_start(chunk.into_end(chunk_size).into_raw());
        if !right_chunk.is_free() {
            println!("only free!");
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        let overall_size = chunk_size + right_chunk.read_size();
        if right_chunk.is_last() {
            println!("unmap {:?}", ptr);
            free_local(unsafe { NonNull::new_unchecked(chunk.into_raw()) }, overall_size.next_multiple_of(PAGE_SIZE));
            return;
        }
        chunk.update_size(overall_size);
        return;
    }
    if chunk.is_last() {
        println!("last chunk!");
        let mut left_chunk = ChunkRef::new_end(unsafe { ptr.sub(CHUNK_METADATA_SIZE_ONE_SIDE) });
        if !left_chunk.is_free() {
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        // FIXME: add threshold until which no chunk is created with empty memory (as it's just unnecessary bookkeeping)
        // FIXME: can we get rid of the first_chunk and last_chunk indicators in the same step?
        let overall_size = left_chunk.read_size() + chunk_size;
        left_chunk.set_last(true);
        left_chunk.update_size(overall_size);
        return;
    }
    // FIXME: support middle chunks!
    todo!()
}

#[inline]
pub fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, _new_align: usize) -> *mut u8 {
    remap_memory(ptr, old_size, new_size)
}

/*#[cfg(not(miri))]
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
            in("r10") MAP_ANON | MAP_PRIVATE,
            in("r8") -1 as c_int,
            in("r9") 0 as off64_t,
            // inlateout("rax") MMAP_SYSCALL_ID => ptr,
            in("rax") MMAP_SYSCALL_ID,
            lateout("rax") ptr,
            lateout("rdx") _,
        );
    }
    ptr
}*/

// #[cfg(miri)]
fn map_memory(size: usize) -> *mut u8 {
    use libc::mmap;

    unsafe { libc::mmap(null_mut(), size as size_t, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1 as c_int, 0 as off64_t) }.cast::<u8>() // FIXME: can we handle return value?
}

#[cfg(miri)]
fn unmap_memory(ptr: *mut u8, size: usize) {
    use libc::munmap;

    use crate::util::abort;

    let result = unsafe { munmap(ptr.cast::<c_void>(), size as size_t) };
    if result != 0 {
        // we can't handle this error properly, so just abort the process
        abort();
    }
}

#[cfg(not(miri))]
#[inline]
fn unmap_memory(ptr: *mut u8, size: usize) {
    const MUNMAP_SYSCALL_ID: usize = 11;

    let result: c_int;

    unsafe {
        asm!(
        "syscall",
        in("rax") MUNMAP_SYSCALL_ID,
        in("rdi") ptr,
        in("rsi") size as size_t,
        lateout("rax") result,
        );
    }

    if result != 0 {
        // we can't handle this error properly, so just abort the process
        abort();
    }
}

#[cfg(miri)]
fn remap_memory(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    use libc::{mremap, MREMAP_MAYMOVE};

    unsafe { mremap(ptr.cast::<c_void>(), old_size, new_size, MREMAP_MAYMOVE) }.cast::<u8>() // FIXME: can we handle return value?
}

#[cfg(not(miri))]
fn remap_memory(ptr: *mut u8, old_size: usize, new_size: usize) -> *mut u8 {
    const MREMAP_SYSCALL_ID: usize = 25;
    let res_ptr;

    // FIXME: set MAY_MOVE flag!

    unsafe {
        asm!(
        "syscall",
        inlateout("rax") MREMAP_SYSCALL_ID => res_ptr,
        in("rdi") ptr,
        in("rsi") old_size,
        in("rdx") new_size,
        );
    }

    res_ptr
}

mod bit_map_list {
    use std::mem::size_of;
    use std::ptr::{NonNull, null_mut};

    use crate::sys::linux::META_ENTRIES_USED;
    use crate::sys::{CACHE_LINE_SIZE, CACHE_LINE_WORD_SIZE, get_page_size};
    use crate::util::min;

    use super::{register_dtor, PAGE_SIZE};

    const REGISTERED_FLAG: usize = 1 << (usize::BITS - 1);

    /// # Design
    /// This is basically just a linked list of bitmaps that has links to the first and last node.
    /// New nodes will be appended to the end in order to allow the root node to be emptied of entries and as such
    /// reduce fragmentation of the system and allow for more frequent deallocation of nodes at the tail (if they are empty).
    #[derive(Default)]
    pub(crate) struct BitMapList {
        head: Option<NonNull<BitMapListNode>>,
        tail: Option<NonNull<BitMapListNode>>,
        entries: usize,
    }

    impl BitMapList {

        #[inline]
        pub const fn new_empty() -> Self {
            Self {
                head: None,
                // setup sentinel such that we can catch calls to malloc from the register_dtor impl.
                // tail: Some(unsafe { NonNull::new_unchecked((&NOT_SETUP_SENTINEL as *const u8).cast_mut().cast::<BitMapListNode>()) }),
                tail: None,
                entries: 0,
            }
        }

        pub fn insert_node(&mut self, node: NonNull<BitMapListNode>) {
            self.entries += 1;
            if self.head.is_none() {
                self.head = Some(node);
                self.tail = Some(node);
                if self.entries & REGISTERED_FLAG == 0 { // FIXME: should we use a sentinel value inside tail instead for optimization purposes?
                    self.entries |= REGISTERED_FLAG;
                    // unsafe { register_dtor(t, dtor); } // FIXME: support this!
                }
                return;
            }
            let mut old_tail = unsafe { self.tail.unwrap_unchecked() };
            self.tail = Some(node);
            unsafe { old_tail.as_mut() }.next = node.as_ptr();
        }

        pub fn try_alloc_free_entry(&mut self) -> Option<BitMapEntry> {
            if self.head.is_none() {
                return None;
            }
            println!("checked head!");
            let mut ret = unsafe { self.head.unwrap_unchecked().as_mut() }.alloc_free_entry();
            println!("ret: {:?}", ret.map(|entry| entry.ptr.as_ptr()));
            if ret.is_none() {
                // get rid of old entry
                self.entries -= 1;
                self.head = NonNull::new(unsafe { self.head.unwrap_unchecked().as_ref().next });
                ret = unsafe { self.head.unwrap_unchecked().as_mut() }.alloc_free_entry();
                println!("replaced!");
            }
            println!("ret: {:?}", ret.map(|entry| entry.ptr.as_ptr()));          
            ret
        }

        #[inline]
        pub fn entries(&self) -> usize {
            self.entries & !REGISTERED_FLAG
        }

    }

    const fn calc_sub_maps() -> usize {
        let mut sub_maps = 0;
        let mut bits = CACHE_LINE_SIZE * 8;
        while bits > 0 {
            let sub_map_size = usize::BITS as usize;
            bits = bits.saturating_sub(SUB_MAP_SLOTS + sub_map_size);
            sub_maps += 1;
        }
        sub_maps
    }

    const NODE_SLOTS: usize = CACHE_LINE_WORD_SIZE - 1; // the - 1 here comes from the fact that we have 1 parent word per node
    const SUB_MAPS: usize = calc_sub_maps();
    const SUB_MAPS_SLOTS_COMBINED: usize = (NODE_SLOTS - SUB_MAPS) * 8;
    const SUB_MAP_SLOTS: usize = min(usize::BITS as usize, CACHE_LINE_SIZE);

    #[derive(Clone, Copy)]
    pub(crate) struct BitMapEntry {
        ptr: NonNull<()>,
    }

    impl BitMapEntry {

        #[inline]
        pub fn into_raw(self) -> NonNull<()> {
            self.ptr
        }

    }

    pub(crate) struct BitMapListNode {
        next: *mut BitMapListNode,
        // FIXME: use one additional word as metadata (as we have to pad anyways)
    }

    impl BitMapListNode {

        pub(crate) fn create(addr: *mut (), element_size: usize, sub_maps: usize, entries_used: usize) -> NonNull<Self> {
            println!("addr {} size: {}", addr as usize, element_size);
            // FIXME: cache all these values for all used bucket sizes on startup
            unsafe { addr.cast::<usize>().write(element_size); }
            unsafe { addr.cast::<usize>().add(1).cast::<BitMapListNode>().write(BitMapListNode {
                next: null_mut(),
            }); }
            println!("used entries: {}", entries_used);
            // mark all the filled nodes as used
            let full_sub_map_bytes = entries_used.div_floor(u8::BITS as usize);
            println!("map bytes: {}", full_sub_map_bytes);
            unsafe { core::ptr::write_bytes(addr.cast::<u8>().add(size_of::<usize>() + size_of::<BitMapListNode>()), u8::MAX, full_sub_map_bytes); }
            // deal with the last partially filled (or empty, doesn't matter) bitset and fill it as required
            let remaining = entries_used - full_sub_map_bytes * (u8::BITS as usize);
            let bit_set = (1 << (remaining + 1)) - 1;
            unsafe { addr.cast::<u8>().add(size_of::<usize>() + size_of::<BitMapListNode>() + full_sub_map_bytes).write(bit_set); }
            // clear the remaining bitmaps to make sure that we indicate that they are usable
            let remaining_bytes = (sub_maps * size_of::<usize>()) - full_sub_map_bytes - 1;
            unsafe { core::ptr::write_bytes(addr.cast::<u8>().add(size_of::<usize>() + size_of::<BitMapListNode>() + full_sub_map_bytes + 1), 0, remaining_bytes); }
            unsafe { NonNull::new_unchecked(addr.cast::<usize>().add(1).cast::<BitMapListNode>()) }
        }

        pub(crate) fn alloc_free_entry(&mut self) -> Option<BitMapEntry> {
            // assume that we are inside the allocation page, such a page looks something like this:
            // [entries, bitmaps, leaf tip, metadata]
            let curr = self as *mut BitMapListNode as *mut ();
            println!("checking {}", curr as usize);
            // FIXME: can we get rid of this div_ceil - we could by caching!
            let bitmap_cnt = (PAGE_SIZE / self.element_size()).div_ceil(usize::BITS as usize).max(1);
            // traverse the map backwards to allow for the possibility that we don't have to fetch
            // another cache line if we are lucky
            println!("pre checking maps!");
            let end = unsafe { curr.cast::<usize>().add(bitmap_cnt) };
            for i in 0..bitmap_cnt {
                println!("check map!");
                let map_ptr = unsafe { end.sub(i) };
                let map = unsafe { *map_ptr };
                let idx = map.trailing_zeros() as usize;
                if idx != usize::BITS as usize {
                    return Some(BitMapEntry {
                        // FIXME: can we get rid of this get_page_size?
                        // ptr: unsafe { NonNull::new_unchecked((curr.cast::<usize>() as usize / get_page_size() + (self.element_cnt - i * 8 - (usize::BITS - idx))) as *mut ()) },
                        ptr: unsafe { NonNull::new_unchecked((curr.cast::<usize>().sub(i * size_of::<usize>() + idx)) as *mut ()) },
                    });
                }
            }
            None
        }

        #[inline]
        fn element_size(&self) -> usize {
            let curr = self as *const BitMapListNode as *mut ();
            unsafe { *curr.cast::<usize>().sub(1) }
        }

    }

}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mov {}, fs", out(reg) res, options(nostack, nomem, preserves_flags)); }
    res
}

#[cfg(all(target_arch = "x86", target_os = "linux"))]
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mov {}, gs", out(reg) res, options(nostack, nomem, preserves_flags)); }
    res
}

#[cfg(all(target_arch = "aarch64"))] // FIXME: does this actually work on linux and is it specific to linux?
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mrs {}, TPIDR_EL0", out(reg) res); } // TODO: add some assembly options
    res
}

/* // FIXME: try using a compatible way to accessing this
#[cfg(all(target_arch = "arm"))] // FIXME: does this actually work on linux and is it specific to linux?
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mrc p15, 0, {}, c13, c0, 2", out(reg) res); } // TODO: add some assembly options
    res
}*/
