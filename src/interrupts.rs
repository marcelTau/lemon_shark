use core::arch::asm;

/// Enables interrupts globally and enables the timer interrupt
/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.4
pub fn init() {
    unsafe {
        asm!("csrs sie, {}", in(reg) 1 << 5); // STIE
        asm!("csrs sstatus, {}", in(reg) 1 << 1); // SIE
    }

    log::info!("Timer interrupt enabled");
}

/// Runs `f` with supervisor interrupts disabled on the current CPU, then
/// restores the previous interrupt-enable state.
///
/// # Critical-section requirements
///
/// The closure runs in interrupt context from a scheduling and locking
/// perspective. It must therefore:
///
/// - finish quickly and have a bounded execution time;
/// - not sleep, yield, or wait for work that must be completed by an interrupt;
/// - not acquire locks that can be held by interrupted code, unless those locks
///   are explicitly designed for interrupt-disabled use;
/// - not allocate or deallocate when called from the allocator or while an
///   allocator lock is held, because that would recursively enter the allocator;
/// - avoid logging, device I/O, and arbitrary callbacks unless they are known to
///   be allocation-free, non-blocking, and safe with interrupts disabled.
///
/// In particular, allocator critical sections should contain only bounded
/// allocator bookkeeping. Diagnostics should be recorded without allocation and
/// emitted after leaving the critical section.
///
/// Nested calls are supported: each call restores the interrupt-enable state it
/// observed on entry.
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
