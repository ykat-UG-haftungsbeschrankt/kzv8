/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

core::arch::global_asm!(
    ".section .text",
    ".global kernel_main",
    ".type kernel_main,%function",
    "kernel_main:",
    "mov x1, xzr",
    "b kz_kernel_main",
    ".size kernel_main, .-kernel_main",
);
