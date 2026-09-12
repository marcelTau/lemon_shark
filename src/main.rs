#![no_std]
#![no_main]

use core::{arch::global_asm, time::Duration};
use lemon_shark::{
    ALLOCATOR, device_tree,
    filesystem::{self, KernelBlockDevice},
    interrupts,
    kernel_layout::KernelLayout,
    logo, page_frame_allocator, page_table, plic, println, process, riscv, shell, timer,
    trap_handler, uart, virtio2,
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

static COUNTER: AtomicUsize = AtomicUsize::new(0);

use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;

fn proc1() {
    let mut val = 0;
    loop {
        let res = COUNTER.compare_exchange(val, val + 1, Ordering::SeqCst, Ordering::SeqCst);

        if let Ok(x) = res {
            log::info!("proc1: {x}");
            val += 2;
        }
    }
}

fn proc2() {
    let mut val = 1;

    loop {
        let res = COUNTER.compare_exchange(val, val + 1, Ordering::SeqCst, Ordering::SeqCst);

        if let Ok(x) = res {
            log::info!("proc2: {x}");
            val += 2;
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn _start(_: usize, device_table_addr: usize) -> ! {
    let kernel_layout = unsafe { KernelLayout::from_labels() };

    unsafe { ALLOCATOR.init(kernel_layout) };

    lemon_shark::klog::init();
    log::warn!("========== Kernel started ==========");
    log::info!("{kernel_layout:#x?}");

    device_tree::init(device_table_addr).expect("failed to initialize device tree");
    virtio2::init_console();

    riscv::asm::sie::probe();

    trap_handler::init(kernel_layout);
    plic::init().expect("Could not initialize PLIC");
    timer::init(device_tree::timer_frequency());

    page_frame_allocator::init(kernel_layout);
    page_table::init(kernel_layout);

    let virtio_device = virtio2::make_device();
    filesystem::init_with_device(KernelBlockDevice::VirtIO(virtio_device));

    println!("{}", logo::SHARK);
    println!("Welcome to LemonShark v0.1.0");

    log::warn!(
        "========== Boot completed {}ms ==========",
        timer::uptime_ms()
    );

    uart::init();

    // Program the first deadline before making timer interrupts observable.
    // Global interrupts are enabled only after boot initialization is complete.
    interrupts::init();

    interrupts::without_interrupts(|| {
        timer::new_time(Duration::from_millis(1));
        process::schedule(proc1, "proc1");
        process::schedule(proc2, "proc2");
        process::schedule(shell::shell, "shell");
        process::init();
        process::start()
    })
}
