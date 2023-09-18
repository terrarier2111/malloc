use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::mem::align_of;
use cache_padded::CachePadded;
use libc::{_SC_PAGESIZE, c_int, MAP_ANON, MAP_PRIVATE, MREMAP_MAYMOVE, off64_t, PROT_READ, PROT_WRITE, size_t, sysconf};
use crate::linux::alloc_ref::AllocRef;
use crate::linux::chunk_ref::ChunkRef;
use crate::util::{align_unaligned_ptr_to, round_up_to};

// FIXME: reuse allocations and maybe use sbrk

const NOT_PRESENT: usize = 0;

static PAGE_SIZE: AtomicUsize = AtomicUsize::new(NOT_PRESENT);

static FREE_CHUNK_ROOT: AtomicPtr<()> = AtomicPtr::new(null_mut()); // this is the root for the implicit RB-tree

/// `chunk_start` indicates the start of the chunk (including metadata)
/// `size` indicates the chunk size in bytes
fn push_free_chunk(chunk_start: *mut u8, size: usize) {
    let curr_start_chunk = FREE_CHUNK_ROOT.load(Ordering::Acquire).cast::<u8>();
    if curr_start_chunk.is_null() {
        FREE_CHUNK_ROOT.store(chunk_start.cast::<_>(), Ordering::Release);
        return;
    }

}

#[inline]
fn get_page_size() -> usize {
    let cached = PAGE_SIZE.load(Ordering::Relaxed);
    if cached != NOT_PRESENT {
        return cached;
    }
    setup_page_size()
}

#[cold]
#[inline(never)]
fn setup_page_size() -> usize {
    let resolved = unsafe { sysconf(_SC_PAGESIZE) as usize };
    PAGE_SIZE.store(resolved, Ordering::Relaxed);
    resolved
}

/// returns memory aligned to ptr size
#[inline]
pub fn alloc(size: usize) -> *mut u8 {
    let size = round_up_to(size, align_of::<usize>());
    let page_size = get_page_size();

    let full_size = round_up_to(size + ALLOC_FULL_INITIAL_METADATA_SIZE, page_size);

    println!("pre alloc {}", full_size);
    let alloc_ptr = map_memory(full_size);
    println!("alloced {:?}", alloc_ptr);
    if alloc_ptr.is_null() {
        return alloc_ptr;
    }
    let mut alloc = AllocRef::new_start(alloc_ptr);
    alloc.setup(full_size, size + CHUNK_METADATA_SIZE);
    let chunk_start = unsafe { alloc_chunk_start(alloc_ptr) };
    let mut chunk = ChunkRef::new_start(chunk_start);
    chunk.setup(size + CHUNK_METADATA_SIZE, true, false);
    if full_size > size + ALLOC_FULL_INITIAL_METADATA_SIZE + ALLOC_FULL_INITIAL_METADATA_PADDING + CHUNK_METADATA_SIZE {
        let mut last_chunk = ChunkRef::new_start(chunk.into_end(size + CHUNK_METADATA_SIZE).into_raw());
        last_chunk.setup(full_size - (size + ALLOC_FULL_INITIAL_METADATA_SIZE), false, true);
        last_chunk.set_free(true);
    } else {
        chunk.set_last(true);
    }
    chunk.into_content_start()
}

#[inline]
pub fn alloc_aligned(size: usize, align: usize) -> *mut u8 {
    let size = round_up_to(size, align_of::<usize>());
    let page_size = get_page_size();
    let full_size = round_up_to(size * 2 + ALLOC_FULL_INITIAL_METADATA_SIZE, page_size);
    let alloc_ptr = map_memory(full_size);
    if alloc_ptr.is_null() {
        return alloc_ptr;
    }
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
pub fn dealloc(ptr: *mut u8) {
    println!("start dealloc {:?}", ptr);
    let mut chunk = ChunkRef::new_start(unsafe { ptr.sub(CHUNK_METADATA_SIZE_ONE_SIDE) });
    let chunk_size = chunk.read_size();
    println!("chunk size: {} chunk {:?} free {}", chunk_size, unsafe { ptr.sub(CHUNK_METADATA_SIZE_ONE_SIDE) }, chunk.is_last());

    if chunk.is_first() {
        println!("first chunk!");
        if chunk.is_last() {
            unreachable!();
        }
        let alloc = AllocRef::new_start(unsafe { chunk.into_raw().sub(ALLOC_METADATA_SIZE_ONE_SIDE) });
        let alloc_size = alloc.read_size();
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
            unmap_memory(alloc.into_raw(), alloc_size);
            return;
        }
        right_chunk.set_first(true); // FIXME: just update right metadata
        right_chunk.update_size(overall_size); // FIXME: just update right metadata
        chunk.update_size(overall_size); // FIXME: just update left metadata
        return;
    } else if chunk.is_last() {
        println!("last chunk!");
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
        println!("weird chunk!");
    }
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

    let result = unsafe { munmap(ptr.cast::<c_void>(), size as size_t) };
    if result != 0 {
        // we can't handle this error properly, so just abort the process
        core::intrinsics::abort();
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
        core::intrinsics::abort();
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

const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE * 2;
const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>() * 2;
const CHUNK_METADATA_SIZE: usize = CHUNK_METADATA_SIZE_ONE_SIDE * 2;
const CHUNK_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;
const ALLOC_FULL_INITIAL_METADATA_PADDING: usize = {
    let diff = ALLOC_FULL_INITIAL_METADATA_SIZE % MIN_ALIGN;
    if diff == 0 {
        0
    } else {
        MIN_ALIGN - diff
    }
};

#[inline]
unsafe fn alloc_chunk_start(alloc: *mut u8) -> *mut u8 {
    alloc.add(ALLOC_METADATA_SIZE_ONE_SIDE)
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
    use crate::linux::CHUNK_METADATA_SIZE_ONE_SIDE;

    // FIXME: ensure that we have alignment >= 8
    const FIRST_CHUNK_FLAG: usize = 1 << 0;
    const LAST_CHUNK_FLAG: usize = 1 << 1;
    const FREE_CHUNK_FLAG: usize = 1 << 2;
    const METADATA_MASK: usize = FIRST_CHUNK_FLAG | LAST_CHUNK_FLAG | FREE_CHUNK_FLAG;
    const SIZE_MASK: usize = !METADATA_MASK;

    /// `START` indicates whether the stored reference is a reference to the chunk's start or end.
    #[derive(Copy, Clone)]
    pub(crate) struct ChunkRef<const START: bool>(*mut u8);

    impl<const START: bool> ChunkRef<START> {

        #[inline]
        pub(crate) fn into_raw(self) -> *mut u8 {
            self.0
        }

        #[inline]
        fn read_size_raw(&self) -> usize {
            if START {
                unsafe { *self.0.cast::<usize>() }
            } else {
                unsafe { *self.0.cast::<usize>().sub(1) }
            }
        }

        #[inline]
        fn write_size_raw(&mut self, size: usize) {
            if START {
                unsafe { *self.0.cast::<usize>() = size; }
            } else {
                unsafe { *self.0.cast::<usize>().sub(1) = size; }
            }
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
            (self.read_size_raw() & SIZE_MASK)
        }

        /// this doesn't modify the FIRST_CHUNK flag
        #[inline]
        pub(crate) fn update_size(&mut self, size: usize) {
            let first_chunk_flag = self.read_size_raw() & METADATA_MASK;
            self.write_size_raw(size | first_chunk_flag);
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
            self.read_size_raw() & FIRST_CHUNK_FLAG != 0
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
            self.read_size_raw() & LAST_CHUNK_FLAG != 0
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
            self.read_size_raw() & FREE_CHUNK_FLAG != 0
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
            /*if self.into_end(size).0 as usize % 8 != 0 {
                panic!("unaligned: {} size: {}", self.into_end(size).0 as usize % 8, size % 8);
            }
            println!("checked alignment end {:?}", self.0);*/
            self.into_end(size).setup_own(size, first_chunk, last_chunk);
        }

        #[inline]
        pub(crate) fn into_end(self, size: usize) -> ChunkRef<false> {
            ChunkRef(unsafe { self.0.add(size) })
        }

        #[inline]
        pub(crate) fn into_content_start(self) -> *mut u8 {
            unsafe { self.0.add(CHUNK_METADATA_SIZE_ONE_SIDE) }
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
        pub(crate) fn into_start(self, size: usize) -> ChunkRef<true> {
            ChunkRef(unsafe { self.0.sub(size) })
        }

    }

}

mod implicit_rb_tree {
    use std::cmp::Ordering;
    use std::mem::size_of;
    use std::ptr::{NonNull, null_mut};

    const RED: usize = 0 << 0;
    const BLACK: usize = 1 << 0;

    const COLOR_MASK: usize = 1 << 0;

    const PTR_MASK: usize = !COLOR_MASK;

    pub const NODE_SIZE: usize = size_of::<ImplicitRbTreeNode>();

    #[derive(Copy, Clone, PartialEq)]
    #[repr(usize)]
    pub enum Color {
        Red = RED,
        Black = BLACK,
    }

    #[repr(C)]
    pub struct ImplicitRbTree {
        root: Option<ImplicitRbTreeNodeRef>,
    }
    
    impl ImplicitRbTree {
        
        #[inline]
        pub const fn new() -> Self {
            Self {
                root: None,
            }
        }

        pub fn insert(&mut self, key: usize, addr: NonNull<()>) {
            match self.root {
                None => {
                    self.root = Some(unsafe { create_tree_node(key, addr, Color::Black, None) });
                }
                Some(root) => {
                    unsafe { root.tree_node_mut() }.insert(key, addr);
                }
            }
        }

        pub fn remove(&mut self, key: usize) -> Option<NonNull<()>> {
            let root = unsafe { self.root.as_mut().unwrap().tree_node_mut() };
            if !root.has_children() {
                return unsafe { self.root.take().map(|node| node.0) }; // FIXME: should we return the data ptr instead?
            }
            Some(unsafe { root.remove(key) })
        }

        pub fn find_approx_ge(&self, approx_key: usize) -> Option<ImplicitRbTreeNodeRef> {
            match self.root {
                None => None,
                Some(node) => unsafe { node.tree_node() }.find_approx_ge(approx_key),
            }
        }
        
    }

    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct ImplicitRbTreeNodeRef(NonNull<()>);

    impl ImplicitRbTreeNodeRef {

        #[inline]
        pub fn new(ptr: NonNull<()>) -> Self {
            Self(ptr)
        }

        #[inline]
        pub(crate) unsafe fn tree_node<'a>(self) -> &'a ImplicitRbTreeNode {
            unsafe { self.0.cast::<ImplicitRbTreeNode>().as_ref() }
        }

        #[inline]
        pub(crate) unsafe fn tree_node_mut<'a>(self) -> &'a mut ImplicitRbTreeNode {
            unsafe { self.0.cast::<ImplicitRbTreeNode>().as_mut() }
        }

        #[inline]
        pub(crate) unsafe fn data_ptr(self) -> NonNull<u8> {
            unsafe { NonNull::new_unchecked(self.0.cast::<u8>().as_ptr().add(size_of::<ImplicitRbTreeNode>())) }
        }

        #[inline]
        pub(crate) fn raw_ptr(self) -> *mut ImplicitRbTreeNode {
            self.0.as_ptr().cast::<ImplicitRbTreeNode>()
        }

    }

    #[inline]
    unsafe fn create_tree_node(key: usize, addr: NonNull<()>, color: Color, parent: Option<ImplicitRbTreeNodeRef>) -> ImplicitRbTreeNodeRef {
        unsafe { addr.cast::<ImplicitRbTreeNode>().as_ptr().write(ImplicitRbTreeNode {
            parent: parent.map_or(null_mut(), |node| node.0.as_ptr()).map_addr(|addr| addr | color as usize),
            left: None,
            right: None,
            key,
        }); }
        ImplicitRbTreeNodeRef(addr)
    }

    #[repr(align(2))]
    #[repr(C)]
    pub struct ImplicitRbTreeNode {
        parent: *mut (),
        left: Option<ImplicitRbTreeNodeRef>,
        right: Option<ImplicitRbTreeNodeRef>,
        key: usize,
    }

    impl ImplicitRbTreeNode {

        #[inline]
        unsafe fn new_red(parent: ImplicitRbTreeNodeRef, key: usize) -> Self {
            Self {
                parent: parent.0.as_ptr().map_addr(|addr| addr | RED),
                left: None,
                right: None,
                key,
            }
        }

        #[inline]
        unsafe fn new_black(parent: ImplicitRbTreeNodeRef, key: usize) -> Self {
            Self {
                parent: parent.0.as_ptr().map_addr(|addr| addr | BLACK),
                left: None,
                right: None,
                key,
            }
        }

        #[inline]
        fn parent_ptr(&self) -> Option<ImplicitRbTreeNodeRef> {
            let ptr = self.parent.mask(PTR_MASK);
            if ptr.is_null() {
                return None;
            }
            Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(ptr) }))
        }

        #[inline]
        pub(crate) fn is_red(&self) -> bool {
            (self.parent as usize) & COLOR_MASK == RED
        }

        #[inline]
        pub(crate) fn is_black(&self) -> bool {
            (self.parent as usize) & COLOR_MASK == BLACK
        }

        #[inline]
        pub(crate) fn set_color(&mut self, color: Color) {
            self.parent = self.parent.map_addr(|raw| (raw & PTR_MASK) | color as usize);
        }

        #[inline]
        fn sibling(&self) -> Option<ImplicitRbTreeNodeRef> {
            match self.parent_ptr() {
                None => None,
                Some(parent_ptr) => {
                    let right = unsafe { parent_ptr.tree_node().right };
                    if right.map_or(null_mut(), |ptr| ptr.0.as_ptr()).cast_const().cast::<ImplicitRbTreeNode>() == self as *const ImplicitRbTreeNode {
                        let left = unsafe { parent_ptr.tree_node().left };
                        return left;
                    }
                    debug_assert_eq!(unsafe { parent_ptr.tree_node().left }, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) })));
                    right
                }
            }
        }

        #[inline]
        fn child_dir(&self, child: *mut ImplicitRbTreeNode) -> Direction {
            if let Some(right) = self.right {
                if right == ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(child.cast::<()>()) }) {
                    return Direction::Right;
                }
            }
            debug_assert_eq!(self.left, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(child.cast::<()>()) })));
            Direction::Left
        }

        #[inline]
        fn has_children(&self) -> bool {
            self.left.is_some() || self.right.is_some()
        }

        fn insert(&mut self, key: usize, addr: NonNull<()>) {
            if self.key < key {
                if let Some(right) = self.right {
                    unsafe { right.tree_node_mut().insert(key, addr); }
                    return;
                }
                let node = unsafe { create_tree_node(key, addr, Color::Red, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *mut ImplicitRbTreeNode).cast::<()>()) }))) };
                self.right = Some(node);
                Self::recolor_node(node.0.as_ptr().cast::<ImplicitRbTreeNode>());
                return;
            }
            if let Some(left) = self.left {
                unsafe { left.tree_node_mut().insert(key, addr); }
                return;
            }
            let node = unsafe { create_tree_node(key, addr, Color::Red, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *mut ImplicitRbTreeNode).cast::<()>()) }))) };
            self.left = Some(node);
            Self::recolor_node(node.0.as_ptr().cast::<ImplicitRbTreeNode>());
        }

        fn recolor_node(mut node: *mut ImplicitRbTreeNode) {
            let mut curr_node = node;
            while let Some(parent) = unsafe { curr_node.as_ref().unwrap_unchecked() }.parent_ptr() {
                if unsafe { parent.tree_node() }.is_black() {
                    // the parent is black so we have nothing to do here.
                    break;
                }
                if let Some(gp) = unsafe { parent.tree_node() }.parent_ptr() {
                    let uncle = unsafe { parent.tree_node() }.sibling();
                    let uncle_red = uncle.map_or(false, |uncle| unsafe { uncle.tree_node() }.is_red());
                    if uncle_red {
                        unsafe { parent.tree_node_mut() }.set_color(Color::Black);
                        unsafe { uncle.unwrap_unchecked().tree_node_mut() }.set_color(Color::Black);
                        // update grand parent
                        let grand_parent = unsafe { parent.tree_node().parent_ptr().unwrap_unchecked() };
                        unsafe { grand_parent.tree_node_mut() }.set_color(Color::Red);
                        curr_node = grand_parent.0.as_ptr().cast::<ImplicitRbTreeNode>();
                        continue;
                    }
                    let dir = unsafe { parent.tree_node() }.child_dir(curr_node);
                    let parent_dir = unsafe { gp.tree_node() }.child_dir(parent.raw_ptr());
                    let rotations = Self::map_rotations(parent_dir, dir);
                    todo!()
                } else {
                    unreachable!();
                }
            }
            // TODO: recolor root node!
        }

        fn rotate_right(&mut self) {
            let right = self.right;
            self.right = if self.parent.is_null() {
                None
            } else {
                Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(self.parent) }))
            };
            if let Some(mut right) = right {
                unsafe { right.tree_node_mut() }.parent = self.parent;
            }
            if let Some(mut parent) = unsafe { self.parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                let this = self as *mut ImplicitRbTreeNode;
                parent.left = right;
                let new_parent = parent.parent;
                parent.parent = this.cast::<()>();
                self.parent = new_parent;
                if let Some(mut new_parent) = unsafe { new_parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                    if new_parent.child_dir(parent) == Direction::Right {
                        new_parent.right = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    } else {
                        new_parent.left = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    }
                }
            } else {
                self.parent = null_mut();
            }
        }

        fn rotate_left(&mut self) {
            let left = self.left;
            self.left = if self.parent.is_null() {
                None
            } else {
                Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(self.parent) }))
            };
            if let Some(mut left) = left {
                let left = unsafe { left.tree_node_mut() };
                left.parent = self.parent;
                println!("rl 0");
            }
            if let Some(mut parent) = unsafe { self.parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                let this = self as *mut ImplicitRbTreeNode;
                assert_eq!(parent.child_dir(this), Direction::Right);
                parent.right = left;
                let new_parent = parent.parent;
                parent.parent = this.cast::<()>();
                self.parent = new_parent;
                if let Some(mut new_parent) = unsafe { new_parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                    if new_parent.child_dir(parent) == Direction::Left {
                        new_parent.left = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    } else {
                        new_parent.right = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    }
                }
                println!("rl 1 {:?}", new_parent);
            } else {
                self.parent = null_mut();
                assert!(left.is_none());
                println!("rl 2");
            }
        }

        #[inline]
        fn map_rotations(parent: Direction, child: Direction) -> (Direction, Option<Direction>) {
            // for cases like:
            // \
            //  P
            //   \
            //    C
            if parent == child {
                return (parent.rev(), None);
            }
            (child.rev(), Some(parent.rev()))
        }

        unsafe fn remove(&mut self, key: usize) -> NonNull<()> {
            if self.key < key {// FIXME: check if key matches with right or left!
                let mut right = unsafe { self.right.unwrap().tree_node_mut() };
                if right.key == key {
                    // FIXME: remove right
                    return todo!();
                }
                right.remove(key)
            } else {
                let left = unsafe { self.left.unwrap().tree_node_mut() };
                if left.key == key {
                    // FIXME: remove left
                    return todo!();
                }
                left.remove(key)
            }
        }

        fn find_approx_ge(&self, approx_key: usize) -> Option<ImplicitRbTreeNodeRef> {
            match self.key.cmp(&approx_key) {
                Ordering::Less => {
                    if let Some(left) = self.left {
                        let node = unsafe { left.tree_node() };
                        if node.key >= approx_key {
                            return node.find_approx_ge(approx_key);
                        }
                    }
                    // just return this node as we are the smallest node that still satisfies the key's requirements
                    Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) }))
                }
                Ordering::Equal => Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) })),
                Ordering::Greater => None,
            }
        }

    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    enum Direction {
        Left,
        Right,
    }

    impl Direction {

        #[inline]
        fn rev(self) -> Self {
            match self {
                Direction::Left => Direction::Right,
                Direction::Right => Direction::Left,
            }
        }

    }

}

mod bit_tree_map {
    use std::mem::size_of;
    use std::ptr::{NonNull, null_mut};
    use cache_padded::CachePadded;
    use crate::linux::{CACHE_LINE_SIZE, CACHE_LINE_WORD_SIZE};
    use crate::util::min;

    pub(crate) struct BitTreeMap<const ELEMENT_SIZE: usize> {
        root: BitTreeMapNode<ELEMENT_SIZE>,
    }
    
    impl<const ELEMENT_SIZE: usize> BitTreeMap<ELEMENT_SIZE> {
        
        pub fn new() -> Self {
            Self {
                root: BitTreeMapNode::new_leaf(),
            }
        }

        pub fn insert_tip(&mut self) {
            if self.root.is_leaf_node() {

            }
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

    const LEAF_NODE_FLAG: usize = 1 << 0;
    const METADATA_MASK: usize = LEAF_NODE_FLAG;
    const PTR_MASK: usize = !METADATA_MASK;

    /// # Design
    /// Every non-leaf node has multiple children and may even have multiple sub-maps
    /// to manage said children, roughly speaking: `sub_map_cnt = (cache_line_size / (size_of::<usize>() * 8))`.
    /// Every leaf node has up `entries = ((cache_line_size - size_of::<usize>()) * 8)` entries (leaf-tips).
    #[repr(C)]
    pub(crate) struct BitTreeMapNode<const ELEMENT_SIZE: usize> {
        storage: CachePadded<[usize; NODE_SLOTS]>, // these are either simple bitmaps for normal nodes or small bitmaps and ptrs to other bitmaps for leaf nodes
        parent: *mut BitTreeMapNode<ELEMENT_SIZE>, // the LSB of this ptr indicates whether we are a leaf node or not.
        _align: [u16; 0], // align this struct at least to 2 bytes.
    }

    impl<const ELEMENT_SIZE: usize> BitTreeMapNode<ELEMENT_SIZE> {

        pub(crate) fn new() -> Self {
            Self {
                storage: Default::default(),
                parent: null_mut(),
                _align: [],
            }
        }

        pub(crate) fn new_leaf() -> Self {
            Self {
                storage: Default::default(),
                parent: null_mut().map_addr(|addr| addr | LEAF_NODE_FLAG),
                _align: [],
            }
        }

        pub(crate) fn alloc_free_entry(&mut self) -> Option<BitMapEntry> {

        }

        #[inline]
        pub(crate) fn is_leaf_node(&self) -> bool {
            self.parent as usize & LEAF_NODE_FLAG != 0
        }

        pub(crate) fn used_nodes(&self) -> usize {
            // FIXME: this may need to fetch multiple cache lines if we have a cache line size larger than 64 bytes on 64 bit systems.
            // FIXME: just place the whole bitmap next to each other to trade best case latency for predictability of the system's latency.
            let mut used_nodes = 0;
            for map_idx in 0..SUB_MAPS {
                let map = unsafe { *(self as *const BitTreeMapNode<ELEMENT_SIZE> as *mut usize).sub(1 + map_idx * SUB_MAP_SLOTS / 8 + map_idx) };
                used_nodes += map.count_ones();
            }
            used_nodes as usize
        }

        pub(crate) fn is_full(&self) -> bool {
            self.used_nodes() != NODE_SLOTS
        }

    }

    pub(crate) struct BitMapEntry {
        ptr: NonNull<()>,
    }

    pub(crate) struct BitTreeMapLeafTip<const ELEMENT_SIZE: usize> {
        element_cnt: usize,
        parent: *mut BitTreeMapNode<ELEMENT_SIZE>,
    }

    impl<const ELEMENT_SIZE: usize> BitTreeMapLeafTip<ELEMENT_SIZE> {

        pub(crate) fn alloc_free_entry(&mut self) -> Option<BitMapEntry> {
            // assume that we are inside the allocation page, such a page looks something like this:
            // [entries, bitmaps, leaf tip, metadata]
            let curr = self as *mut BitTreeMapLeafTip<ELEMENT_SIZE> as *mut ();
            // FIXME: can we get rid of this div_ceil - we could by caching!
            let bitmap_cnt = self.element_cnt.div_ceil(usize::BITS as usize);
            // traverse the map backwards to allow for the possibility that we don't have to fetch
            // another cache line if we are lucky
            let end = unsafe { curr.cast::<usize>().sub(1) };
            for i in 0..bitmap_cnt {
                let map_ptr = unsafe { end.sub(i) };
                let map = unsafe { *map_ptr };
                let idx = map.trailing_zeros();
                if idx != usize::BITS {
                    Some(BitMapEntry {
                        // FIXME: can we get rid of this get_page_size?
                        // ptr: unsafe { NonNull::new_unchecked((curr.cast::<usize>() as usize / get_page_size() + (self.element_cnt - i * 8 - (usize::BITS - idx))) as *mut ()) },
                        ptr: unsafe { NonNull::new_unchecked((curr.cast::<usize>().sub(i * size_of::<usize>() + idx)) as *mut ()) },
                    })
                }
            }
            None
        }

    }

}

const CACHE_LINE_SIZE: usize = align_of::<CachePadded<()>>();
const CACHE_LINE_WORD_SIZE: usize = CACHE_LINE_SIZE / size_of::<usize>();

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
