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
    movb $'1', %al
    outb %al, $0xe9
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
    ljmpl $0x18, $(AP_TRAMPOLINE_BASE + ap_trampoline_pm32 - ap_trampoline_start)

.code32
ap_trampoline_pm32:
    movb $'2', %al
    outb %al, $0xe9
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl $pml4, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $0x620, %eax
    movl %eax, %cr4
#if defined(KZ_FPU_AVX)
    movl $0, %eax
    cpuid
    cmpl $0xd, %eax
    jb .Lap_avx_unsupported
    movl $1, %eax
    cpuid
    movl %ecx, %ebx
    andl $0x14000000, %ebx
    cmpl $0x14000000, %ebx
    jne .Lap_avx_unsupported
    movl $0xd, %eax
    xorl %ecx, %ecx
    cpuid
    andl $7, %eax
    cmpl $7, %eax
    jne .Lap_avx_unsupported
    movl %cr4, %eax
    orl $0x40000, %eax
    movl %eax, %cr4
    xorl %ecx, %ecx
    movl $7, %eax
    xorl %edx, %edx
    xsetbv
    jmp .Lap_avx_ready
.Lap_avx_unsupported:
    cli
.Lap_avx_halt:
    hlt
    jmp .Lap_avx_halt
.Lap_avx_ready:
#endif
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
    movb $'3', %al
    outb %al, $0xe9
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movabsq $(AP_TRAMPOLINE_BASE + (ap_trampoline_stack - ap_trampoline_start)), %rax
    movq (%rax), %rsp
    movabsq $(AP_TRAMPOLINE_BASE + (ap_trampoline_slot - ap_trampoline_start)), %rax
    movl (%rax), %edi
    pushq %rax
    popq %rax
    movb $'5', %al
    outb %al, $0xe9
    xorq %rbp, %rbp
    movabsq $kz_secondary_main, %r11
    movb $'4', %al
    outb %al, $0xe9
    call *%r11
6:
    hlt
    jmp 6b

.code16
.balign 8
ap_trampoline_gdt:
    .quad 0
    .quad 0x00af9a000000ffff
    .quad 0x00cf92000000ffff
    .quad 0x00cf9a000000ffff
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
