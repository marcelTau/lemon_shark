QEMU := qemu-system-riscv64 

 # replace machine with dumpdtb=qemu.dtb to dump device tree. Use dtc to decompile it.
ARGS := -machine virt \
		-bios default \
		-cpu rv64     \
		-nographic    \
		-drive file=disk.img,if=none,format=raw,id=hd0 \
		-device virtio-blk-device,drive=hd0 \
		-kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark 

all: run

debug: ARGS += -s -S
debug: run

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
