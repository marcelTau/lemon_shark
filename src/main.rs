#![no_std]
#![no_main]

use core::arch::global_asm;
use lemon_shark::{
    device_tree,
    filesystem::{self, KernelBlockDevice},
    interrupts,
    kernel_layout::KernelLayout,
    logo, page_frame_allocator, page_table, println, riscv, shell, timer, trap_handler, virtio2,
    ALLOCATOR,
};

// This is the section that we mapped first in the linker script `linker.ld`
// .section .text.boot
//
// Export the `_boot` symbol, now referenced in the linker script
global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    // "   la gp, __global_pointer$",  // Initialize global pointer for
    // accessing static variables TODO(mt): do we really need this?
    "   la sp, _stack_top",
    "   call _start",
);

#[unsafe(no_mangle)]
extern "C" fn _start(_: usize, device_table_addr: usize) -> ! {
    let kernel_layout = unsafe { KernelLayout::from_labels() };

    unsafe { ALLOCATOR.init(kernel_layout) };

    lemon_shark::klog::init();
    log::warn!("========== Kernel started ==========");
    log::info!("{kernel_layout:#x?}");

    riscv::asm::sie::probe();

    virtio2::init_console();
    trap_handler::init(kernel_layout);
    device_tree::init(device_table_addr).expect("failed to initialize device tree");
    timer::init(device_tree::timer_frequency());

    page_frame_allocator::init(kernel_layout);
    page_table::init(kernel_layout);

    let virtio_device = virtio2::make_device();
    filesystem::init_with_device(KernelBlockDevice::VirtIO(virtio_device));

    println!("{}", logo::SHARK);
    println!("Welcome to LemonShark v0.0.1");

    log::warn!(
        "========== Boot completed {}ms ==========",
        timer::uptime_ms()
    );

    // Program the first deadline before making timer interrupts observable.
    // Global interrupts are enabled only after boot initialization is complete.
    timer::new_time(1);
    interrupts::init();

    shell::shell()
}
