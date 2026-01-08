use core::arch::asm;

/// Helper function that creates a timer for 1 second using the frequency read
/// from the device tree.
pub fn new_time(secs: usize) {
    // TODO(mt): create a mapping for functions in sbi module.
    const TIME_FN: usize = 0x54494D45;
    let freq = crate::device_tree::timer_frequency();

    let time = freq * secs;

    unsafe {
        asm!(
            "rdtime t0",
            "add a0, t0, t1",
            "li a6, 0x0",
            "ecall",
            in("t1") time,
            in("a7") TIME_FN,
            out("t0") _,
            out("a0") _,
            out("a6") _,
        )
    }
}

pub fn uptime() -> usize {
    let time: usize;

    unsafe { asm!("rdtime {}", out(reg) time) };

    let freq = crate::device_tree::timer_frequency();

    time / freq
}
