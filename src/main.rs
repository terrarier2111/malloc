#![no_std]

#![feature(core_intrinsics)]

use core::intrinsics::abort;

mod linux;
mod util;

fn main() {

}

#[panic_handler]
fn panic_handler() {
    abort();
}
