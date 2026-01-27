#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lemon_shark::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::global_asm;

use lemon_shark::allocator::FreeListAllocator;
use lemon_shark::{timer, interrupts, trap_handler};
use core::arch::asm;

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
    interrupts::init();

    test_main();
    loop {}
}

fn make_alloc() -> FreeListAllocator {
    let mut alloc = FreeListAllocator { head: None };
    unsafe { alloc.init() };
    alloc
}

#[test_case]
fn benchmark_1000_allocations() {
    use lemon_shark::println;

    let mut alloc = make_alloc();

    let layout = core::alloc::Layout::from_size_align(1000, 8).unwrap();

    let start = timer::rdtime();

    for _ in 0..1000 {
        let ptr = alloc.alloc(layout);
        alloc.dealloc(ptr, layout);
    }

    let end = timer::rdtime();
    let elapsed = end - start;

    // Assuming 10 MHz timebase (adjust for your system)
    let frequency = 10_000_000;
    let total_us = (elapsed * 1_000_000) / frequency;
    let per_op_ns = (elapsed * 1_000_000_000) / (frequency * 2000); // 2000 ops (alloc+dealloc)

    println!("1000 alloc/dealloc pairs:");
    println!("  Total: {} μs", total_us);
    println!("  Per operation: {} ns", per_op_ns);
    println!("  Cycles per op: {}", elapsed / 2000);
}

#[test_case]
fn benchmark_10000_allocations() {
    use lemon_shark::println;

    let mut alloc = make_alloc();

    let layout = core::alloc::Layout::from_size_align(10000, 8).unwrap();

    let start = timer::rdtime();

    for _ in 0..10000 {
        let ptr = alloc.alloc(layout);
        alloc.dealloc(ptr, layout);
    }

    let end = timer::rdtime();
    let elapsed = end - start;

    // Assuming 10 MHz timebase (adjust for your system)
    let frequency = 10_000_000;
    let total_us = (elapsed * 1_000_000) / frequency;
    let per_op_ns = (elapsed * 1_000_000_000) / (frequency * 2000); // 2000 ops (alloc+dealloc)

    println!("10_000 alloc/dealloc pairs:");
    println!("  Total: {} μs", total_us);
    println!("  Per operation: {} ns", per_op_ns);
    println!("  Cycles per op: {}", elapsed / 2000);
}

#[test_case]
fn benchmark_100000_allocations() {
    use lemon_shark::println;

    let mut alloc = make_alloc();

    let layout = core::alloc::Layout::from_size_align(100000, 8).unwrap();

    let start = timer::rdtime();

    for _ in 0..100000 {
        let ptr = alloc.alloc(layout);
        alloc.dealloc(ptr, layout);
    }

    let end = timer::rdtime();
    let elapsed = end - start;

    // Assuming 10 MHz timebase (adjust for your system)
    let frequency = 10_000_000;
    let total_us = (elapsed * 1_000_000) / frequency;
    let per_op_ns = (elapsed * 1_000_000_000) / (frequency * 2000); // 2000 ops (alloc+dealloc)

    println!("100_000 alloc/dealloc pairs:");
    println!("  Total: {} μs", total_us);
    println!("  Per operation: {} ns", per_op_ns);
    println!("  Cycles per op: {}", elapsed / 2000);
}
