/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

.section .text.ap_trampoline,"ax"
.code16
.set AP_TRAMPOLINE_BASE, 0x8000
.global ap_trampoline_start
.global ap_trampoline_end
.global ap_trampoline_stack
.global ap_trampoline_slot
.extern pml4
.extern kz_secondary_main

ap_trampoline_start:
    cli
    cld
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    inb $0x92, %al
    andb $0xfe, %al
    orb $0x02, %al
    outb %al, $0x92
    lgdt AP_TRAMPOLINE_BASE + (ap_trampoline_gdt_pointer - ap_trampoline_start)
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl $0x08, $(AP_TRAMPOLINE_BASE + ap_trampoline_pm32 - ap_trampoline_start)

.code32
ap_trampoline_pm32:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl $pml4, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $0x620, %eax
    movl %eax, %cr4
    movl $0x80000001, %eax
    cpuid
    movl %edx, %ebx
    movl $0xc0000080, %ecx
    rdmsr
    orl $0x100, %eax
    testl $(1 << 20), %ebx
    jz 5f
    orl $0x800, %eax
5:
    wrmsr
    movl %cr0, %eax
    andl $0xfffffffb, %eax
    orl $0x80000002, %eax
    movl %eax, %cr0
    .byte 0xea
    .long AP_TRAMPOLINE_BASE + (ap_trampoline_long_mode - ap_trampoline_start)
    .word 0x08

.code64
ap_trampoline_long_mode:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movabsq $(AP_TRAMPOLINE_BASE + (ap_trampoline_stack - ap_trampoline_start)), %rax
    movq (%rax), %rsp
    movabsq $(AP_TRAMPOLINE_BASE + (ap_trampoline_slot - ap_trampoline_start)), %rax
    movl (%rax), %edi
    xorq %rbp, %rbp
    movabsq $kz_secondary_main, %rax
    call *%rax
6:
    hlt
    jmp 6b

.code16
.balign 8
ap_trampoline_gdt:
    .quad 0
    .quad 0x00af9a000000ffff
    .quad 0x00cf92000000ffff
ap_trampoline_gdt_end:
ap_trampoline_gdt_pointer:
    .word ap_trampoline_gdt_end-ap_trampoline_gdt-1
    .long AP_TRAMPOLINE_BASE + (ap_trampoline_gdt - ap_trampoline_start)
.balign 8
ap_trampoline_stack:
    .quad 0
ap_trampoline_slot:
    .long 0
ap_trampoline_end:

.section .note.GNU-stack,"",@progbits
