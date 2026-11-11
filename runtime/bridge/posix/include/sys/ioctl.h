/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_SYS_IOCTL_H_
#define KZ_POSIX_SYS_IOCTL_H_

#ifndef _IOC
#define _IOC(direction, type, number, size) \
    (((direction) << 30) | ((size) << 16) | ((type) << 8) | (number))
#define _IOC_NONE 0U
#define _IOC_WRITE 1U
#define _IOC_READ 2U
#define _IO(type, number) _IOC(_IOC_NONE, (type), (number), 0)
#define _IOR(type, number, data_type) \
    _IOC(_IOC_READ, (type), (number), sizeof(data_type))
#define _IOW(type, number, data_type) \
    _IOC(_IOC_WRITE, (type), (number), sizeof(data_type))
#define _IOWR(type, number, data_type) \
    _IOC(_IOC_READ | _IOC_WRITE, (type), (number), sizeof(data_type))
#endif

#ifdef __cplusplus
extern "C" {
#endif

int ioctl(int file_descriptor, unsigned long request, ...);

#ifdef __cplusplus
}
#endif

#endif