#!/bin/bash

cargo build 

if [[ $? -ne 0 ]]; then
    printf "\n\nFailed!\n"
    exit 1
fi

qemu-system-riscv64 \
    -machine virt \
    -bios default \
    -kernel ./target/riscv64gc-unknown-none-elf/debug/lemon_shark \
    -nographic
