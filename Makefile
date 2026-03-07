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
		-drive file=disk.img,if=none,format=raw,id=hd0 \
		-device virtio-blk-device,drive=hd0 \
		-chardev file,id=log,path=kernel.log \
		-device virtio-serial-device \
		-device virtconsole,chardev=log \
		-kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark

all: run

debug: ARGS += -s -S
debug: run

reset_disk:
	rm disk.img && truncate -s 16M disk.img	

run:
	@cargo build
	@echo "Running: $(QEMU) $(ARGS)"
	@$(QEMU) $(ARGS)

test:
	@cargo test



#	qemu-system-riscv64 \
#   -machine virt \
#   -bios default \
#   -cpu rv64 \
#   -nographic \
#   -kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark \
#   -drive file=disk.img,if=none,id=hd0,format=raw \
#   -device virtio-blk-device,drive=hd0
