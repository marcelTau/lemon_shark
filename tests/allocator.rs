#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(lemon_shark::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::{arch::global_asm, panic::PanicInfo};

use lemon_shark::{Testable, log, logln, timer, interrupts, trap_handler};
use lemon_shark::heap::FreeListAllocator;

#[cfg(feature = "stats")]
use lemon_shark::heap::AllocationStats;

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
pub extern "C" fn _start(_: usize, device_tree: usize) -> ! {
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

#[test_case]
fn allocator_init() {
    let mut alloc = FreeListAllocator {
        head: None,
        #[cfg(feature = "stats")]
        stats: AllocationStats::new(),
    };

    unsafe { alloc.init() };
}

fn make_alloc() -> FreeListAllocator {
    let mut alloc = FreeListAllocator {
        head: None,
        #[cfg(feature = "stats")]
        stats: AllocationStats::new(),
    };

    unsafe { alloc.init() };

    alloc
}

#[test_case]
fn allocator_can_allocate() {
    let mut alloc = make_alloc();

    let initial_free_bytes = alloc.free();

    let layout = core::alloc::Layout::new::<[usize; 100]>();

    let align = layout.align();

    let ptr = alloc.alloc(layout);
    let mem = ptr as *mut usize;

    assert_eq!(alloc.free(), initial_free_bytes - layout.size());

    assert!(!mem.is_null());
    assert!((mem as usize) % align == 0);

    unsafe { core::ptr::write_bytes(mem, 1u8, 100) };

    // memory is writable
    for i in 0..100 {
        unsafe { *mem.add(i) = i as usize };
    }

    // memory is readable
    for i in 0..100 {
        unsafe {
            assert_eq!(*mem.add(i), i as usize);
        }
    }

    alloc.dealloc(ptr, layout);

    assert_eq!(alloc.free(), initial_free_bytes);
}

#[test_case]
fn fragmentation_and_merge() {
    let mut alloc = make_alloc();

    // layout to allocate 800 bytes
    let layout = core::alloc::Layout::new::<[usize; 100]>();
    let initial_free = alloc.free();

    assert_eq!(alloc.free_blocks(), 1);

    let left = alloc.alloc(layout);
    let mid = alloc.alloc(layout);
    let right = alloc.alloc(layout);

    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), initial_free - layout.size() * 3);

    // At this point, the allocator should have created 3 blocks of useable memory
    // all before the main block of free memory.

    // This should free the mid block and thus create 2 separate free blocks that
    // can't yet be merged because of `right`.
    alloc.dealloc(mid, layout);

    assert_eq!(alloc.free_blocks(), 2);
    assert_eq!(alloc.free(), initial_free - layout.size() * 2);

    // Deallocating `left` should cause `left` and `mid` to be merged into a single
    // `FreeBlock` and thus the number of free blocks should not increase.
    alloc.dealloc(left, layout);
    assert_eq!(alloc.free_blocks(), 2);
    assert_eq!(alloc.free(), initial_free - layout.size());

    // Deallocating the `right` now should consolidate all the blocks into a single
    // block again and restore the initial state of the allocator.
    alloc.dealloc(right, layout);
    assert_eq!(alloc.free_blocks(), 1);
    assert_eq!(alloc.free(), initial_free);

    alloc.dump_state();
}
