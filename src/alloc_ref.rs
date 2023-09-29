use std::mem::size_of;

use crate::{chunk_ref::CHUNK_METADATA_SIZE};

pub(crate) const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE;
pub(crate) const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
pub(crate) const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

// FIXME: get rid of the right part of the metadata

// FIXME: also we don't really need max_chunk_size anymore as the chunks have to know themselves when they are free
// and we will insert all free chunks into thread local caches.

const BUCKET_FLAG: usize = 1 << (usize::BITS - 1);
const METADATA_MASK: usize = BUCKET_FLAG;
const SIZE_MASK: usize = !METADATA_MASK;

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
        pub(crate) fn setup(&mut self, size: usize) {
            self.write_size(size);
        }

        #[inline]
        fn read_size_raw(&self) -> usize {
            unsafe { *self.0.cast::<usize>() }
        }

        /// this size value actually represents the size of the allocation in case
        /// of a huge allocation and it contains the element count and in the higher bits it contains the element size of the bucket
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

        #[inline]
        pub(crate) fn is_bucket(&self) -> bool {
            self.read_size_raw() & BUCKET_FLAG != 0         
        }

        #[inline]
        pub(crate) fn set_bucket(&mut self, bucket: bool) {
            let flags = if bucket {
                BUCKET_FLAG
            } else {
                0
            };
            self.write_size_raw(self.read_size() | flags);
        }

        #[inline]
        pub(crate) unsafe fn into_chunk_start(self) -> *mut u8 {
            self.0.add(ALLOC_METADATA_SIZE_ONE_SIDE)
        }

    }