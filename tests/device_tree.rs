#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(dt_test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::arch::global_asm;
use lemon_shark::{ALLOCATOR, device_tree, println, trap_handler};

extern crate alloc;

static mut FDT_ADDR: usize = 0;

global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);

pub fn dt_test_runner(tests: &[&dyn lemon_shark::Testable]) {
    println!("\nRunning {} tests...\n", tests.len());
    for test in tests {
        test.run();
    }
    println!("\n\nAll {} tests passed!\n", tests.len());
    lemon_shark::exit_qemu(0);
}

#[unsafe(no_mangle)]
pub extern "C" fn _start(_hartid: usize, fdt_addr: usize) -> ! {
    trap_handler::init();
    unsafe { ALLOCATOR.init() };
    unsafe { FDT_ADDR = fdt_addr };
    test_main();
    loop {}
}

#[test_case]
fn virtio_mmio_device_present() {
    let fdt_addr = unsafe { FDT_ADDR };
    let devices = device_tree::virtio_mmio_devices(fdt_addr);
    assert!(
        !devices.is_empty(),
        "No virtio,mmio devices found in device tree"
    );
    assert!(
        devices.contains(&0x10008000),
        "Expected virtio,mmio device at 0x10008000, found: {:?}",
        devices
    );
}
