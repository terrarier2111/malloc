#![feature(core_intrinsics)]

use allocator::{alloc, dealloc};

mod linux;
mod util;

fn main() {
    println!("main!");
    let alloced = alloc(20);
    println!("errno: {}", errno::errno());
    println!("alloc: {:?}", alloced);
    dealloc(alloced);
}
