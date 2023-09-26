use std::cmp::Ordering;
    use std::mem::size_of;
    use std::ptr::{NonNull, null_mut};

    use crate::util::abort;

    const RED: usize = 0 << 0;
    const BLACK: usize = 1 << 0;

    const COLOR_MASK: usize = 1 << 0;

    const PTR_MASK: usize = !COLOR_MASK;

    pub const NODE_SIZE: usize = size_of::<ImplicitRbTreeNode>();

    #[derive(Copy, Clone, PartialEq)]
    #[repr(usize)]
    pub enum Color {
        Red = RED,
        Black = BLACK,
    }

    #[repr(C)]
    pub struct ImplicitRbTree {
        root: Option<ImplicitRbTreeNodeRef>,
    }
    
    impl ImplicitRbTree {
        
        #[inline]
        pub const fn new() -> Self {
            Self {
                root: None,
            }
        }

        pub fn insert(&mut self, key: usize, addr: NonNull<()>) {
            match self.root {
                None => {
                    self.root = Some(unsafe { create_tree_node(key, addr, Color::Black, None) });
                }
                Some(root) => {
                    unsafe { root.tree_node_mut() }.insert(key, addr, root.raw_ptr());
                }
            }
        }

        pub fn remove(&mut self, key: usize) -> Option<NonNull<()>> { // FIXME: support removal of ptrs instead of keys!
            let root = unsafe { self.root.as_mut().unwrap().tree_node_mut() };
            if !root.has_children() {
                return unsafe { self.root.take().map(|node| node.0) }; // FIXME: should we return the data ptr instead?
            }
            Some(unsafe { root.remove(key, &mut self.root as *mut Option<ImplicitRbTreeNodeRef>) })
        }

        pub fn find_approx_ge(&self, approx_key: usize) -> Option<ImplicitRbTreeNodeRef> {
            match self.root {
                None => None,
                Some(node) => unsafe { node.tree_node() }.find_approx_ge(approx_key),
            }
        }

        #[inline]
        pub const fn root_ref(&self) -> Option<ImplicitRbTreeNodeRef> {
            self.root
        }
        
    }

    #[derive(Copy, Clone, Debug, PartialEq)]
    pub struct ImplicitRbTreeNodeRef(NonNull<()>);

    impl ImplicitRbTreeNodeRef {

        #[inline]
        pub fn new(ptr: NonNull<()>) -> Self {
            Self(ptr)
        }

        #[inline]
        pub(crate) unsafe fn tree_node<'a>(self) -> &'a ImplicitRbTreeNode {
            unsafe { self.0.cast::<ImplicitRbTreeNode>().as_ref() }
        }

        #[inline]
        pub(crate) unsafe fn tree_node_mut<'a>(self) -> &'a mut ImplicitRbTreeNode {
            unsafe { self.0.cast::<ImplicitRbTreeNode>().as_mut() }
        }

        #[inline]
        pub(crate) unsafe fn data_ptr(self) -> NonNull<u8> {
            unsafe { NonNull::new_unchecked(self.0.cast::<u8>().as_ptr().add(size_of::<ImplicitRbTreeNode>())) }
        }

        #[inline]
        pub(crate) fn raw_ptr(self) -> *mut ImplicitRbTreeNode {
            self.0.as_ptr().cast::<ImplicitRbTreeNode>()
        }

    }

    #[inline]
    unsafe fn create_tree_node(key: usize, addr: NonNull<()>, color: Color, parent: Option<ImplicitRbTreeNodeRef>) -> ImplicitRbTreeNodeRef {
        unsafe { addr.cast::<ImplicitRbTreeNode>().as_ptr().write(ImplicitRbTreeNode {
            parent: parent.map_or(null_mut(), |node| node.0.as_ptr()).map_addr(|addr| addr | color as usize),
            left: None,
            right: None,
            key,
        }); }
        ImplicitRbTreeNodeRef(addr)
    }

    #[repr(align(2))]
    #[repr(C)]
    pub struct ImplicitRbTreeNode {
        key: usize,
        parent: *mut (),
        left: Option<ImplicitRbTreeNodeRef>,
        right: Option<ImplicitRbTreeNodeRef>,
    }

    impl ImplicitRbTreeNode {

        #[inline]
        unsafe fn new_red(parent: ImplicitRbTreeNodeRef, key: usize) -> Self {
            Self {
                key,
                parent: parent.0.as_ptr().map_addr(|addr| addr | RED),
                left: None,
                right: None,
            }
        }

        #[inline]
        unsafe fn new_black(parent: ImplicitRbTreeNodeRef, key: usize) -> Self {
            Self {
                key,
                parent: parent.0.as_ptr().map_addr(|addr| addr | BLACK),
                left: None,
                right: None,
            }
        }

        #[inline]
        fn parent_ptr(&self) -> Option<ImplicitRbTreeNodeRef> {
            let ptr = self.parent.mask(PTR_MASK);
            if ptr.is_null() {
                return None;
            }
            Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(ptr) }))
        }

        #[inline]
        pub(crate) fn is_red(&self) -> bool {
            (self.parent as usize) & COLOR_MASK == RED
        }

        #[inline]
        pub(crate) fn is_black(&self) -> bool {
            (self.parent as usize) & COLOR_MASK == BLACK
        }

        #[inline]
        pub(crate) fn set_color(&mut self, color: Color) {
            self.parent = self.parent.map_addr(|raw| (raw & PTR_MASK) | color as usize);
        }

        #[inline]
        pub(crate) fn get_color(&self) -> Color {
            if self.is_red() {
                Color::Red
            } else {
                Color::Black
            }
        }

        #[inline]
        fn sibling(&self) -> Option<ImplicitRbTreeNodeRef> {
            match self.parent_ptr() {
                None => None,
                Some(parent_ptr) => {
                    let right = unsafe { parent_ptr.tree_node().right };
                    if right.map_or(null_mut(), |ptr| ptr.0.as_ptr()).cast_const().cast::<ImplicitRbTreeNode>() == self as *const ImplicitRbTreeNode {
                        let left = unsafe { parent_ptr.tree_node().left };
                        return left;
                    }
                    debug_assert_eq!(unsafe { parent_ptr.tree_node().left }, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) })));
                    right
                }
            }
        }

        #[inline]
        fn child_dir(&self, child: *mut ImplicitRbTreeNode) -> Direction {
            if let Some(right) = self.right {
                if right == ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(child.cast::<()>()) }) {
                    return Direction::Right;
                }
            }
            debug_assert_eq!(self.left, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(child.cast::<()>()) })));
            Direction::Left
        }

        #[inline]
        fn has_children(&self) -> bool {
            self.left.is_some() || self.right.is_some()
        }

        // FIXME: should we really pass an additional root argument around or should we instead just
        // re-read the root node from the TLS?
        fn insert(&mut self, key: usize, addr: NonNull<()>, root: *mut ImplicitRbTreeNode) {
            if self.key < key {
                if let Some(right) = self.right {
                    unsafe { right.tree_node_mut().insert(key, addr, root); }
                    return;
                }
                let node = unsafe { create_tree_node(key, addr, Color::Red, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *mut ImplicitRbTreeNode).cast::<()>()) }))) };
                self.right = Some(node);
                Self::recolor_node(node.0.as_ptr().cast::<ImplicitRbTreeNode>(), root);
                return;
            }
            if let Some(left) = self.left {
                unsafe { left.tree_node_mut().insert(key, addr, root); }
                return;
            }
            let node = unsafe { create_tree_node(key, addr, Color::Red, Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *mut ImplicitRbTreeNode).cast::<()>()) }))) };
            self.left = Some(node);
            Self::recolor_node(node.0.as_ptr().cast::<ImplicitRbTreeNode>(), root);
        }

        fn recolor_node(mut node: *mut ImplicitRbTreeNode, root: *mut ImplicitRbTreeNode) {
            let mut curr_node = node;
            while let Some(parent) = unsafe { curr_node.as_ref().unwrap_unchecked() }.parent_ptr() {
                if unsafe { parent.tree_node() }.is_black() {
                    // the parent is black so we have nothing to do here.
                    break;
                }
                if let Some(gp) = unsafe { parent.tree_node() }.parent_ptr() {
                    let uncle = unsafe { parent.tree_node() }.sibling();
                    let uncle_red = uncle.map_or(false, |uncle| unsafe { uncle.tree_node() }.is_red());
                    if uncle_red {
                        unsafe { parent.tree_node_mut() }.set_color(Color::Black);
                        unsafe { uncle.unwrap_unchecked().tree_node_mut() }.set_color(Color::Black);
                        // update grand parent
                        let grand_parent = unsafe { parent.tree_node().parent_ptr().unwrap_unchecked() };
                        unsafe { grand_parent.tree_node_mut() }.set_color(Color::Red);
                        curr_node = grand_parent.0.as_ptr().cast::<ImplicitRbTreeNode>();
                        continue;
                    }
                    let dir = unsafe { parent.tree_node() }.child_dir(curr_node);
                    let parent_dir = unsafe { gp.tree_node() }.child_dir(parent.raw_ptr());
                    let rotations = Self::map_rotations(parent_dir, dir);
                    match rotations.1 {
                        Some(first_rotation) => {
                            unsafe { &mut *curr_node }.rotate(first_rotation);
                            unsafe { &mut *parent.tree_node_mut() }.rotate(rotations.0);
                            unsafe { &mut *curr_node }.set_color(Color::Black);
                        },
                        None => {
                            unsafe { &mut *parent.tree_node_mut() }.rotate(rotations.0);
                            unsafe { &mut *parent.tree_node_mut() }.set_color(Color::Black);
                        },
                    }
                    unsafe { &mut *gp.tree_node_mut() }.set_color(Color::Red);
                    // try checking the next node
                    if let Some(next_check) = unsafe { curr_node.as_ref().unwrap_unchecked() }.parent_ptr() {
                        curr_node = next_check.raw_ptr();
                    } else {
                        // we went through all things, so we should be fine now
                        break;
                    }
                } else {
                    abort();
                }
            }
            unsafe { &mut *root }.set_color(Color::Black);
        }

        fn rotate_right(&mut self) {
            let right = self.right;
            self.right = if self.parent.is_null() {
                None
            } else {
                Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(self.parent) }))
            };
            if let Some(mut right) = right {
                unsafe { right.tree_node_mut() }.parent = self.parent;
            }
            if let Some(mut parent) = unsafe { self.parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                let this = self as *mut ImplicitRbTreeNode;
                parent.left = right;
                let new_parent = parent.parent;
                parent.parent = this.cast::<()>();
                self.parent = new_parent;
                if let Some(mut new_parent) = unsafe { new_parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                    if new_parent.child_dir(parent) == Direction::Right {
                        new_parent.right = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    } else {
                        new_parent.left = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    }
                }
            } else {
                self.parent = null_mut();
            }
        }

        fn rotate_left(&mut self) {
            let left = self.left;
            self.left = if self.parent.is_null() {
                None
            } else {
                Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(self.parent) }))
            };
            if let Some(mut left) = left {
                let left = unsafe { left.tree_node_mut() };
                left.parent = self.parent;
                println!("rl 0");
            }
            if let Some(mut parent) = unsafe { self.parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                let this = self as *mut ImplicitRbTreeNode;
                assert_eq!(parent.child_dir(this), Direction::Right);
                parent.right = left;
                let new_parent = parent.parent;
                parent.parent = this.cast::<()>();
                self.parent = new_parent;
                if let Some(mut new_parent) = unsafe { new_parent.cast::<ImplicitRbTreeNode>().as_mut() } {
                    if new_parent.child_dir(parent) == Direction::Left {
                        new_parent.left = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    } else {
                        new_parent.right = Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked(this.cast::<()>()) }));
                    }
                }
                println!("rl 1 {:?}", new_parent);
            } else {
                self.parent = null_mut();
                assert!(left.is_none());
                println!("rl 2");
            }
        }

        fn rotate(&mut self, dir: Direction) {
            match dir {
                Direction::Left => self.rotate_left(),
                Direction::Right => self.rotate_right(),
            }
        }

        #[inline]
        fn map_rotations(parent: Direction, child: Direction) -> (Direction, Option<Direction>) {
            // for cases like:
            // \
            //  P
            //   \
            //    C
            if parent == child {
                return (parent.rev(), None);
            }
            (child.rev(), Some(parent.rev()))
        }

        unsafe fn remove(&mut self, key: usize, own_ptr: *mut Option<ImplicitRbTreeNodeRef>) -> NonNull<()> {
            if self.key < key {// FIXME: check if key matches with right or left!
                let mut right = unsafe { self.right.unwrap().tree_node_mut() };
                if right.key == key {
                    let right = self.right.unwrap_unchecked().raw_ptr();
                    unsafe { Self::inner_removal(right, own_ptr); }
                    return unsafe { NonNull::new_unchecked(right.cast::<()>()) };
                }
                right.remove(key, &mut self.right as *mut Option<ImplicitRbTreeNodeRef>)
            } else {
                let left = unsafe { self.left.unwrap().tree_node_mut() };
                if left.key == key {
                    let left = self.left.unwrap_unchecked().raw_ptr();
                    unsafe { Self::inner_removal(left, own_ptr); }
                    return unsafe { NonNull::new_unchecked(left.cast::<()>()) };
                }
                left.remove(key, &mut self.left as *mut Option<ImplicitRbTreeNodeRef>)
            }
        }

        unsafe fn inner_removal(node: *mut ImplicitRbTreeNode, parent: *mut Option<ImplicitRbTreeNodeRef>) {
            let mut original_color = unsafe { &*node }.get_color();
            if unsafe { &*node }.left.is_none() {
                let right = unsafe { &*node }.right;
                unsafe { *parent = right; }
            } else if unsafe { &*node }.right.is_none() {
                let left = unsafe { &*node }.left;
                unsafe { *parent = left; }
            } else {
                // FIXME: this whole branch is REALLY sus, audit it properly!
                let right = unsafe { &*node }.right;
                let (new_node, deep) = {
                    let mut result = unsafe { right.unwrap_unchecked() };
                    let mut deep = false;
                    while let Some(left) = unsafe { result.tree_node().left } {
                        result = left;
                        deep = true;
                    }
                    (result, deep)
                };
                let new_color = unsafe { new_node.tree_node() }.get_color();
                unsafe { new_node.tree_node_mut() }.set_color(original_color); // FIXME: is this step correct at this place?
                original_color = new_color;
                let child = unsafe { new_node.tree_node() }.right;
                if !deep {
                    // y = y ?
                } else {
                    unsafe { new_node.tree_node().parent_ptr().unwrap_unchecked().tree_node_mut().left = child; }
                }
            }
            if original_color == Color::Black {
                Self::fixup_removal(node);
            }
        }

        unsafe fn fixup_removal(node: *mut ImplicitRbTreeNode) {

        }

        fn find_approx_ge(&self, approx_key: usize) -> Option<ImplicitRbTreeNodeRef> {
            match self.key.cmp(&approx_key) {
                Ordering::Less => {
                    if let Some(left) = self.left {
                        let node = unsafe { left.tree_node() };
                        if node.key >= approx_key {
                            return node.find_approx_ge(approx_key);
                        }
                    }
                    // just return this node as we are the smallest node that still satisfies the key's requirements
                    Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) }))
                }
                Ordering::Equal => Some(ImplicitRbTreeNodeRef(unsafe { NonNull::new_unchecked((self as *const ImplicitRbTreeNode).cast_mut().cast::<()>()) })),
                Ordering::Greater => None,
            }
        }

    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    enum Direction {
        Left,
        Right,
    }

    impl Direction {

        #[inline]
        fn rev(self) -> Self {
            match self {
                Direction::Left => Direction::Right,
                Direction::Right => Direction::Left,
            }
        }

    }