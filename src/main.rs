#![no_std]
#![no_main]

use core::arch::global_asm;
use lemon_shark::{
    ALLOCATOR, device_tree,
    filesystem::{self, KernelBlockDevice},
    interrupts, page_frame_allocator, println, shell, timer, trap_handler, virtio2,
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

#[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn _start(_: usize, device_table_addr: usize) -> ! {
    unsafe { ALLOCATOR.init() };

    lemon_shark::klog::init();
    log::warn!("========== Kernel started ==========");

    virtio2::init_console();
    trap_handler::init();
    interrupts::init();

    device_tree::init(device_table_addr);

    page_frame_allocator::init();

    crate::timer::new_time(1);

    let virtio_device = virtio2::make_device();
    filesystem::init_with_device(KernelBlockDevice::VirtIO(virtio_device));

    print_welcome();

    log::warn!(
        "========== Boot completed {}ms ==========",
        timer::uptime_ms()
    );

    shell::shell()
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
