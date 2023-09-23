#[inline]
pub(crate) unsafe fn align_unaligned_ptr_up_to<const REGION_SIZE: usize>(ptr: *mut u8, len: usize, align: usize) -> *mut u8 {
    let end = ptr as usize + len;
    let additional = end % align;
    let region_start = ptr.add(len - (additional + REGION_SIZE));
    region_start
}

#[inline]
pub(crate) fn round_up_to_multiple_of(val: usize, to: usize) -> usize {
    let diff = val % to;
    val + (to - diff)
}

#[inline]
pub(crate) fn round_down_to_multiple_of(val: usize, to: usize) -> usize {
    let diff = val % to;
    val - diff
}

#[inline]
pub(crate) const fn min(left: usize, right: usize) -> usize {
    if left > right {
        right
    } else {
        left
    }
}

/// This function should be used instead of any specific
/// instrinsic or library function as there doesn't seem to
/// be a stable, fast way to abort in no_std contexts.
pub(crate) fn abort() -> ! {
    core::intrinsics::abort()
}
