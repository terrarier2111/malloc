use std::ptr::null_mut;

pub struct LinkedList<T> {
    root: *mut Node<T>,
    len: usize,
}

impl<T> LinkedList<T> {

    #[inline]
    pub const fn new() -> Self {
        Self {
            root: null_mut(),
            len: 0,
        }
    }

    /// #Safety
    /// In order for this to be safe, the passed allocation pointer
    /// has to point to the beginning of an allocation that is
    /// at least size_of(T) + size_of(ptr) in size.
    #[inline]
    pub unsafe fn push(&mut self, node_addr: *mut (), val: T) {
        let node_addr = node_addr.cast::<Node<T>>();
        unsafe { node_addr.write(Node {
            val,
            next: self.root,
        }); }
        self.root = node_addr;
        self.len += 1;
    }

    #[inline]
    pub fn pop_front(&mut self) -> *mut T {
        if self.root.is_null() {
            return null_mut();
        }
        let ret = self.root;
        self.root = unsafe { ret.as_ref().unwrap_unchecked().next };
        self.len -= 1;
        ret.cast::<T>()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

}

#[repr(C)]
struct Node<T> {
    val: T,
    next: *mut Node<T>,
}
