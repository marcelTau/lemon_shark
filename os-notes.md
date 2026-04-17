# Risc V

- the stack grows downwards
- stack pointer is 16 byte aligned
- div by 0 is not an exception and is well defined for performance reasons

## SBI (Supervisor Binary Interface) -- OpenSBI

Like a Mini BIO - running firmware

This is a RISC-V specific implementation that defines the access from S-mode
into M-mode for example for reboots.

## Kernel vs Userspace

KernelMode has full access to the hardware. Userspace can only communicate
with hardware via syscalls through the kernel.

On RISC-V there are 3 modes

M-mode (machine): Full hardware access
S-mode (supervisor): where the kernel runs
U-mode (user): applications

# Assembly

## linker symbols
`_stack_top` is set by the linker to the top of the stack

## Instructions
`la <dest>, <src>` - load address 
`call <fn>` - call function

## Calling convention for SBI functions

`a7` SBI function id of the _extension_ we're calling (SBI extension ID EID)
`a6` SBI function id (FID) of the extension id encoded in a7 for SBI extension defined in SBIv0.2
`a0, a1, ..., a5` function parameters

`ecall` to execute the function

`a0` contains the return code of the SBI call
`a1` contains the return value?

`ra` contains return address

**All registers except `a0` and `a1` must be preserved by the caller**

## Registers

- General purpose registers (GPRs) x0-x31 (other names for aN,tN ...)
    - like rax, rbx etc on x86

- Control and Status Register (CSRs)
    - control CPU behaviour
    - store system state
    - have special instructions (starting with `csr`)

`stvec`: jump to that address when trap occures
`satp`: page table pointer
`mepc`: "machine exception program counter"
`sepc`: "Supervisor exception program counter" contains PC when exception is thrown
`scause`: exception cause

`csr{r,w,s,c}` CSR instructions to read/write/set bit/clear bits
`csrrw` Swaps atomically

# Qemu

- UART (Universal Asynchronous Receiver/Transmitter)
Is the hardware interface that we use to print to the console via memory mapped IO.

# Traps vs Interrupts
**Traps are both interrupts + exceptions**

Interrupts can be from the OS (timer, IO)
Exceptions are thrown by the CPU (div by zero), page faults etc.

# Exceptions

When an exception occures:
1. Save the PC to `sepc`
2. Write cause to `scause`
3. Jump to address in `stvec`

- Stack pointer (sp) holds the address of the top element of the stack in RAM.

# Boot sequence:

1. Setup stack
2. Setup trap handler (stvec)
...
N. Enable interrupts when ready


# ELF sections

.text - code (machine instructions)
.rodata - read only data (string literals, const values)
.data - initialized global variables & statics
.bss - Uninitialized global variables

__This is the conventional ordering of the sections in an executable, even though they are changable__


# Tools

- Use `readelf` to inspect the ELF file.
```
readelf -h <binary>
=> shows ELF header output - determine entry point

readelf -S <binary>
=> shows the sections

```

# Links

extension functions for SBI: https://github.com/riscv-non-isa/riscv-sbi-doc/tree/master/src
SBI spec: file:///home/marcel/Downloads/riscv-sbi.pdf (https://github.com/riscv-non-isa/riscv-sbi-doc/releases)
  - https://www.scs.stanford.edu/~zyedidia/docs/riscv/riscv-sbi.pdf
calling convention: https://riscv.org/wp-content/uploads/2024/12/riscv-calling.pdf
privileged mode: file:///home/marcel/Downloads/riscv-privileged.pdf (https://github.com/riscv/riscv-isa-manual/releases/tag/riscv-isa-release-cc9df7f-2025-12-16)
device tree spec: file:///home/marcel/Downloads/devicetree-specification-v0.4.pdf (https://github.com/devicetree-org/devicetree-specification/releases)
interrupts: https://courses.grainger.illinois.edu/ECE391/fa2025/docs/riscv-plic-1.0.0.pdf
instructions: https://msyksphinz-self.github.io/riscv-isadoc/#_csrrc

virtio: https://docs.oasis-open.org/virtio/virtio/v1.1/cs01/virtio-v1.1-cs01.html#x1-1440002

**all instructions (very good)**: file:///home/marcel/Downloads/riscv-unprivileged.pdf

**blog about riscv**: https://operating-system-in-1000-lines.vercel.app/en/11-page-table
