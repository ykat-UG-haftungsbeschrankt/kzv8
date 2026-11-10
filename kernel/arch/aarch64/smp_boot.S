/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

.section .text.boot.smp,"ax"
.global aarch64_secondary_entry
.extern KZ_CPU_STACKS
.extern kz_secondary_main

aarch64_secondary_entry:
    msr daifset, #15
    mrs x9, CurrentEL
    cmp x9, #8
    b.eq aarch64_secondary_el2
    cmp x9, #4
    b.eq aarch64_secondary_el1
1:
    wfi
    b 1b

aarch64_secondary_el2:
#ifdef KZ_AARCH64_STAGE2
    bl kz_aarch64_el2_initialize
#endif
    mov x9, #(1 << 31)
    msr hcr_el2, x9
    msr cptr_el2, xzr
    mov x9, #0x3c5
    msr spsr_el2, x9
    adr x9, aarch64_secondary_el1
    msr elr_el2, x9
    eret

aarch64_secondary_el1:
    msr spsel, #1
    msr tpidr_el1, x0
    mov x9, #(3 << 20)
    msr cpacr_el1, x9
    isb
    ldr x9, =KZ_CPU_STACKS
    add x9, x9, x0, lsl #14
    add sp, x9, #16384
    b kz_secondary_main

.section .note.GNU-stack,"",%progbits
