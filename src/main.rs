#![no_std]
#![no_main]

mod log;
mod trap_handler;

use core::{
    arch::{asm, global_asm},
    panic::PanicInfo,
};

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

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    log!("Hello from Lemon Shark: v0.0.{}\n", 1);

    trap_handler::init();

    unsafe {
        asm!("unimp");
    }

    log!("Bye :^)\n");
    loop {}
}

#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    log!("Oh shit! {info:?}");
    loop {}
}
