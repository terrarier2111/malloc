#[inline]
pub(crate) unsafe fn align_unaligned_ptr_up_to(ptr: *mut u8, alloc_len: usize, align: usize, req_len: usize) -> *mut u8 {
    let end = ptr as usize + alloc_len;
    let additional = end % align;
    let region_start = ptr.add(alloc_len - (additional + req_len));
    region_start
}

#[inline]
pub(crate) const fn round_up_to_multiple_of(val: usize, to: usize) -> usize {
    let diff = val % to;
    val + (to - diff)
}

#[inline]
pub(crate) const fn round_down_to_multiple_of(val: usize, to: usize) -> usize {
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
