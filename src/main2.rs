#![no_std]
#![no_main]

mod interrupts;
mod log;
mod sbi;
mod timer;
mod trap_handler;

use core::{
    arch::{asm, global_asm},
    panic::PanicInfo,
    sync::atomic::Ordering,
};

use timer::new_timer_in;
use trap_handler::{trap_handler, TIMER_FIRE};

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
    "   la sp, _stack_top",
    "   call _start", // can use `tail` here because _start never returns hence we don't need to
                      // store the return address
);

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler() {
    naked_asm!(
            // Swap `sp` and `sscratch` atomically
            "csrrw sp, sscratch, sp",

            // Manually allocate stack frame, 16 bytes to stay aligned
            "addi sp, sp, -0x10",

            // store return address on the stack as the `call` will overwrite it.
            "sd ra, 8(sp)",

            // call the rust code
            "call {trap_handler_rust}",

            // set return address again
            "ld ra, 8(sp)",

            // reset stack
            "addi sp, sp, 0x10",

            // swap back the stacks
            "csrrw sp, sscratch, sp",

            // return from trap handler
            "sret",

            trap_handler_rust = sym trap_handler_rust,
     ); 
}

use core::sync::atomic::AtomicBool;

static TRAP: AtomicBool = AtomicBool::new(false);

#[unsafe(no_mangle)]
extern "C" fn trap_handler_rust() {
    TRAP.store(true, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
extern "C" fn _start(hart_id: usize, dtb_addr: usize) -> ! {
    unsafe extern "C" {
        static _trap_stack_top: u8;
    }

    // Set the `sscratch` register to a 'known good' stack that the `trap_handler can use`.
    unsafe {
        let trap_stack = &_trap_stack_top as *const u8 as usize;
        asm!("csrw sscratch, {}", in(reg) trap_stack);
    }

    let trap_handler_addr = trap_handler as usize;

    let stvec: usize;
    unsafe {
        asm!("csrw {}, stvec", out(reg) trap_handler_addr);
    }

    loop {
        if !TRAP.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed).is_ok().is_err(){
            let uart = 0x10000000 as *mut u8;
            uart.write_volatile(b'.');
        }
    }
}

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    unsafe {
        let uart = 0x10000000 as *mut u8;
        uart.write_volatile(b'P');
        uart.write_volatile(b'A');
        uart.write_volatile(b'N');
        uart.write_volatile(b'I');
        uart.write_volatile(b'C');
        uart.write_volatile(b'\n');
    }
    // log!("Oh shit! {info:?}");
    loop {}
}
