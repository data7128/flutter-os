/* AeroOS user-space program linker script.
 *
 * User ELF binaries are loaded at virtual address 0x400000 (see the
 * kernel's ELF loader in kernel/src/userproc.rs). The kernel maps each
 * PT_LOAD segment, zero-fills BSS, and builds a fresh user stack at
 * 0x7fff00000000. `_start` is the true entry point.
 */
ENTRY(_start)

SECTIONS {
    . = 0x400000;

    .text : {
        *(.text .text.*)
    }
    .rodata : {
        *(.rodata .rodata.*)
    }
    .data : {
        *(.data .data.*)
        *(.got .got.*)
    }
    .bss : {
        *(.bss .bss.*)
    }

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment*)
        *(.note*)
    }
}
