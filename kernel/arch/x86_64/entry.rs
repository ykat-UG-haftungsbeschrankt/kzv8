core::arch::global_asm!(
    ".section .text",
    ".global kernel_main",
    ".type kernel_main,@function",
    "kernel_main:",
    "mov edi, ebx",
    "mov esi, 0x2badb002",
    "jmp kz_kernel_main",
    ".size kernel_main, .-kernel_main",
);
