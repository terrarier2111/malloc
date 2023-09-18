#[inline]
pub(crate) unsafe fn align_unaligned_ptr_to<const REGION_SIZE: usize>(ptr: *mut u8, len: usize, align: usize) -> *mut u8 {
    let end = ptr as usize + len;
    let additional = end % align;
    let region_start = ptr.add(len - (additional + REGION_SIZE));
    region_start
}

#[inline]
pub(crate) fn round_up_to(val: usize, to: usize) -> usize {
    let diff = val % to;
    val + (to - diff)
}

#[inline]
pub(crate) const fn min(left: usize, right: usize) -> usize {
    if left > right {
        right
    } else {
        left
    }
}
