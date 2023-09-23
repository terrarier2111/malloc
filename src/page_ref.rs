use crate::util::round_down_to_multiple_of;

// FIXME: merge this with alloc_ref

#[derive(Clone, Copy, Debug)]
pub(crate) struct PageRef(*mut ());

impl PageRef {

    #[inline]
    pub unsafe fn from_unaligned_ptr(ptr: *mut (), page_size: usize) -> Self {
        Self(round_down_to_multiple_of(ptr as usize, page_size) as *mut ())
    }

    #[inline]
    pub unsafe fn from_page_ptr(ptr: *mut()) -> Self {
        Self(ptr)
    }

    #[inline]
    pub unsafe fn setup(&mut self, kind: PageKind) {
        let raw = match kind {
            PageKind::ChunkPage { alloc_offset } => alloc_offset | CHUNK_ALLOC_FLAG,
            PageKind::AllocPage { size } => size,
        };
        unsafe { self.0.cast::<usize>().write(raw); }
    }

    #[inline]
    pub fn kind(&self) -> PageKind {
        let raw = unsafe { self.0.cast::<usize>().read() };
        if raw & CHUNK_ALLOC_FLAG != 0 {
            PageKind::ChunkPage { alloc_offset: raw & DATA_MASK }
        } else {
            PageKind::AllocPage { size: raw }
        }
    }

    #[inline]
    pub fn into_content_start(self) -> *mut () {
        unsafe { self.0.cast::<usize>().add(1).cast::<()>() }
    }

}

const CHUNK_ALLOC_FLAG: usize = 1 << (usize::BITS - 1);
const FLAGS_MASK: usize = CHUNK_ALLOC_FLAG;
const DATA_MASK: usize = !FLAGS_MASK;

#[derive(Clone, Copy)]
pub enum PageKind {
    ChunkPage {
        /// the offset to the left side in unit of bytes
        /// to the page containing the central allocation.
        alloc_offset: usize,
    },
    AllocPage {
        /// the size of the allocation that starts at this page.
        size: usize,
    },
}

impl PageKind {

    #[inline]
    pub fn offset(&self) -> usize {
        match self {
            PageKind::ChunkPage { alloc_offset } => *alloc_offset,
            PageKind::AllocPage { .. } => core::intrinsics::abort(),
        }
    }

    #[inline]
    pub fn size(&self) -> usize {
        match self {
            PageKind::ChunkPage { .. } => core::intrinsics::abort(),
            PageKind::AllocPage { size } => *size,
        }
    }

    #[inline]
    fn into_raw(self) -> usize {
        match self {
            PageKind::ChunkPage { alloc_offset } => alloc_offset | CHUNK_ALLOC_FLAG,
            PageKind::AllocPage { size } => size,
        }
    }

}
