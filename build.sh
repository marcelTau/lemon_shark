#!/bin/bash

cargo build $@

if [[ $? -ne 0 ]]; then
    printf "\n\nFailed!\n"
    exit 1
fi

qemu-system-riscv64 \
    -machine virt \
    -bios default \
    -cpu rv64 \
    -kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark \
    -nographic \
    -d int,cpu_reset \
    -D qemu.log \
    # -semihosting \
    # -semihosting-config enable=on,target=native
    # -s -S
