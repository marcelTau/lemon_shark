#![no_std]
#![no_main]

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

static TRAP: AtomicBool = AtomicBool::new(false);

use core::arch::naked_asm;
use core::sync::atomic::Ordering;
use core::sync::atomic::AtomicBool;

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub extern "C" fn trap_handler() -> ! {
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

#[unsafe(no_mangle)]
extern "C" fn trap_handler_rust() {
    // let scause: usize;
    // unsafe { asm!("csrr {}, sstatus", out(reg) scause) };
    // unsafe { asm!("sret", options(noreturn)) };

    let uart = 0x10000000 as *mut u8;
    unsafe { 
        uart.write_volatile(b'T');
        uart.write_volatile(b'R');
        uart.write_volatile(b'A');
        uart.write_volatile(b'P');
        uart.write_volatile(b'\n');
    };
    let scause: usize;
    unsafe { asm!("csrr {}, scause", out(reg) scause) };

    if (scause as isize) < 0 && (scause & 0xFF) == 5 {
        // unsafe { uart.write_volatile(b'-') };
        new_time();

        // Setting the TRAP to true if it's currently false, not changing it if it's true.
        // Don't care about the result here.
        let _ = TRAP.compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed);
    } 
}

fn new_time() {
    unsafe {
        asm!(
            "rdtime t0",
            "li t1, 10000000",
            "add a0, t0, t1",
            "li a7, 0x54494D45",
            "li a6, 0x0",
            "ecall",
        )
    }
}

/// Enables interrupts globally and enables the timer interrupt
fn enable_interrupts() {
    // unsafe {
    //     asm!("csrs sie, {}", in(reg) 1 << 5); // STIE
    //     asm!("csrs sstatus, {}", in(reg) 1 << 1); // SIE
    // }
    let sie: usize;
    let sstatus: usize;

    unsafe {
        asm!("csrr {}, sie", out(reg) sie);
        asm!("csrr {}, sstatus", out(reg) sstatus);

        asm!("csrw sie, {}", in(reg) sie | (1 << 5));
        asm!("csrw sstatus, {}", in(reg) sstatus | (1 << 1));
    }
}

fn register_trap_handler() {
    let trap_handler_addr = (trap_handler as usize) & !0b11;

    unsafe {
        asm!("csrw stvec, {}", in(reg) trap_handler_addr);
    }
}

fn simple_print() {
}

#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    let uart = 0x10000000 as *mut u8;
    unsafe { uart.write_volatile(b'S') };
    unsafe { uart.write_volatile(b'\n') };


    unsafe extern "C" {
        static _trap_stack_top: u8;
    }

    // Set the `sscratch` register to a 'known good' stack that the `trap_handler can use`.
    unsafe {
        let trap_stack = &_trap_stack_top as *const u8 as usize;
        asm!("csrw sscratch, {}", in(reg) trap_stack);
    }

    register_trap_handler();
    enable_interrupts();
    new_time();

    loop {
        if TRAP.compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            unsafe { uart.write_volatile(b't') };
        } else {
            // unsafe { uart.write_volatile(b'-') };
        }
    }
}

#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    let uart = 0x10000000 as *mut u8;
    unsafe {
        uart.write_volatile(b'P');
    }
    loop {}
}
