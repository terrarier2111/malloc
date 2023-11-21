#![feature(core_intrinsics)]
#![feature(ptr_mask)]
#![feature(strict_provenance)]
#![feature(int_roundings)]
#![feature(thread_local)]

use allocator::{alloc, free};

mod util;

fn main() {
    println!("main!");
    let alloced = alloc(20);
    unsafe { *alloced = 9; }
    assert_eq!(unsafe { *alloced }, 9);
    println!("errno: {}", errno::errno());
    println!("alloc: {:?}", alloced);
    free(alloced);
}
