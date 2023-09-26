use std::mem::size_of;

use crate::{chunk_ref::CHUNK_METADATA_SIZE};

pub(crate) const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE;
pub(crate) const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();
pub(crate) const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;

// FIXME: get rid of the right part of the metadata

// FIXME: also we don't really need max_chunk_size anymore as the chunks have to know themselves when they are free
// and we will insert all free chunks into thread local caches.

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
        pub(crate) fn read_size(&self) -> usize {
            unsafe { *self.0.cast::<usize>() }
        }

        #[inline]
        pub(crate) fn write_size(&mut self, size: usize) {
            unsafe { *self.0.cast::<usize>() = size; }
        }

        #[inline]
        pub(crate) unsafe fn into_chunk_start(self) -> *mut u8 {
            self.0.add(ALLOC_METADATA_SIZE_ONE_SIDE)
        }

    }