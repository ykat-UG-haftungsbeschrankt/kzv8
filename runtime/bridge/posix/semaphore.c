/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#include <errno.h>
#include <semaphore.h>
#include <stdarg.h>

extern sem_t *kz_rust_sem_open(const char *name, int flags,
                               unsigned int mode, unsigned int value);

sem_t *sem_open(const char *name, int flags, ...) {
    unsigned int mode = 0;
    unsigned int value = 0;
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode = va_arg(arguments, unsigned int);
        value = va_arg(arguments, unsigned int);
        va_end(arguments);
    }
    return kz_rust_sem_open(name, flags, mode, value);
}