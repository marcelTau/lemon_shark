use core::arch::asm;

use crate::riscv;

/// Enables interrupts globally and enables the timer & external interrupts
/// https://people.eecs.berkeley.edu/~krste/papers/riscv-privileged-v1.9.1.pdf
/// Section 4.1.4
pub fn init() {
    riscv::asm::sie::enable_timer_interrupt();
    riscv::asm::sie::enable_external_interrupt();

    // Enables the `SIE` bit in `sstatus`
    unsafe { asm!("csrsi sstatus, 0x2", options(nostack)) };

    log::info!("Initialized");
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
    let sstatus = read_sstatus_and_disable_sie();
    let result = f();
    restore_sstatus(sstatus);

    result
}

/// Returns the current state of the `sstatus` register and atomically unsets the `sstatus.SIE` bit
/// after reading it out in the live register.
#[inline(always)]
fn read_sstatus_and_disable_sie() -> usize {
    let sstatus: usize;

    unsafe {
        core::arch::asm!(
            "csrrci {}, sstatus, 0x2",
            out(reg) sstatus,
            options(nostack)
        );
    }

    sstatus
}

/// The idea of this function is to block until `poll` returns `Some(T)`. The blocking is achieved
/// by waiting for interrupts using the `wfi` instruction.
///
/// We disable global interrupts in the `sstatus` register but keep the individual interrupts in the
/// `sie` register enabled. This is important because `wfi` will still wake even if global
/// interrupts are disabled but the interrupts in the `sie` register are still enabled.
///
/// When global interrupts are disabled the trap-handler will not be called.
///
/// We use this to:
/// 1. Disable global interrupts.
/// 2. Call `poll`.
/// 3. When `poll` returned, the lock it held is released.
/// 4. If it succeeded, restore `sstatus` and return.
/// 5. If not, wait for an interrupt while `sstatus.SIE` is disabled, i.e the handler is not called.
/// 6. Once there was an interrupt, enable `sstatus.SIE` to allow the handler to run and the
///    trap-handler will populate the `RX_RING` if there was an interrupt for the UART.
/// 7. loop - `poll` will now return `Some(T)` as the `RX_RING` was filled in the interrupt.
///
/// # Panics
///
/// Panics if `sstatus.SIE` is not set. In particular, do not call this from an interrupt handler or
/// inside a `without_interrupts` block.
pub(crate) fn wait_until<F, T>(mut poll: F) -> T
where
    F: FnMut() -> Option<T>,
{
    let mut sstatus = read_sstatus_and_disable_sie();
    assert!(
        sstatus & 0x2 != 0,
        "`wait_until` requires interrupts enabled on entry"
    );

    loop {
        if let Some(byte) = poll() {
            restore_sstatus(sstatus);
            return byte;
        }

        unsafe { core::arch::asm!("wfi", options(nomem, nostack)) }

        restore_sstatus(sstatus);
        sstatus = read_sstatus_and_disable_sie();
    }
}

/// Enables `sstatus.SIE` if `prev` had it enabled.
#[inline(always)]
fn restore_sstatus(prev: usize) {
    if (prev & 0x2) != 0 {
        unsafe { asm!("csrsi sstatus, 0x2", options(nostack)) };
    }
}
