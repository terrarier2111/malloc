use std::mem::size_of;

// just note that if we have to align the content of a chunk to more than 2 words, we have to set another flag (maybe via a combination of LAST, FIRST and FREE flags)
// to the word before said chunk (if there isn't enough space for a useful chunk beforehand). Said word beforehand represents a 'ghost chunk' i.e a chunk without any
// payload and with just one word of metadata.


pub(crate) const CHUNK_METADATA_SIZE: usize = CHUNK_METADATA_SIZE_ONE_SIDE * 2;
pub(crate) const CHUNK_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();

// FIXME: ensure that we have alignment >= 4
const FIRST_CHUNK_FLAG: usize = 1 << 0;
const LAST_CHUNK_FLAG: usize = 1 << 1;
const FREE_CHUNK_FLAG: usize = 1 << (usize::BITS - 1);
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
        self.read_size_raw() & SIZE_MASK
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