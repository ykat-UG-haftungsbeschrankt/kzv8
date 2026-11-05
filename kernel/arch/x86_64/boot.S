/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

.section .multiboot,"a"
.balign 4
.long 0x1badb002
.long 0
.long -0x1badb002

.section .text.boot,"ax"
.code32
.global _start
_start:
    cli
    cld
    mov $__bss_start, %edi
    mov $__bss_end, %ecx
    sub %edi, %ecx
    xor %eax, %eax
    rep stosb
    mov $stack_top, %esp
    mov $pdpt, %eax
    or $3, %eax
    mov %eax, pml4
    xor %ecx, %ecx
1:
    mov %ecx, %eax
    shl $12, %eax
    add $directories, %eax
    or $3, %eax
    mov %eax, pdpt(,%ecx,8)
    inc %ecx
    cmp $4, %ecx
    jne 1b
    xor %ecx, %ecx
2:
    mov %ecx, %eax
    shl $21, %eax
    or $0x83, %eax
    cmp $1920, %ecx
    jb 3f
    or $0x18, %eax
3:
    mov %eax, directories(,%ecx,8)
    inc %ecx
    cmp $2048, %ecx
    jne 2b
    mov %cr4, %eax
    or $0x620, %eax
    mov %eax, %cr4
#if defined(KZ_FPU_AVX)
    mov $0, %eax
    cpuid
    cmp $0xd, %eax
    jb .Lavx_unsupported
    mov $1, %eax
    cpuid
    mov %ecx, %ebx
    and $0x14000000, %ebx
    cmp $0x14000000, %ebx
    jne .Lavx_unsupported
    mov $0xd, %eax
    xor %ecx, %ecx
    cpuid
    and $7, %eax
    cmp $7, %eax
    jne .Lavx_unsupported
    mov %cr4, %eax
    or $0x40000, %eax
    mov %eax, %cr4
    xor %ecx, %ecx
    mov $7, %eax
    xor %edx, %edx
    xsetbv
    jmp .Lavx_ready
.Lavx_unsupported:
    cli
.Lavx_halt:
    hlt
    jmp .Lavx_halt
.Lavx_ready:
#endif
    mov $pml4, %eax
    mov %eax, %cr3
    mov $0xc0000080, %ecx
    rdmsr
    or $0x100, %eax
    wrmsr
    lgdt gdt_pointer
    mov %cr0, %eax
    and $0xfffffffb, %eax
    or $0x80000002, %eax
    mov %eax, %cr0
    ljmp $8, $long_mode
.code64
long_mode:
    mov $16, %eax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov $stack_top, %rsp
    xor %rbp, %rbp
    call kernel_main
4:
    hlt
    jmp 4b
.balign 8
gdt:
    .quad 0
    .quad 0x00af9a000000ffff
    .quad 0x00cf92000000ffff
gdt_end:
gdt_pointer:
    .word gdt_end-gdt-1
    .long gdt

.section .bss,"aw",@nobits
.balign 4096
pml4:
    .skip 4096
pdpt:
    .skip 4096
directories:
    .skip 16384
.balign 16
stack_bottom:
    .skip 65536
stack_top:
.section .note.GNU-stack,"",@progbits