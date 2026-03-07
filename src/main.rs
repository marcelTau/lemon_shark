#![no_std]
#![no_main]

use core::arch::global_asm;
use lemon_shark::{
    ALLOCATOR, device_tree,
    filesystem::{self, KernelBlockDevice},
    interrupts, println, shell, trap_handler, virtio2,
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
    lemon_shark::klog::init();
    trap_handler::init();
    interrupts::init();
    unsafe { ALLOCATOR.init() };
    virtio2::init_console();

    device_tree::init(device_table_addr);

    let virtio_device = virtio2::make_device();
    filesystem::init_with_device(KernelBlockDevice::VirtIO(virtio_device));

    print_welcome();

    println!("Hello normal");
    log::debug!("Hello debug");

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
