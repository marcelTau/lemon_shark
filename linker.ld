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

    .bss : {
        . = ALIGN(8);
        *(.bss .bss.*)
        . = ALIGN(16);  /* the calling convention for risc v dictates that the stack pointer 16 byte aligned.*/
        . += 16384;     /* allocate 16kb  */
        _stack_top = .; /* define the `_stack_top` symbol */
    }
}
