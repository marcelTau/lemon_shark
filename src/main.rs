#![no_std]
#![no_main]

use core::{arch::global_asm, panic::PanicInfo};

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
    "   call _start",
);

fn write_char_to_uart(c: char) {
    /// The UART is a hardware device which QEMU reads from and displays in the terminal.
    const UART_ADDRESS: usize = 0x10000000;

    unsafe {
        let uart = UART_ADDRESS as *mut u8;
        uart.write_volatile(c as u8);
    }
}

fn write_string_to_uart(s: &str) {
    for c in s.chars() {
        write_char_to_uart(c);
    }
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    write_string_to_uart("Hello World!");

    loop {}
}

#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    loop {}
}
