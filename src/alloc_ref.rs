use std::{mem::size_of, ptr::NonNull};

use crate::{chunk_ref::{CHUNK_METADATA_SIZE, ChunkRef, meta::ChunkMeta}, util::align_unaligned_ptr_up_to};

pub(crate) const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE;
pub(crate) const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
pub(crate) const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

// FIXME: also we don't really need max_chunk_size anymore as the chunks have to know themselves when they are free
// and we will insert all free chunks into thread local caches.

const BUCKET_TY_FLAG: usize = 1 << (usize::BITS - 1);


const CHUNK_METADATA_MASK: usize = BUCKET_TY_FLAG;
const CHUNK_SIZE_MASK: usize = !CHUNK_METADATA_MASK;


const BUCKET_IDX_MASK: usize = 0b1111;
const REMAINING_ELEM_CNT_MASK: usize = !(BUCKET_IDX_MASK | BUCKET_TY_FLAG); // we store a counter inside the bucket meta that just counts the local free slots as a fast path?

#[derive(Clone, Copy)]
pub struct BucketAlloc(NonNull<u8>);

impl BucketAlloc {

    #[inline]
    pub(crate) fn into_raw(self) -> *mut u8 {
        self.0.as_ptr()
    }

    #[inline]
    pub(crate) fn read_bucket_idx(self) -> usize {
        self.read_raw() & BUCKET_IDX_MASK
    }

    #[inline]
    pub(crate) fn read_remaining_elem_cnt(self) -> usize {
        (self.read_raw() & REMAINING_ELEM_CNT_MASK) >> BUCKET_IDX_MASK.leading_ones()
    }

    #[inline]
    pub(crate) fn update_remaining_elem_cnt(self, elem_cnt: usize) {
        self.write_raw(elem_cnt | (self.read_raw() & !REMAINING_ELEM_CNT_MASK))
    }

    fn setup(self, bucket_idx: usize) {
        self.write_raw(bucket_idx | BUCKET_TY_FLAG);
    }

    fn write_raw(mut self, val: usize) {
        unsafe { self.0.as_ptr().cast::<usize>().write(val); }
    }

    #[inline]
    fn read_raw(self) -> usize {
        unsafe { *self.0.as_ptr().cast::<usize>() }
    }

}

// FIXME: the information contained inside this is just redundant as the chunk_ref already contains the size!
pub(crate) struct ChunkedAlloc(NonNull<u8>);

impl ChunkedAlloc {

    #[inline]
    pub(crate) fn setup(&mut self, meta: ChunkMeta) {
        ChunkRef::<true>(self.0.as_ptr()).setup_own(meta);
    }

    pub(crate) fn setup_double_sided(&mut self, meta: ChunkMeta) {
        ChunkRef::<true>(self.0.as_ptr()).setup(meta);
    }

    /// returns the size of the allocation in bytes
    #[inline]
    pub(crate) fn read_size(&self) -> usize {
        ChunkRef::<true>(self.0.as_ptr()).read_size()
    }

    #[inline]
    pub(crate) fn update_size(&mut self, size: usize) {
        ChunkRef::<true>(self.0.as_ptr()).update_size(size);
    }

    #[inline]
    pub(crate) fn update_size_double_sided(&mut self, size: usize) {
        let meta = ChunkRef::<true>(self.0.as_ptr()).read_meta().set_size(size);
        ChunkRef::<true>(self.0.as_ptr()).update_size(size);
        ChunkRef::<true>(self.0.as_ptr()).into_end(size).setup_own(meta);
    }

}

#[derive(Copy, Clone)]
    pub(crate) struct AllocRef(NonNull<u8>);

    impl AllocRef {

        #[inline]
        pub(crate) fn into_raw(self) -> NonNull<u8> {
            self.0
        }

        #[inline]
        pub(crate) fn new_start(alloc_start: NonNull<u8>) -> Self {
            Self(alloc_start)
        }

        /// This method may only be used if there are no explicit alignment requirements.
        #[inline]
        pub(crate) fn setup_bucket(&mut self, bucket_idx: usize) {
            BucketAlloc(self.0).setup(bucket_idx);
        }

        /// This method may only be used if there are no explicit alignment requirements.
        #[inline]
        pub(crate) fn setup_chunked(&mut self, size: usize, free: bool) {
            // FIXME: how do we support arbitratily large allocations with arbitary alignment requirements without having to encode the metadata out-of-line?

            // one option would be to allocate the required amount of memory * 2 + 2 and store the metadata in the page directly before the start ptr
            // and return a ptr to the start of the next page and on dealloc special case such page aligned ptrs to read the metadata from a page before.
            ChunkedAlloc(self.0).setup(ChunkMeta::empty().set_first(true).set_last(true).set_free(free).set_size(size));
        }

        #[inline]
        pub(crate) fn read_bucket(self) -> BucketAlloc {
            BucketAlloc(self.0)
        }

        #[inline]
        pub(crate) fn read_chunked(self) -> ChunkedAlloc {
            ChunkedAlloc(self.0)
        }

        #[inline]
        fn read_raw(self) -> usize {
            unsafe { *self.0.as_ptr().cast::<usize>() }
        }

        #[inline]
        pub(crate) fn is_bucket(&self) -> bool {
            self.read_raw() & BUCKET_TY_FLAG != 0
        }

        #[inline]
        pub(crate) unsafe fn into_start(self) -> *mut u8 {
            self.0.as_ptr().add(ALLOC_METADATA_SIZE_ONE_SIDE)
        }

    }

/// used to split up an allocation for a memory region that has an alignment > PAGE_SIZE / 2
#[inline]
pub(crate) fn from_base_overaligned(base: *mut (), size: usize, align: usize,  req_size: usize) -> (Option<ChunkedAlloc>, ChunkedAlloc, Option<ChunkedAlloc>) {
    let aligned = unsafe { align_unaligned_ptr_up_to(base.cast::<u8>(), size, align, req_size) };
    let left = base.cast::<u8>();
    let right = unsafe { base.cast::<u8>().add(req_size) };
    let left = if left == aligned {
        None
    } else {
        let mut left = ChunkedAlloc(unsafe { NonNull::new_unchecked(left) });
        left.setup(ChunkMeta::empty().set_first(true).set_free(true).set_size(aligned as usize - left.0.as_ptr() as usize));
        Some(left)
    };
    let right = if right == aligned {
        None
    } else {
        let mut right = ChunkedAlloc(unsafe { NonNull::new_unchecked(right) });
        right.setup(ChunkMeta::empty().set_last(true).set_free(true).set_size(right.0.as_ptr() as usize  - aligned as usize));
        Some(right)
    };
    (left, ChunkedAlloc(unsafe { NonNull::new_unchecked(aligned) }), right)
}
