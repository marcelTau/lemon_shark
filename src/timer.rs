use crate::{device_tree, riscv};

/// Helper function that creates a timer for `secs` second using the frequency read from the device
/// tree.
///
/// NOTE: Since this is reading values from the device tree it has to be initialized, even in
/// tests.
pub fn new_time(secs: usize) {
    const TIME_FN: usize = 0x54494D45;

    let freq = device_tree::timer_frequency();
    let time = freq * secs;

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

pub fn uptime() -> usize {
    riscv::asm::rdtime() / device_tree::timer_frequency()
}

pub fn uptime_ms() -> usize {
    let freq = device_tree::timer_frequency() / 1000;
    riscv::asm::rdtime() / freq
}
