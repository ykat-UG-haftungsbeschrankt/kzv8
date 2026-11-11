/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#include <errno.h>
#include <linux/fs.h>
#include <stdarg.h>
#include <sys/ioctl.h>
#include <unistd.h>

extern int kz_rust_ioctl(int file_descriptor, unsigned long request, void *argument);

int ioctl(int file_descriptor, unsigned long request, ...) {
    if (request != FS_IOC_GETFLAGS && request != FS_IOC_SETFLAGS) {
        return kz_rust_ioctl(file_descriptor, request, NULL);
    }
    va_list arguments;
    va_start(arguments, request);
    void *argument = va_arg(arguments, void *);
    va_end(arguments);
    return kz_rust_ioctl(file_descriptor, request, argument);
}

int execl(const char *path, const char *argument, ...) {
    (void)path;
    (void)argument;
    errno = ENOSYS;
    return -1;
}

int execle(const char *path, const char *argument, ...) {
    (void)path;
    (void)argument;
    errno = ENOSYS;
    return -1;
}

int execlp(const char *file, const char *argument, ...) {
    (void)file;
    (void)argument;
    errno = ENOSYS;
    return -1;
}
