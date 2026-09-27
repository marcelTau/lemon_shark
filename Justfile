qemu := "qemu-system-riscv64"

# Replace `-machine virt` with `-machine virt,dumpdtb=qemu.dtb` to dump the
# device tree. Use `dtc` to decompile it. The `generate_device_tree` recipe
# below performs both steps.
#
# UART (`-serial`) -> stdio: interactive shell, clean output only.
# virtio-serial -> kernel.log: kernel debug logs, separate from the console.
#   Use `tail -f kernel.log` in a second terminal to watch logs live.

# Build and run the kernel (the default recipe).
all: run

# Build and run the kernel with QEMU's GDB stub enabled and the CPU paused.
debug: (run "-s" "-S")

# Generate a decompiled QEMU device tree at qemu.dts.
generate_device_tree:
	@{{qemu}} \
		-machine virt,dumpdtb=qemu.dtb \
		-bios default \
		-cpu rv64 \
		-display none
	@dtc -I dtb -O dts -o qemu.dts qemu.dtb
	@echo 'Generated `qemu.dts`.'
	@rm qemu.dtb

# Build a filesystem image, forwarding all arguments to mkfs.
[positional-arguments]
fs *args:
	#!/usr/bin/env bash
	set -euo pipefail
	cargo run -p mkfs --target x86_64-unknown-linux-gnu -- "$@"

# Build and run the kernel, optionally forwarding extra arguments to QEMU.
[positional-arguments]
run *extra_args:
	#!/usr/bin/env bash
	set -euo pipefail
	qemu=(
		"{{qemu}}"
		-machine virt
		-bios default
		-cpu rv64
		-display none
		-chardev stdio,id=con,signal=on
		-serial chardev:con
		-drive file=root.img,if=none,format=raw,id=hd0
		-device virtio-blk-device,drive=hd0
		-chardev file,id=log,path=kernel.log
		-device virtio-serial-device
		-device virtconsole,chardev=log
		-kernel ./target/riscv64gc-unknown-none-elf/debug/lemon-shark
	)
	cargo build
	printf 'Running:'
	printf ' %q' "${qemu[@]}" "$@"
	printf '\n'
	truncate -s 0 kernel.log
	exec "${qemu[@]}" "$@"

# Type-check the workspace.
check:
	@cargo check

# Run all workspace and host-target tests.
test:
	@cargo test
	@cargo test -p allocator --target x86_64-unknown-linux-gnu
	@cargo test -p filesystem --target x86_64-unknown-linux-gnu
	@cargo test -p virtual_memory --target x86_64-unknown-linux-gnu
	@cargo test -p bitmap --target x86_64-unknown-linux-gnu
