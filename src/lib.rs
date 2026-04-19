#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::new_without_default)]
#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

pub mod allocator;
pub mod device_tree;
pub mod filesystem;
pub mod interrupts;
pub mod klog;
pub mod page_frame_allocator;
pub mod page_table;
pub mod println;
pub mod ramdisk;
pub mod scheduler;
pub mod shell;
pub mod timer;
pub mod trap_handler;
pub mod virtio;
pub mod virtio2;

use crate::allocator::LockedAllocator;

use core::panic::PanicInfo;

/// This abstraction reads the addresses of all used linker-inserted labels that the kernel needs
/// for orientation once in the beginning instead of having unsafe functions that assume that those
/// labels exist down the line. This also makes testing a lot easier.
#[derive(Copy, Clone, Debug)]
pub struct KernelLayout {
    kernel_start: usize,
    kernel_end: usize,
    heap_start: usize,
    heap_end: usize,
    trap_stack_top: usize,
}

impl KernelLayout {
    /// SAFETY: This function requires a specific set of valid labels to be set in place by the
    /// linker.
    pub unsafe fn from_lables() -> Self {
        unsafe extern "C" {
            static _kernel_end: u8;
            static _heap_start: u8;
            static _heap_end: u8;
            static _trap_stack_top: u8;
        }

        unsafe {
            Self {
                kernel_start: 0x80200000, // took from linker.ld
                kernel_end: &_kernel_end as *const u8 as usize,
                heap_start: &_heap_start as *const u8 as usize,
                heap_end: &_heap_end as *const u8 as usize,
                trap_stack_top: &_trap_stack_top as *const u8 as usize,
            }
        }
    }
}

/// This panic handler is used by the kernel and the tests.
#[panic_handler]
fn panic_handler(info: &PanicInfo) -> ! {
    let msg = info.message();
    if let Some(loc) = info.location() {
        println!("[PANIC] oh shit ... {msg} in {}:{}", loc.file(), loc.line());
        log::error!("oh shit ... {msg} in {}:{}", loc.file(), loc.line());
    } else {
        println!("[PANIC] oh shit ... {msg}");
        log::error!("oh shit ... {msg}");
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
