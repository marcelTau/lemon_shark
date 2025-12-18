# Booting Risc-V Kernel from scratch

## Setting up Rust to build a RISC-V 64 target for QEMU

The default target for Rust binaries is `x86-64` hence we can't use it for our
RISC-V kernel. To change the build target we need to insert this code into a
`.cargo/config.toml` file in the root directory.

```toml
[build]
target = "riscv64gc-unknown-none-elf"
```

This now let's us build the executable for RISC-V.

## Build script for QEMU

To make our lives easier we can create a simple shell script that runs the QEMU
command for us.

```bash
qemu-system-riscv64 \
    -machine virt \
    -bios default \
    -kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark \
    -nographic
```

* `-machine virt` tells QEMU to use a generic environment instead of modeling
  some real hardware. This is ideal for leanring to build a kernel. 
* `-bios default` Uses the default for RISC-V and uses OPEN-SBI the firmware
  layer that runs in the highest privilege mode and ultimately loads our
  kernel.
* `-kernel` tells QEMU which binary to load.
* `-nographic` allows us to run QEMU without a GUI.

## The boot process

1. Setup the stack
2. Setup a trap handler
...
N. Enable interrupts


Let's begin by setting up the stack. Currently we don't have a way of
allocating memory so we need to create our own stack. For that we need the help
of the linker.

We can write a custom [linker script](linker.ld) that reserves space in the
binary layout that we can then use as the stack for our kernel to bootstrap.

The linker script first defines the entry point (\_boot) and then lays out the
ELF sections of our binary.

We want to start with `.text.boot` as this will be the section we define ourselves
to setup the stack pointer correctly later on.

Then we follow the conventional order of the sections:
1. `.text`
2. `.rodata`
3. `.data`
4. `.bss`

The `.bss` is where we will make some space for our stack. The `.bss` segment
hold uninitialized memory such as global variables.

Inside of the linker script we add 16k to the end of the `.bss` section and
store the address in a special `_stack_top` symbol.

In our rust code now, we have to setup the stack pointer using `global_asm!`.
Here we can now access the `_stack_top` symbol and also have access to the
registers.

```rust
global_asm!(
    ".section .text.boot",
    ".global _boot",
    "_boot:",
    "   la sp, _stack_top",
    "   call _start",
);
```

First we define that we are now writing code that belong into the section
`.text.boot`. This is the section that we have defined to be the first one in
the linker script.

Then we create a new label `_boot` which is the entry point defined in the
linker script.

Then inside of the label, we load the value of our special `_stack_top` symbol
into `sp` (the stack pointer register) and yield execution to `_start`.


## Executing Rust Code

Since we are building an OS, we can't rely on rust's standard library to
provide us with all the helpful things that under the hood are coming from the
OS. This means we need to run in no-std mode. Adding `#![no_std]` to the top of
`main.rs`.

Since we don't have the rust runtime executing our code here, we can't just define
a `fn main()` function and need to use the `#![no_main]` attribute.

In the inline assembly above, we yield execution to the `_start` symbol which
is usually the entry point for most programs. Now we need to define this
ourselves.

To do that, we need to create a function called `_start` in rust. We also need to
annotate this function with `#[unsafe(no_mangle)]` to avoid name mangling as we need
this symbol to be called exactly `_start`. 

Also we need to mark this function as `extern "C"` since it's being called from
RISC-V assembly and RISC-V specifies the C calling convention.

## Panic handler

To create a rust program, we need a panic handler. Usually this is done by the
standard library so we need to define one for our OS.

The function signature is `fn panic_handler(info: &core::panic::PanicInfo) -> !;`.
This means the handler is not allowed to return. In order to do that we can
just `loop {}` for now until we figured out how to do that without spinning on
the CPU.

## Trap handler

We need to define a trap handler that catches interrupts and exceptions. On
RISC-V the trap handler is configured via a special register. On a trap, the
system jumps to the address defined in the `stvec` register.

The system then sets some other registers to capture information about the trap
such as the address and the cause in `sepc` and `scause` respectively.

## Writing to the console

In order to write something to the console we can use the UART (serial console).
On QEMU's virt machines, there is a memory mapped UART at address 0x10000000.
Writing a byte to this address outputs it to the console.

# Current state

Now we have a basic kernel that can catch exceptions and output some
information to the screen.

Yay :)
