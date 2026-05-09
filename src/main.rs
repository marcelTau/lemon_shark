#![no_std]
#![no_main]

use core::arch::global_asm;
use lemon_shark::{
    device_tree,
    filesystem::{self, KernelBlockDevice},
    interrupts, page_frame_allocator, page_table, println, scheduler, shell, timer, trap_handler,
    virtio2, KernelLayout, ALLOCATOR,
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

// #[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn _start(_: usize, device_table_addr: usize) -> ! {
    let kernel_layout = unsafe { KernelLayout::from_lables() };

    unsafe { ALLOCATOR.init(kernel_layout) };

    lemon_shark::klog::init();
    log::warn!("========== Kernel started ==========");

    virtio2::init_console();
    trap_handler::init(kernel_layout);
    interrupts::init();

    device_tree::init(device_table_addr);

    page_frame_allocator::init(kernel_layout);
    page_table::init(kernel_layout);

    crate::timer::new_time_ms(10);

    let virtio_device = virtio2::make_device();
    filesystem::init_with_device(KernelBlockDevice::VirtIO(virtio_device));

    print_welcome();

    log::warn!(
        "========== Boot completed {}ms ==========",
        timer::uptime_ms()
    );

    scheduler::init_with_shell(shell::shell);
    scheduler::spawn(second_process);
    scheduler::spawn(third_process);
    scheduler::start()
}

fn second_process() {
    let mut i = 0u64;

    loop {
        log::info!("[process 2] tick {i}");
        i += 1;
        // busy-wait so we don't spam too fast
        for _ in 0..5_000_000 {
            core::hint::spin_loop();
        }
    }
}

fn third_process() {
    let mut i = 0u64;

    loop {
        log::info!("[process 3] tick {i}");
        i += 1;
        // busy-wait so we don't spam too fast
        for _ in 0..5_000_000 {
            core::hint::spin_loop();
        }
    }
}

fn print_welcome() {
    let shark = r#"
                     .#@@@@*....
                    @@-....-@@@@@@:.
                    .@@.        ..@@@@.     ........
                     .@@-.          .@@@@@@@@@@@@@@@@@@@@@@...
                      .@@.         =+......          .....-@@@@@@..
                       .@@.                                   ..:@@@@@..
                       .@@@@..                                     ...@@@@.
                     .@@@..                                            ..@@.
                   .@@@.                      .@@@@.                    .@@.
                 ..@@..                      .@@@.+@.             .     .@@.
                .@@+.                        .@@@@@@.           =@.    .@@.
    ..         .@@..                          .@@@@.                   %@..
  .@@@@@%.    .@@.              ..                                    @@=.
   .@@..@@@. :@@.             @.@.                                  .@@..
    .@@. .=@@@#.            ..@.@.        .@@@..                  .@@@.
     *@*.  +..             .@.@.=@.       ...@.@-@@@@@%+=-+*@@@@@@@@..
      @@.                  .@.-@..@.         .%@....@*.@.@.+@@.@@..
      .@@.                  .@..+. ..          .#@@.@@..:. ...@@:.
      .@@.  ..                                    ..@@@@-@@-@@@@.
      =@-.  .@@@..     .@.                             ......@@@.
     .@@. .@@%@@@.:@@@@+.         @.                      =@@@..
     *@:@@@@.. .:@@@..@.        .@-.                  .@@@@..
     .@@:..       .-@@.        .@-.              ..@@@@@..
                  .@@..      .@@:.........:@@@@@@@%@@.
                 .@@.      .@@@:%@@@@@@@@@@@@...  .@@.
               .@@@.    .*@@@..            .:@@@. @@*.
               @@:..%@@@@@..                  .@@@@..
               .-@@@=...
        "#;

    println!("{shark}");
    println!("Welcome to LemonShark v0.0.1");
}
