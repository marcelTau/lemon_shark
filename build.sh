#!/bin/bash

qemu-system-riscv64 \
    -machine virt \
    -bios default \
    -kernel ./target/riscv64gc-unknown-none-elf/release/lemon_shark \
    -nographic
