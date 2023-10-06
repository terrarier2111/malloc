use core::arch::asm;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::mem::{align_of, size_of};
use std::ptr::{null_mut, NonNull};
use libc::{_SC_PAGESIZE, c_int, MAP_ANON, MAP_PRIVATE, off64_t, PROT_READ, PROT_WRITE, size_t, sysconf};
use crate::alloc_ref::{AllocRef, ALLOC_FULL_INITIAL_METADATA_SIZE, ALLOC_METADATA_SIZE_ONE_SIDE, ALLOC_METADATA_SIZE, CHUNK_ALLOC_METADATA_SIZE_ONE_SIDE, BUCKET_METADATA_SIZE};
use crate::chunk_ref::meta::ChunkMeta;
use crate::chunk_ref::{CHUNK_METADATA_SIZE, ChunkRef, CHUNK_METADATA_SIZE_ONE_SIDE};
use crate::util::{align_unaligned_ptr_up_to, round_up_to_multiple_of, abort};

use self::bit_map_list::BitMapListNode;
use self::global_free_list::GlobalFreeList;
use self::local_cache::ThreadLocalCache;

use super::{CACHE_LINE_SIZE, CACHE_LINE_WORD_SIZE};

// FIXME: reuse allocations and maybe use sbrk

// FIXME: try avoiding atomic instructions if the freeing thread is the same as the original thread.
// FIXME: we could do this by having 2 bitsets (1 for non-atomic local frees and 1 for atomic non-local frees)

const NOT_PRESENT: usize = 0;

static GLOBAL_FREE_LIST: GlobalFreeList = GlobalFreeList::new(); // this is the root for the implicit global RB-tree

mod global_free_list {
    use std::{ptr::{null_mut, NonNull}, sync::atomic::{AtomicPtr, Ordering}};

    use crate::{alloc_ref::AllocRef};


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
            let alloc_ref = unsafe { AllocRef::new_start(unsafe { NonNull::new_unchecked(alloc_start.cast::<u8>()) }) };
            let next_ptr = unsafe { alloc_ref.into_start().cast::<usize>() };
            let mut curr_start_alloc = self.root.load(Ordering::Acquire);
            
        }

    }

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
static LOCAL_CACHE: ThreadLocalCache = ThreadLocalCache::new();

fn cleanup_tls() {
    // we store the metadata about the size and next ptr in the allocations themselves
    // this means that we will trash the cache more and we have to fetch more things into
    // it but we don't have to use as much memory. We have to load 24 bytes (on 64 bit systems)
    // anyways as we have 2 ptrs to children and one key for traversal.
    // FIXME: is this tradeoff worth it?
    let mut chunk_ref = LOCAL_CACHE.free_chunk_root.root_ref();
    while let Some(chunk) = chunk_ref {
        let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(chunk.raw_ptr().cast::<u8>()) });
        let size = alloc.read_chunked().read_size() / PAGE_SIZE; // FIXME: should the size value already be stored in multiples of page size inside the alloc?

    }
}

mod local_cache {

    // static NOT_SETUP_SENTINEL: u8 = 0;

    use std::mem::{MaybeUninit, transmute};

    use crate::implicit_rb_tree::ImplicitRbTree;

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

    pub(crate) struct ThreadLocalCache {
        pub(crate) buckets: [BitMapList; BUCKETS],
        pub(crate) free_chunk_root: ImplicitRbTree, // this acts as a local cache for free chunks
    }

    unsafe impl Send for ThreadLocalCache {}
    unsafe impl Sync for ThreadLocalCache {}

    impl ThreadLocalCache {

        pub const fn new() -> Self {
            Self {
                buckets: construct_buckets(),
                free_chunk_root: ImplicitRbTree::new(),
            }
        }

        pub fn push_free_chunk(&mut self, chunk: *mut ()) {
            
        }

    }
}

const BUCKETS: usize = 10;
const BUCKET_ELEM_SIZES: [usize; BUCKETS] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096];
const LARGEST_BUCKET: usize = BUCKET_ELEM_SIZES[BUCKETS - 1];
// the first array is for meta size and the second is for elem cnt
const BUCKET_MAP_AND_ELEM_CNTS: ([usize; BUCKETS], [usize; BUCKETS]) = {
    let mut ret = ([0; BUCKETS], [0; BUCKETS]);
    let mut i = 0;
    while i < BUCKETS {
        // calculate bucket size by first estimating the meta size and then
        // iteratively improving our estimate until we have the perfect values.
        let mut max_elems = PAGE_SIZE / BUCKET_ELEM_SIZES[i];
        // our metadata consists of a bitmap of used values and another atomic bitmap, which has to be aligned to cache line size.
        const BUCKET_META_WORD_SIZE: usize = BUCKET_METADATA_SIZE.div_ceil(size_of::<usize>());
        let mut meta_size = (max_elems.div_ceil(usize::BITS as usize) * 2 + BUCKET_META_WORD_SIZE).next_multiple_of(CACHE_LINE_WORD_SIZE);
        loop {
            let prev = max_elems;
            max_elems = (PAGE_SIZE - meta_size * size_of::<usize>()) / BUCKET_ELEM_SIZES[i];
            // check if we can't improve our estimate, in which case we must already have the perfect values
            if prev == max_elems {
                break;
            }
            meta_size = (max_elems.div_ceil(usize::BITS as usize) * 2 + BUCKET_META_WORD_SIZE).next_multiple_of(CACHE_LINE_WORD_SIZE);
        }
        ret.0[i] = meta_size - BUCKET_META_WORD_SIZE;
        ret.1[i] = max_elems;
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

/// returns memory aligned to word size
#[inline]
pub fn alloc(size: usize) -> *mut u8 {
    if size > LARGEST_BUCKET {
        return alloc_chunked(size);
    }
    let bin_idx = bin_idx(size);
    let bucket = &LOCAL_CACHE.buckets[bin_idx];
    if let Some(entry) = bucket.alloc_free_entry() {
        return entry.into_raw().cast::<u8>().as_ptr();
    }
    // FIXME: reuse cached pages!
    let alloc = alloc_bucket(bin_idx);
    if alloc.is_null() {
        return alloc;
    }
    let alloc = unsafe { AllocRef::new_start(NonNull::new_unchecked(alloc)).into_start() };
    let node = BitMapListNode::create(alloc.cast::<()>(), BUCKET_ELEM_SIZES[bin_idx]);
    bucket.insert_node(node);
    bucket.alloc_free_entry().map(|entry| entry.into_raw().as_ptr().cast::<u8>()).unwrap_or(null_mut())
}

#[inline]
fn bin_idx(size: usize) -> usize {
    // FIXME: support bins in between larger powers of two that are themselves non-power of two bins.
    let rounded_up = size.next_power_of_two();
    (rounded_up.leading_zeros() - size_of::<usize>().leading_zeros()) as usize
}

fn alloc_chunked(size: usize) -> *mut u8 {
    let size = round_up_to_multiple_of(size, align_of::<usize>());

    let full_size = round_up_to_multiple_of(size + ALLOC_FULL_INITIAL_METADATA_SIZE, PAGE_SIZE);

    println!("pre alloc {}", full_size);
    let alloc_ptr = map_memory(full_size);
    println!("alloced {:?}", alloc_ptr);
    if alloc_ptr.is_null() {
        return alloc_ptr;
    }
    let mut alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(alloc_ptr) });
    alloc.read_chunked().setup(ChunkMeta::empty().set_size(full_size).set_first(true).set_free(false));
    let chunk_start = unsafe { alloc.into_start() };
    let mut chunk = ChunkRef::new_start(chunk_start);
    let mut chunk_meta = ChunkMeta::empty().set_size(size + CHUNK_METADATA_SIZE).set_first(true).set_free(false);
    if full_size > size + ALLOC_FULL_INITIAL_METADATA_SIZE + CHUNK_METADATA_SIZE {
        let mut last_chunk = ChunkRef::new_start(chunk.into_end(size + CHUNK_METADATA_SIZE).into_raw());
        last_chunk.setup(ChunkMeta::empty().set_size(full_size - (size + ALLOC_FULL_INITIAL_METADATA_SIZE)).set_first(false).set_last(true).set_free(true));
    } else {
        chunk_meta = chunk_meta.set_last(true);
    }
    chunk.setup(chunk_meta);
    chunk.into_content_start()
}

#[inline]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    let size = round_up_to_multiple_of(size, align_of::<usize>());
    let full_size = round_up_to_multiple_of(size * 2 + ALLOC_FULL_INITIAL_METADATA_SIZE, PAGE_SIZE);
    let alloc_ptr = map_memory(full_size);
    if alloc_ptr.is_null() {
        return alloc_ptr;
    }
    let mut alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(alloc_ptr) });
    alloc.read_chunked().setup(full_size);
    let mut desired_chunk_start = unsafe { align_unaligned_ptr_up_to(alloc_ptr, full_size - ALLOC_METADATA_SIZE_ONE_SIDE, align, ALLOC_METADATA_SIZE_ONE_SIDE).sub(ALLOC_METADATA_SIZE_ONE_SIDE) };
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
fn alloc_bucket(idx: usize) -> *mut u8 {
    let alloc = map_memory(get_page_in_os_page_cnt()); // FIXME: align the allocated pages to PAGE_SIZE! (for this we will have to allocate 2 * PAGE_SIZE) pages and dealloc the trailing pages and unaligned pages at the beginning!
    if alloc.is_null() {
        return alloc;
    }
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
    let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(ptr) });
    if alloc.is_bucket() {
        let alloc = alloc.read_bucket();
        
    } else {
        dealloc_chunked(ptr);
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
            unmap_memory(chunk.into_raw(), chunk.read_size());
            return;
        }
        let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(chunk.into_raw().sub(ALLOC_METADATA_SIZE_ONE_SIDE)) });
        let alloc_size = alloc.read_chunked().read_size();
        let mut right_chunk = ChunkRef::new_start(chunk.into_end(chunk_size).into_raw());
        if !right_chunk.is_free() {
            println!("only free!");
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        let overall_size = chunk_size + right_chunk.read_size();
        if right_chunk.is_last() {
            println!("unmap {:?}", alloc.into_raw());
            unmap_memory(alloc.into_raw().as_ptr(), alloc_size);
            return;
        }
        right_chunk.set_first(true); // FIXME: just update right metadata
        right_chunk.update_size(overall_size); // FIXME: just update right metadata
        chunk.update_size(overall_size); // FIXME: just update left metadata
        return;
    }
    if chunk.is_last() {
        println!("last chunk!");
        // FIXME: try merging with left chunk
        let alloc = AllocRef::new_start(unsafe { NonNull::new_unchecked(chunk.into_raw().sub(ALLOC_METADATA_SIZE_ONE_SIDE + CHUNK_METADATA_SIZE_ONE_SIDE)) });
        let alloc_size = alloc.read_chunked().read_size();
        let mut left_chunk = ChunkRef::new_end(unsafe { alloc.into_raw().as_ptr().sub(ALLOC_METADATA_SIZE_ONE_SIDE) });
        if !left_chunk.is_free() {
            // we can't merge with any other chunk, so we can just mark ourselves as free
            chunk.set_free(true);
            return;
        }
        let overall_size = chunk_size + left_chunk.read_size();
        if overall_size + ALLOC_METADATA_SIZE == alloc_size {
            unmap_memory(alloc.into_raw().as_ptr(), alloc_size);
            return;
        }
        left_chunk.set_last(true); // FIXME: just update left metadata
        left_chunk.update_size(overall_size); // FIXME: just update left metadata
        chunk.update_size(overall_size); // FIXME: just update right metadata
        return;
    }
    println!("weird chunk!");
}

#[inline]
pub fn realloc(ptr: *mut u8, old_size: usize, new_size: usize, _new_align: usize) -> *mut u8 {
    remap_memory(ptr, old_size, new_size)
}

#[cfg(not(miri))]
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
            inlateout("rax") MMAP_SYSCALL_ID => ptr,
            lateout("rdx") _,
        );
    }
    ptr
}

#[cfg(miri)]
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
    use libc::mremap;

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

    use crate::sys::{CACHE_LINE_SIZE, CACHE_LINE_WORD_SIZE, get_page_size};
    use crate::util::min;

    use super::{register_dtor, PAGE_SIZE};

    const REGISTERED_FLAG: usize = 1 << (usize::BITS - 1);

    /// # Design
    /// This is basically just a linked list of bitmaps that has links to the first and last node.
    /// New nodes will be appended to the end in order to allow the root node to be emptied of entries and as such
    /// reduce fragmentation of the system and allow for more frequent deallocation of nodes at the tail (if the are empty).
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

        pub fn insert_node(&mut self, mut node: NonNull<BitMapListNode>) {
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

        pub fn alloc_free_entry(&mut self) -> Option<BitMapEntry> {
            if self.head.is_none() {
                return None;
            }
            let ret = unsafe { self.head.unwrap_unchecked().as_mut() }.alloc_free_entry();
            if ret.is_none() {
                // get rid of old entry
                self.entries -= 1;
                self.head = NonNull::new(unsafe { self.head.unwrap_unchecked().as_ref().next });
            }
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
        element_cnt: usize,
        next: *mut BitMapListNode,
        // FIXME: use one additional word as metadata (as we have to pad anyways)
    }

    impl BitMapListNode {

        pub(crate) fn create(addr: *mut (), element_size: usize) -> NonNull<Self> {
            // FIXME: cache all these values for all used bucket sizes on startup
            let max_elem_cnt = PAGE_SIZE / element_size;
            let sub_maps = max_elem_cnt.div_ceil(usize::BITS as usize);
            let meta = size_of::<usize>() + size_of::<BitMapListNode>() + sub_maps;
            let elem_cnt = max_elem_cnt - meta.div_ceil(element_size);
            unsafe { addr.cast::<usize>().write(element_size); }
            unsafe { addr.cast::<usize>().add(1).cast::<BitMapListNode>().write(BitMapListNode {
                element_cnt: elem_cnt,
                next: null_mut(),
            }); }
            for i in 0..sub_maps {
                unsafe { addr.cast::<u8>().add(size_of::<usize>() + size_of::<BitMapListNode>() + size_of::<usize>() * i).write(0); }
            }
            unsafe { NonNull::new_unchecked(addr.cast::<usize>().add(1).cast::<BitMapListNode>()) }
        }

        pub(crate) fn alloc_free_entry(&mut self) -> Option<BitMapEntry> {
            // assume that we are inside the allocation page, such a page looks something like this:
            // [entries, bitmaps, leaf tip, metadata]
            let curr = self as *mut BitMapListNode as *mut ();
            // FIXME: can we get rid of this div_ceil - we could by caching!
            let bitmap_cnt = self.element_cnt.div_ceil(usize::BITS as usize);
            // traverse the map backwards to allow for the possibility that we don't have to fetch
            // another cache line if we are lucky
            let end = unsafe { curr.cast::<usize>().sub(1) };
            for i in 0..bitmap_cnt {
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

    }

}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mov {}, fs", out(reg) res, options(nostack, nomem, pure, preserves_flags)); }
    res
}

#[cfg(all(target_arch = "x86", target_os = "linux"))]
fn thread_identifier() -> usize {
    let res;
    unsafe { asm!("mov {}, gs", out(reg) res, options(nostack, nomem, pure, preserves_flags)); }
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
