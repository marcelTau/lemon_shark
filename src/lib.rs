#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::new_without_default)]
#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

pub mod allocator;
pub mod bitmap;
pub mod bytereader;
pub mod device_tree;
pub mod filesystem;
pub mod interrupts;
pub mod log;
pub mod println;
pub mod ramdisk;
pub mod shell;
pub mod timer;
pub mod trap_handler;
pub mod virtio;
pub mod virtio2;

use crate::allocator::LockedAllocator;

use core::panic::PanicInfo;

/// This panic handler is used by the kernel and the tests.
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    let msg = info.message();
    if let Some(loc) = info.location() {
        // #[cfg(not(feature = "logging"))]
        {
            println!("[PANIC] oh shit ... {msg} in {}:{}", loc.file(), loc.line());
        }
        logln!("[PANIC] oh shit ... {msg} in {}:{}", loc.file(), loc.line());
    } else {
        logln!("[PANIC] oh shit ... {msg}");

        // #[cfg(not(feature = "logging"))]
        {
            println!("[PANIC] oh shit ... {msg}");
        }
    }

    #[cfg(test)]
    lemon_shark::exit_qemu(1);

    loop {}
}

#[global_allocator]
pub static ALLOCATOR: LockedAllocator = LockedAllocator::new();

pub fn dump_memory() {
    unsafe { ALLOCATOR.dump_state() };
}

pub fn test_runner(tests: &[&dyn Testable]) {
    println!("\nRunning {} tests...\n", tests.len());
    for test in tests {
        test.run();
    }
    println!("\n\nAll tests passed!\n");
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
        // Align [ok] at column 100 for consistent formatting
        let test_name = core::any::type_name::<T>();
        print!("{:<95}... ", test_name);
        self();
        println!("[ok]");
    }
}

/// Exit QEMU using SBI shutdown call
pub fn exit_qemu(exit_code: u32) {
    use core::arch::asm;

    // https://lists.riscv.org/g/tech-brs/attachment/361/0/riscv-sbi.pdf
    // chapter 10
    const SBI_EXT_SYSTEM_RESET: usize = 0x53525354;
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
