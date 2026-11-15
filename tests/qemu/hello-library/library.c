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

int hello_from_library(void) {
    static const char message[] = "Hello, world from the loaded library!\n";
    long written = linux_write(message, sizeof(message) - 1);

    return written == (long)(sizeof(message) - 1) ? 0 : 1;
}