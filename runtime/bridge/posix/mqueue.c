/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#include <errno.h>
#include <mqueue.h>
#include <stdarg.h>

extern mqd_t kz_rust_mq_open(const char *name, int flags, mode_t mode,
                             const struct mq_attr *attributes);

mqd_t mq_open(const char *name, int flags, ...) {
    mode_t mode = 0;
    const struct mq_attr *attributes = NULL;
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode = va_arg(arguments, mode_t);
        attributes = va_arg(arguments, const struct mq_attr *);
        va_end(arguments);
    }
    return kz_rust_mq_open(name, flags, mode, attributes);
}