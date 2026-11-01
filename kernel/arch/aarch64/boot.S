.section .text.boot,"ax"
.global _start
_start:
    msr daifset, #15
    mrs x9, CurrentEL
    cmp x9, #8
    b.ne el1_start
    mov x9, #(1 << 31)
    msr hcr_el2, x9
    msr cptr_el2, xzr
    mov x9, #0x3c5
    msr spsr_el2, x9
    adr x9, el1_start
    msr elr_el2, x9
    eret
el1_start:
    msr spsel, #1
    mov x9, #(3 << 20)
    msr cpacr_el1, x9
    isb
    ldr x9, =__bss_start
    ldr x10, =__bss_end
1:
    cmp x9, x10
    b.hs 2f
    str xzr, [x9], #8
    b 1b
2:
    ldr x9, =stack_top
    mov sp, x9
    bl kernel_main
3:
    wfi
    b 3b

.section .bss,"aw",%nobits
.balign 16
stack_bottom:
    .skip 65536
stack_top:
.section .note.GNU-stack,"",%progbits