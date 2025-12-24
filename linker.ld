/* Define the entry point and load the kernel at a specific address */

ENTRY(_boot)

SECTIONS
{
    . = 0x80200000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : {
        . = ALIGN(8);
        *(.rodata .rodata.*)
    }

    .data : {
        . = ALIGN(8);
        *(.data .data.*)
    }

    /* Define global pointer symbol for efficient global variable access */
    __global_pointer$ = MIN(. + 0x800, 0x80200000 + 0x7ff);

    .bss : {
        . = ALIGN(8);
        *(.bss .bss.*)


        /* Reserve some stack for the trap handler */
        /* We save 16 registers (128 bytes) + trap_handler_rust uses ~128 bytes = need at least 512 bytes */
        . = ALIGN(16);
        _trap_stack_start = .;
        . += 8192;  /* 8KB to be safe */
        _trap_stack_top = .;

        /* Reserve stack for kernel */
        . = ALIGN(16);  /* the calling convention for risc v dictates that the stack pointer 16 byte aligned.*/
        _stack_start = .;
        . += 16384;     /* allocate 16kb  */
        _stack_top = .; /* define the `_stack_top` symbol */
    }
}
