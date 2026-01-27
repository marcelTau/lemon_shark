#![no_std]
#![no_main]

use core::arch::{asm, global_asm};
use lemon_shark::{ALLOCATOR, device_tree, filesystem, interrupts, println, shell, trap_handler};

// ; This is the section that we mapped first in the linker script `linker.ld`
// .section .text.boot
//
// ; Export the `_boot` symbol, now referenced in the linker script
// .global _boot
//
// ; Define the `_boot` symbol
// _boot:
//     ; set the stack pointer to the top of the stack
//     ; `_stack_top` is set by the linker
//     la sp, _stack_top
//     ; call into the kernel
//     call _start
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
    print_welcome();

    // This is defined in the linker script and reserves space for a trap
    // stack.
    unsafe extern "C" {
        static _trap_stack_top: u8;
        static _heap_top: u8;
    }

    // Set the `sscratch` register to a 'known good' stack that the
    // `trap_handler` can use.
    unsafe {
        let trap_stack = &_trap_stack_top as *const u8 as usize;
        asm!("csrw sscratch, {}", in(reg) trap_stack);
    }

    trap_handler::init();
    interrupts::init();

    unsafe { ALLOCATOR.init() };

    device_tree::init(device_table_addr);

    filesystem::init();
    filesystem::init();

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
