Learning OS development by building a small RISC-V kernel in Rust.

## Prerequisites

1. Install the RISC-V toolchain.

```bash
rustup target add riscv64gc-unknown-none-elf
```

2. Install `qemu-system-riscv64` and `gdb-multiarch` / `rust-gdb`.

## Build a filesystem image

The files and directories under `rootfs/` become the root of the LemonFS image.
Build the image explicitly after changing that directory:

```bash
cargo run -p mkfs --target x86_64-unknown-linux-gnu --
```

This creates `lemonfs.img`, which QEMU exposes to the kernel as its VirtIO block
device. Running the kernel does not rebuild the image, so changes made from the
kernel remain until `mkfs` is run again. `make fs` is a convenience alias for
the command above.

The defaults can be overridden with named options:

```bash
cargo run -p mkfs --target x86_64-unknown-linux-gnu -- \
    --source rootfs \
    --output lemonfs.img \
    --blocks 32768
```

Run the command with `--help` for the complete interface. LemonFS currently
limits each UTF-8 path component to 24 bytes and each file to 8192 bytes.
Regular files, directories, empty directories, and hidden entries are imported.
Symlinks and other special host entries are skipped with a warning. Host
permissions, ownership, and timestamps are not represented by LemonFS.

## Run the kernel

After creating the image, boot the kernel with:

```bash
make run
```

The shell's `tree`, `ls`, and `cat` commands can be used to inspect imported
content. For example, the repository's sample file is available as
`/hello.txt`.
