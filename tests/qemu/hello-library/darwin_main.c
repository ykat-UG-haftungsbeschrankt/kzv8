/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

extern int (*hello_library_entry)(void);

static __attribute__((noreturn)) void darwin_exit(int status) {
    long syscall_number = 0x02000001L;

    __asm__ volatile(
        "syscall"
        : "+a"(syscall_number)
        : "D"((long)status)
        : "rcx", "r11", "memory");
    __builtin_unreachable();
}

__attribute__((noreturn)) void _start(void) {
    darwin_exit(hello_library_entry());
}