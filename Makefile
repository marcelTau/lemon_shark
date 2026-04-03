QEMU := qemu-system-riscv64

# replace machine with dumpdtb=qemu.dtb to dump device tree. Use dtc to decompile it.
#
# UART (serial) -> stdio: interactive shell, clean output only.
# virtio-serial -> kernel.log: kernel debug logs, separate from the console.
#   Use `tail -f kernel.log` in a second terminal to watch logs live.
ARGS := -machine virt \
		-bios default \
		-cpu rv64     \
		-display none \
		-chardev stdio,id=con,signal=off \
		-serial chardev:con \
		-drive file=lemonfs.img,if=none,format=raw,id=hd0 \
		-device virtio-blk-device,drive=hd0 \
		-chardev file,id=log,path=kernel.log \
		-device virtio-serial-device \
		-device virtconsole,chardev=log \
		-kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark

all: run

debug: ARGS += -s -S
debug: run

fs:
	rm lemonfs.img && cargo run -p mkfs --target x86_64-unknown-linux-gnu


run:
	@cargo build
	@echo "Running: $(QEMU) $(ARGS)"
	@truncate -s 0 kernel.log
	@$(QEMU) $(ARGS)

test:
	@cargo test
	@cargo test -p allocator --target x86_64-unknown-linux-gnu
	@cargo test -p filesystem --target x86_64-unknown-linux-gnu
	@cargo test -p virtual_memory --target x86_64-unknown-linux-gnu
