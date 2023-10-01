use std::mem::size_of;

use crate::{chunk_ref::CHUNK_METADATA_SIZE};

pub(crate) const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE;
pub(crate) const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
pub(crate) const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

// FIXME: get rid of the right part of the metadata

// FIXME: also we don't really need max_chunk_size anymore as the chunks have to know themselves when they are free
// and we will insert all free chunks into thread local caches.

const BUCKET_TY_FLAG: usize = 1 << (usize::BITS - 1);


const METADATA_MASK: usize = BUCKET_TY_FLAG;
const SIZE_MASK: usize = !METADATA_MASK;


const BUCKET_IDX_MASK: usize = 0b1111;
const ALLOCED_ELEM_CNT_MASK: usize = !(BUCKET_IDX_MASK | BUCKET_TY_FLAG); // we store a counter inside the bucket meta that just counts the local free slots as a fast path?

#[derive(Clone, Copy)]
pub struct BucketAlloc(*mut u8);

impl BucketAlloc {

    #[inline]
    pub(crate) fn into_raw(self) -> *mut u8 {
        self.0
    }

    #[inline]
    pub(crate) fn read_bucket_idx(self) -> usize {
        self.read_raw() & BUCKET_IDX_MASK
    }

    fn setup(self, bucket_idx: usize) {
        unsafe { self.0.cast::<usize>().write(bucket_idx | BUCKET_TY_FLAG); }
    }

    #[inline]
    fn read_raw(self) -> usize {
        unsafe { *self.0.cast::<usize>() }
    }

}

pub(crate) struct ChunkedAlloc(*mut u8);

impl ChunkedAlloc {

    #[inline]
    fn read_size_raw(&self) -> usize {
        unsafe { *self.0.cast::<usize>() }
    }

    /// this size value actually represents the size of the allocation in case
    /// of a huge allocation and it contains the element count and in the higher bits it contains the bucket index
    /// in case of 
    #[inline]
    pub(crate) fn read_size(&self) -> usize {
        self.read_size_raw() & SIZE_MASK
    }

    #[inline]
    fn write_size_raw(&mut self, size: usize) {
        unsafe { *self.0.cast::<usize>() = size; }
    }

    #[inline]
    pub(crate) fn write_size(&mut self, size: usize) {
        self.write_size_raw(size | (self.read_size_raw() & METADATA_MASK));
    }

}

#[derive(Copy, Clone)]
    pub(crate) struct AllocRef(*mut u8);

    impl AllocRef {

        #[inline]
        pub(crate) fn into_raw(self) -> *mut u8 {
            self.0
        }

        #[inline]
        pub(crate) fn new_start(alloc_start: *mut u8) -> Self {
            Self(alloc_start)
        }

        #[inline]
        pub(crate) fn setup_bucket(&mut self, bucket_idx: usize) {
            BucketAlloc(self.0).setup(bucket_idx);
        }

        #[inline]
        pub(crate) fn setup_chunked(&mut self, size: usize) {
            // FIXME: how do we support arbitratily large allocations with arbitary alignment requirements without having to encode the metadata out-of-line?

            // one option would be to allocate the required amount of memory * 2 + 2 and store the metadata in the page directly before the start ptr
            // and return a ptr to the start of the next page and on dealloc special case such page aligned ptrs to read the metadata from a page before.
            todo!()
        }

        #[inline]
        fn read_raw(self) -> usize {
            unsafe { *self.0.cast::<usize>() }
        }

        #[inline]
        pub(crate) fn is_bucket(&self) -> bool {
            self.read_raw() & BUCKET_TY_FLAG != 0         
        }

        #[inline]
        pub(crate) unsafe fn into_start(self) -> *mut u8 {
            self.0.add(ALLOC_METADATA_SIZE_ONE_SIDE)
        }

    }