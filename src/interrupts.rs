#![allow(unused)]
use core::arch::asm;

use crate::logln;

/// Enables interrupts globally and enables the timer interrupt
/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.4
pub fn init() {
    unsafe {
        asm!("csrs sie, {}", in(reg) 1 << 5); // STIE
        asm!("csrs sstatus, {}", in(reg) 1 << 1); // SIE
    }

    logln!("Timer interrupt enabled");
}

pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let sstatus: usize;

    unsafe {
        asm!("csrr {}, sstatus", out(reg) sstatus);
        asm!("csrci sstatus, 0x2");
    }

    let result = f();

    if (sstatus & 0x2) != 0 {
        unsafe {
            asm!("csrsi sstatus, 0x2");
        }
    }

    result
}
