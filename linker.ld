/* Define the entry point and load the kernel at a specific address */

ENTRY(_start)

SECTIONS
{
    . = 0x80200000;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }
}
