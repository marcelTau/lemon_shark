#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lemon_shark::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::global_asm;

use lemon_shark::allocator::FreeListAllocator;
use lemon_shark::{ALLOCATOR, logln, trap_handler, filesystem};

use crate::filesystem::{dump, Error};

use core::arch::asm;

extern crate alloc;

use alloc::string::String;

global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);

/// Custom `_start` function allows each test suite to explicitly initialize the things
/// it needs.
#[unsafe(no_mangle)]
pub extern "C" fn _start(_: usize, _: usize) -> ! {
    unsafe extern "C" {
        static _trap_stack_top: u8;
        static _heap_top: u8;
    }

    // Set the `sscratch` register to a 'known good' stack that the `trap_handler` can use.
    unsafe {
        let trap_stack = &_trap_stack_top as *const u8 as usize;
        asm!("csrw sscratch, {}", in(reg) trap_stack);
    }

    trap_handler::init();

    unsafe { ALLOCATOR.init() };

    filesystem::init();

    test_main();
    loop {}
}

#[test_case]
fn bitmap_tests() {
    use lemon_shark::bitmap::Bitmap;
    // 2 WORDS = 64 bytes
    let mut bm = Bitmap::<2>::new();

    for i in 0..64 {
        assert!(!bm.is_set(i as u32));
    }

    let free = bm.find_free().unwrap();
    assert_eq!(free, 0);
    bm.set(free);

    let free = bm.find_free().unwrap();
    assert_eq!(free, 1);
    bm.set(free);

}


#[test_case]
fn writing_to_file() {
    // Create file in root directory
    let index = filesystem::api::create_file(String::from("/text.txt")).expect("Unable to create file");
    let content = "A".repeat(511);
    let bytes_written = filesystem::api::write_to_file(index.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 511);

    // Create file with the same name again.
    let failed = filesystem::api::create_file(String::from("/text.txt"));
    assert!(failed.err() == Some(Error::DuplicatedEntry));

    // Create another file and write to it to create another block.
    let other_file = filesystem::api::create_file(String::from("/other-text.txt")).expect("Unable to create file");
    let content = "B".repeat(513);
    let bytes_written = filesystem::api::write_to_file(other_file.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 513);

    let content = "C".repeat(5);
    let bytes_written = filesystem::api::write_to_file(index.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 5);

    // dump();
}
