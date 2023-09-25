use std::mem::size_of;

use crate::{sys::MIN_ALIGN, chunk_ref::CHUNK_METADATA_SIZE};

pub(crate) const ALLOC_METADATA_SIZE: usize = ALLOC_METADATA_SIZE_ONE_SIDE * 2;
pub(crate) const ALLOC_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>() * 2;
pub(crate) const ALLOC_FULL_INITIAL_METADATA_SIZE: usize = ALLOC_METADATA_SIZE + CHUNK_METADATA_SIZE;
pub(crate) const ALLOC_FULL_INITIAL_METADATA_PADDING: usize = {
    let diff = ALLOC_FULL_INITIAL_METADATA_SIZE % MIN_ALIGN;
    if diff == 0 {
        0
    } else {
        MIN_ALIGN - diff
    }
};

// FIXME: get rid of the right part of the metadata

// FIXME: also we don't really need max_chunk_size anymore as the chunks have to know themselves when they are free
// and we will insert all free chunks into thread local caches.

#[inline]
pub(crate) unsafe fn alloc_chunk_start(alloc: *mut u8) -> *mut u8 {
    alloc.add(ALLOC_METADATA_SIZE_ONE_SIDE)
}

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

        #[inline]
        pub(crate) unsafe fn into_chunk_start(self) -> *mut u8 {
            self.0.add(ALLOC_METADATA_SIZE_ONE_SIDE)
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
        pub(crate) fn setup_own(&mut self, size: usize, chunk_size: usize) {
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