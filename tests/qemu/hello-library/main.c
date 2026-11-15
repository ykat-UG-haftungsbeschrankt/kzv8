/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

extern int hello_from_library(void);

static __attribute__((noreturn)) void exit_process(int status) {
    long syscall_number = 60;

    __asm__ volatile(
        "syscall"
        : "+a"(syscall_number)
        : "D"((long)status)
        : "rcx", "r11", "memory");
    __builtin_unreachable();
}

__attribute__((noreturn)) void _start(void) {
    exit_process(hello_from_library());
}