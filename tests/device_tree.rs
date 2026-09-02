#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(dt_test_runner)]
#![reexport_test_harness_main = "test_main"]

mod common;

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
    let layout = common::init_kernel_layout();

    trap_handler::init(layout);
    unsafe { ALLOCATOR.init(layout) };
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

#[test_case]
fn virtio_mmio_resources_preserve_device_boundaries() {
    let fdt_addr = unsafe { FDT_ADDR };
    let regions = device_tree::virtio_mmio_regions(fdt_addr).unwrap();

    assert_eq!(regions.len(), 8, "unexpected VirtIO resources: {regions:?}");
    assert!(
        regions
            .iter()
            .all(|region| region.size() == 0x1000 && region.start() != 0x10000000),
        "VirtIO resources included a non-VirtIO page or lost their exact sizes: {regions:?}",
    );
}

#[test_case]
fn kernel_mmio_regions_cover_uart_and_virtio_devices() {
    let fdt_addr = unsafe { FDT_ADDR };
    let regions = device_tree::mmio_regions(fdt_addr).unwrap();

    assert_eq!(regions.len(), 2, "unexpected MMIO ranges: {regions:?}");
    assert_eq!(regions[0].start(), 0x0c000000);
    assert_eq!(regions[0].end(), 0x0c600000);
    assert_eq!(regions[1].start(), 0x10000000);
    assert_eq!(regions[1].end(), 0x10009000);
}

#[test_case]
fn console_interrupt_devices_are_discovered() {
    let fdt_addr = unsafe { FDT_ADDR };
    let (uart, plic) = device_tree::interrupt_devices(fdt_addr).unwrap();

    assert_eq!(uart.mmio_region.start(), 0x10000000);
    assert_eq!(uart.mmio_region.size(), 0x100);
    assert_eq!(uart.interrupt_id, 10);

    assert_eq!(plic.mmio_region.start(), 0x0c000000);
    assert_eq!(plic.mmio_region.size(), 0x600000);
    assert_eq!(plic.num_sources, 95);
    assert_eq!(plic.supervisor_contexts.len(), 1);
    assert_eq!(plic.supervisor_contexts[0].hart_id, 0);
    assert_eq!(plic.supervisor_contexts[0].context_index, 1);
}
