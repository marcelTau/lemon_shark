#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

pub mod heap;
pub mod interrupts;
pub mod log;
pub mod timer;
pub mod trap_handler;

use core::panic::PanicInfo;

/// This panic handler is used by the kernel and the tests.
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    let msg = info.message().as_str().unwrap_or("<no message>");

    if let Some(loc) = info.location() {
        log!("[PANIC] oh shit ... {msg} in {}:{}", loc.file(), loc.line());
    } else {
        log!("[PANIC] oh shit ... {msg}");
    }

    #[cfg(test)]
    lemon_shark::exit_qemu(1);

    loop {}
}

pub fn test_runner(tests: &[&dyn Testable]) {
    logln!("\nRunning {} tests...\n", tests.len());
    for test in tests {
        test.run();
    }
    logln!("\n\nAll tests passed!\n");
    exit_qemu(0);
}

/// Simple trait which logs the test name and result.
pub trait Testable {
    fn run(&self);
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        log!("{}...\t", core::any::type_name::<T>());
        self();
        log!("[ok]");
    }
}

/// Exit QEMU using SBI shutdown call
fn exit_qemu(exit_code: u32) {
    use core::arch::asm;
    
    // https://lists.riscv.org/g/tech-brs/attachment/361/0/riscv-sbi.pdf
    // chapter 10
    const SBI_EXT_SYSTEM_RESET: usize = 0x53525354;
    const SBI_SYSTEM_RESET_TYPE: usize = 0; // shutdown 
    const SBI_SYSTEM_RESET_REASON_NO_REASON: usize = 0;
    const SBI_SYSTEM_RESET_REASON_SYSTEM_FAILURE: usize = 1;

    let reason = if exit_code == 0 {
        SBI_SYSTEM_RESET_REASON_NO_REASON
    } else {
        SBI_SYSTEM_RESET_REASON_SYSTEM_FAILURE
    };

    unsafe {
        asm!(
            "ecall",
            in("a7") SBI_EXT_SYSTEM_RESET,
            in("a6") 0,
            in("a0") 0,
            in("a1") reason,
        );
    }
}
