#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(fs_test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::{asm, global_asm};
use lemon_shark::{println, ALLOCATOR, logln, trap_handler, filesystem};
use crate::filesystem::{dump, Error};

extern crate alloc;

use alloc::format;

use alloc::string::{ToString, String};

global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);

/// Custom `test runner` that resets the filesystem so that each test can run
/// in a clean environment.
pub fn fs_test_runner(tests: &[&dyn lemon_shark::Testable]) {
    println!("\nRunning {} tests...\n", tests.len());
    for test in tests {
        filesystem::api::reset();
        test.run();
    }
    println!("\n\nAll {} tests passed!\n", tests.len());
    lemon_shark::exit_qemu(0);
}

/// Custom `_start` function allows each test suite to explicitly initialize the things
/// it needs.
#[unsafe(no_mangle)]
pub extern "C" fn _start(_: usize, _: usize) -> ! {
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
    use filesystem::api::{create_file, write_to_file};


    // Create file in root directory
    let index = create_file("/text.txt").expect("Unable to create file");
    let content = "A".repeat(511);
    let bytes_written = write_to_file(index.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 511);

    // Create file with the same name again.
    let failed = create_file("/text.txt");
    assert!(failed.err() == Some(Error::DuplicatedEntry));

    // Create another file and write to it to create another block.
    let other_file = create_file("/other-text.txt").expect("Unable to create file");
    let content = "B".repeat(513);
    let bytes_written = write_to_file(other_file.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 513);

    let content = "C".repeat(5);
    let bytes_written = write_to_file(index.0 as usize, content).expect("Failed to write to file");
    assert_eq!(bytes_written, 5);
}

#[test_case]
fn create_directory_structure() {
    use filesystem::api::mkdir;

    mkdir("/test").expect("Could not create directory");
    mkdir("/test/foo").expect("Could not create nested directory");
    mkdir("/foo").expect("Could not create directory with same name in root");
}

#[test_case]
fn writing_multiple_blocks() {
    use filesystem::{Error, BLOCK_SIZE};
    use filesystem::api::{write_to_file, create_file};
    const MAX_FILE_SIZE: usize = 16 * BLOCK_SIZE;

    let content = "A".repeat(MAX_FILE_SIZE);
    let index = create_file("/test.txt").unwrap();
    write_to_file(index.0 as usize, content).unwrap();

    let res = write_to_file(index.0 as usize, String::from("overflow"));

    assert_eq!(res.err(), Some(Error::NoSpaceInFile));
}

#[test_case]
fn file_and_directory_with_same_name() {
    use filesystem::api::{create_file, write_to_file, mkdir};

    mkdir("/test").unwrap();
    mkdir("/test/x").unwrap();
    mkdir("/test/x.txt").unwrap();
}

#[test_case]
fn writing_to_directory_returns_error() {
    use filesystem::api::{create_file, write_to_file, mkdir};

    let index = mkdir("/test").unwrap();

    let res = write_to_file(index.0 as usize, "xd".to_string());

    assert_eq!(res.err(), Some(Error::NotAFile));
}

#[test_case]
fn mkdir_into_file_returns_error() {
    use filesystem::api::{create_file, write_to_file, mkdir};

    let index = mkdir("/test.txt").unwrap();

    let res = mkdir("/test.txt/huh");

    assert_eq!(res.err(), Some(Error::NotADirectory));
}


