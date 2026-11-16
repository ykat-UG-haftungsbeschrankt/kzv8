/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

static long linux_write(const char *buffer, unsigned long length) {
    long result;

    __asm__ volatile(
        "syscall"
        : "=a"(result)
        : "a"(1L), "D"(1L), "S"(buffer), "d"(length)
        : "rcx", "r11", "memory");
    return result;
}

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
    static const char message[] = "Hello, world from static Linux!\n";
    long written = linux_write(message, sizeof(message) - 1);

    exit_process(written == (long)(sizeof(message) - 1) ? 0 : 1);
}
