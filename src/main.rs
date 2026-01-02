#![no_std]
#![no_main]
// #![feature(custom_test_frameworks)]
// #![test_runner(crate::test_runner)]
// #![reexport_test_harness_main = "test_main"]

use core::{
    arch::{asm, global_asm},
    panic::PanicInfo,
};

use core::sync::atomic::Ordering;
use lemon_shark::{heap, interrupts, log, timer, trap_handler};
use trap_handler::TRAP;
use heap::LockedAllocator;
extern crate alloc;
use alloc::vec::Vec;

#[global_allocator]
static ALLOCATOR: LockedAllocator = LockedAllocator::new();

// ; This is the section that we mapped first in the linker script `linker.ld`
// .section .text.boot
//
// ; Export the `_boot` symbol, now referenced in the linker script
// .global _boot
//
// ; Define the `_boot` symbol
// _boot:
//     ; set the stack pointer to the top of the stack
//     ; `_stack_top` is set by the linker
//     la sp, _stack_top
//     ; call into the kernel
//     call _start
global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    // "   la gp, __global_pointer$",  // Initialize global pointer for accessing static variables TODO(mt): do we really need this?
    "   la sp, _stack_top",
    "   call _start", // can use `tail` here because _start never returns hence we don't need to
                      // store the return address
);

#[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn _start(_: usize, device_table_addr: usize) -> ! {
    // This is defined in the linker script and reserves space for a trap stack.
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
    timer::init(device_table_addr);
    timer::new_time();

    #[cfg(test)]
    {
        // test_main();
         loop {}
    }

    unsafe { ALLOCATOR.init() };

    let mut v = Vec::new();

    let mut n = 0;

    loop {
        if TRAP
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            log!(".");

            v.push(n);
            n += 1;
        }
    }
}
