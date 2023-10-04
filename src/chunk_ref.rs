use std::mem::size_of;

use self::meta::ChunkMeta;

// just note that if we have to align the content of a chunk to more than 2 words, we have to set another flag (maybe via a combination of LAST, FIRST and FREE flags)
// to the word before said chunk (if there isn't enough space for a useful chunk beforehand). Said word beforehand represents a 'ghost chunk' i.e a chunk without any
// payload and with just one word of metadata.

pub(crate) const CHUNK_METADATA_SIZE: usize = CHUNK_METADATA_SIZE_ONE_SIDE * 2;
pub(crate) const CHUNK_METADATA_SIZE_ONE_SIDE: usize = size_of::<usize>();

/// `START` indicates whether the stored reference is a reference to the chunk's start or end.
#[derive(Copy, Clone)]
pub(crate) struct ChunkRef<const START: bool>(pub(crate) *mut u8);

impl<const START: bool> ChunkRef<START> {

    #[inline]
    pub(crate) fn into_raw(self) -> *mut u8 {
        self.0
    }

    #[inline]
    fn read_meta_raw(&self) -> usize {
        if START {
            unsafe { *self.0.cast::<usize>() }
        } else {
            unsafe { *self.0.cast::<usize>().sub(1) }
        }
    }

    #[inline]
    fn write_meta_raw(&mut self, size: usize) {
        if START {
            unsafe { *self.0.cast::<usize>() = size; }
        } else {
            unsafe { *self.0.cast::<usize>().sub(1) = size; }
        }
    }

    #[inline]
    pub(crate) fn setup_own(&mut self, meta: ChunkMeta) {
        self.write_meta_raw(meta.into_raw());
    }

    #[inline]
    pub(crate) fn read_meta(&self) -> ChunkMeta {
        ChunkMeta::from_raw(self.read_meta_raw())
    }

    #[inline]
    pub(crate) fn read_size(&self) -> usize {
        ChunkMeta::from_raw(self.read_meta_raw()).size()
    }

    /// this doesn't modify the FIRST_CHUNK flag
    #[inline]
    pub(crate) fn update_size(&mut self, size: usize) {
        let metadata = ChunkMeta::from_raw(self.read_meta_raw()).set_size(size);
        self.write_meta_raw(metadata.into_raw());
    }

    #[inline]
    pub(crate) fn set_first(&mut self, first: bool) {
        let metadata = ChunkMeta::from_raw(self.read_meta_raw()).set_first(first);
        self.write_meta_raw(metadata.into_raw());
    }

    #[inline]
    pub(crate) fn is_first(&self) -> bool {
        ChunkMeta::from_raw(self.read_meta_raw()).is_first()
    }

    #[inline]
    pub(crate) fn set_last(&mut self, last: bool) {
        let metadata = ChunkMeta::from_raw(self.read_meta_raw()).set_last(last);
        self.write_meta_raw(metadata.into_raw());
    }

    #[inline]
    pub(crate) fn is_last(&self) -> bool {
        ChunkMeta::from_raw(self.read_meta_raw()).is_last()
    }

    #[inline]
    pub(crate) fn set_free(&mut self, free: bool) {
        let metadata = ChunkMeta::from_raw(self.read_meta_raw()).set_free(free);
        self.write_meta_raw(metadata.into_raw());
    }

    #[inline]
    pub(crate) fn is_free(&self) -> bool {
        ChunkMeta::from_raw(self.read_meta_raw()).is_free()
    }

}

impl ChunkRef<true> {

    #[inline]
    pub(crate) fn new_start(chunk_start: *mut u8) -> Self {
        Self(chunk_start)
    }

    #[inline]
    pub(crate) fn setup(&mut self, meta: ChunkMeta) {
        self.setup_own(meta);
        /*if self.into_end(size).0 as usize % 8 != 0 {
            panic!("unaligned: {} size: {}", self.into_end(size).0 as usize % 8, size % 8);
        }
        println!("checked alignment end {:?}", self.0);*/
        self.into_end(meta.size()).setup_own(meta);
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
    pub(crate) fn setup(&mut self, meta: ChunkMeta) {
        self.setup_own(meta);
        self.into_start(meta.size()).setup_own(meta);
    }

    #[inline]
    pub(crate) fn into_start(self, size: usize) -> ChunkRef<true> {
        ChunkRef(unsafe { self.0.sub(size) })
    }

}

pub(crate) mod meta {

// FIXME: ensure that we have alignment >= 8
const FIRST_CHUNK_FLAG: usize = 1 << 0;
const LAST_CHUNK_FLAG: usize = 1 << 1;
const FREE_CHUNK_FLAG: usize = 1 << 2;

const METADATA_MASK: usize = FIRST_CHUNK_FLAG | LAST_CHUNK_FLAG | FREE_CHUNK_FLAG;
const SIZE_MASK: usize = !METADATA_MASK;


#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkMeta(usize);

impl ChunkMeta {

    #[inline]
    pub const fn new(size: usize, first: bool, last: bool, free: bool) -> Self {
        let ret = if first {
            FIRST_CHUNK_FLAG
        } else {
            0
        };
        let ret = if last {
            ret | LAST_CHUNK_FLAG
        } else {
            ret
        };
        let ret = if free {
            ret | FREE_CHUNK_FLAG
        } else {
            ret
        };
        Self(ret)
    }

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    #[inline]
    pub const fn is_first(self) -> bool {
        self.0 & FIRST_CHUNK_FLAG != 0
    }

    #[inline]
    pub const fn is_last(self) -> bool {
        self.0 & LAST_CHUNK_FLAG != 0
    }

    #[inline]
    pub const fn is_free(self) -> bool {
        self.0 & FREE_CHUNK_FLAG != 0
    }

    #[inline]
    pub const fn size(self) -> usize {
        self.0 & SIZE_MASK
    }

    #[inline]
    pub const fn set_first(mut self, first: bool) -> Self {
        if first {
            self.0 |= FIRST_CHUNK_FLAG;
        } else {
            self.0 &= !FIRST_CHUNK_FLAG;
        }
        self
    }

    #[inline]
    pub const fn set_last(mut self, last: bool) -> Self {
        if last {
            self.0 |= LAST_CHUNK_FLAG;
        } else {
            self.0 &= !LAST_CHUNK_FLAG;
        }
        self
    }

    #[inline]
    pub const fn set_free(mut self, free: bool) -> Self {
        if free {
            self.0 |= FREE_CHUNK_FLAG;
        } else {
            self.0 &= !FREE_CHUNK_FLAG;
        }
        self
    }

    #[inline]
    pub const fn set_size(mut self, size: usize) -> Self {
        self.0 = (self.0 & METADATA_MASK) | size;
        self
    }

    #[inline]
    pub const fn into_raw(self) -> usize {
        self.0
    }

}

}