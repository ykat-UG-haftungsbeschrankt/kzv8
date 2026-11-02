ENTRY(_start)
SECTIONS {
    . = 0x100000;
    .multiboot : { KEEP(*(.multiboot)) }
    .text : ALIGN(4096) { *(.text.boot) *(.text .text.*) }
    .rodata : ALIGN(4096) { *(.rodata .rodata.*) }
    .data : ALIGN(4096) { *(.data .data.*) *(.got .got.*) }
    .tdata : ALIGN(8) {
        __tls_start = .;
        *(.tdata .tdata.*)
        __tls_data_end = .;
    }
    .tbss (NOLOAD) : ALIGN(8) {
        *(.tbss .tbss.*)
        __tls_end = .;
    }
    .bss (NOLOAD) : ALIGN(4096) {
        __bss_start = .;
        *(.bss .bss.*) *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }
    /DISCARD/ : { *(.eh_frame*) *(.comment*) }
}