use spin::Once;

use crate::riscv;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Stores the hardware timebase frequency locally so timer operations do not
/// depend on the device-tree subsystem after initialization.
///
/// After initialization, `spin::Once::get` performs an atomic state load; it
/// does not acquire a spin lock.
static TIMER_FREQUENCY: Once<usize> = Once::new();

pub fn init(freq: usize) {
    assert_ne!(freq, 0, "Timer frequency must be non-zero");

    TIMER_FREQUENCY.call_once(|| freq);
}

fn freq() -> usize {
    *TIMER_FREQUENCY
        .get()
        .expect("Hardware Timer accessed before `timer::init()`")
}

/// Number of supervisor timer interrupts handled since boot.
///
/// `Relaxed` ordering is sufficient because this counter does not publish any
/// other memory. It is an observation point for diagnostics and tests, not a
/// synchronization primitive.
static INTERRUPT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Program a timer deadline `secs` seconds from the current hardware time.
///
/// [`init`] must be called before this function.
pub fn new_time(secs: usize) {
    const TIME_FN: usize = 0x54494D45;
    let time = freq() * secs;

    let error: usize;

    unsafe {
        core::arch::asm!(
            "rdtime t0",
            "add a0, t0, t1",
            "li a6, 0x0",
            "ecall",
            in("t1") time,
            in("a7") TIME_FN,
            out("t0") _,
            out("a6") _,
            lateout("a0") error,
            lateout("a1") _,
        )
    }

    // The [spec](https://docs.riscv.org/reference/sbi/v3.0/ext-time.html) defines `error` is
    // always 0.
    debug_assert_eq!(error, 0);
}

/// Handle one supervisor timer interrupt and program the next deadline.
///
/// Keeping both operations here gives tests one observable event while the
/// trap handler remains responsible only for dispatching the trap cause.
pub(crate) fn handle_interrupt() {
    INTERRUPT_COUNT.fetch_add(1, Ordering::Relaxed);
    new_time(1);
}

pub fn interrupt_count() -> usize {
    INTERRUPT_COUNT.load(Ordering::Relaxed)
}

pub fn uptime() -> usize {
    riscv::asm::rdtime() / freq()
}

pub fn uptime_ms() -> usize {
    let freq = freq() / 1000;
    riscv::asm::rdtime() / freq
}
