# Timer Interrupts and device trees

## Enabling interrupts

RISC-V has 2 flags that need to be enabled in order to make interrupts possible. 
The first one is a global switch that can turn interrupts on or off. The second
one is more fine grained and can allow/disallow specific interrupts.

This is useful as it allow us to disable some interrupts in critical code
sections for example when we're currently dealing with an interrupt we don't
want another interrupt to fire before we're done dealing with the first one.

To enable interrupts globally, we need to set the `Supervisor Interrupts Enable`
bit at index 1 in the `sstatus` register.

To enable the timer interrupt specifically, we need to set the `Timer` bit at index 5
in the 'Supervisor Interrupt Enable Register' `SIE`.

## Starting a timer

In order to start a timer, we first need to read the current time out of a register.

For that we can use the special instruction `rdtime`. The time is not measured in
seconds rather than ticks and ticks depend on the timer that is used.

At the moment we don't have a way to extract this information at runtime but using
QEMU's virt machine, the default timer has a frequency of 10Mhz i.e
10\_000\_000 ticks per second.

Now that we have the time and know by how much we need to increment it to fire
off a timer interrupt in N seconds, we can use the `sbi_set_timer()` function
provided by the SBI.

To call a SBI function, we need to load the function id, into the `a7` register
and push the argument in `a0`.

Then we can call the function with the `ecall` instruction in assembly.

Now we can see that a trap was caught in our previously installed trap handler
function.

If we want to we can catch that exact case, the SupervisorTimerInterrupt and restart
the timer.

## Reading the device tree

In order to understand some of the hardware specs of the underlying system, the RISC-V
SBI defines that the first 2 arguments that are passed to the kernel when it's called,
are the `hart_id` and the `fdt_addr`.

The `hart_id` is not that important for us at this stage but the `fdt_addr` is the base
address to the flattened device tree in memory.

This tree contains information about the system such as how many CPUs do we
have, and also the frequency of the timer.

To parse this tree out of the raw memory, we can use a library `fdt` to help
us. We just need to pass a slice to the tree to this function. 

Here we have a problem though as we don't know the size of this tree. Luckily the first
bytes of the memory are a header structure in a specific format of 

```rust
#[repr(C)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}
```

This means we can just interpret the bytes as the header structure and can read the
`totalsize` field out of it. Easy!

Well, not that easy, unfortunately the Device Tree spec defines all those values to
be interpreted as big endian and our RISC-V kernel is running in little endian.

This means we need to write a little helper function which can read each of the
4 byte numbers and converts them from big endian for us.

Now we have the total size of the device tree and thus can pass the slice to the
`fdt` library which handles the rest of the parsing for us.

The `fdt` contians a nested structure of cpus and from that CPU we can read the
`timebase_frequency()` which is exactly what we need. No more hard-coded values!

## Interrupt stack

It's a common practice to swap the stacks to a 'known good' stack when handling
a TRAP. This is important for example in the case when an exception get's
thrown for a stack overflow. This means that we're already in memory that we
should not be able to access and when we use the same stack for handling this
exception things can only get worse.

For that to work, we tweak the `linker.ld` script to reserve some extra space
and we store a pointer to the top of that stack. Then we load this linker
symbol in the beginning of the `_start()` function and set the `sscratch`
register to that value. 

In order to not use the current stack in the trap handler, we need to make it a
naked function by adding the `#[naked]` attribute. This tells the rust compiler
that it shouldn't add any instructions to the function itself and that we're
going to handle this as we don't want the compiler to insert some instructions
that would still use the old stack and thus trigger a new exception.

By making the function naked we can control exactly which instructions are
executed and thus can swap out the stack in the first instruction. Then 
we need to create a stack frame to store all the caller-saved registers
that might be clobbered in the actual implementation of the trap handler.

Then we can call out to our rust function. After that, we need to restore the 
current state by resetting all the registers, deallocating the stack frame
and swapping the stack back.

Finally we can use the `sret` instruction to return from the trap handler.

