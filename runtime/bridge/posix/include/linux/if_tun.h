/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_LINUX_IF_TUN_H_
#define KZ_POSIX_LINUX_IF_TUN_H_

#include <net/if.h>
#include <sys/ioctl.h>

#define IFF_TUN 0x0001
#define IFF_TAP 0x0002
#define IFF_NO_PI 0x1000
#define IFF_ONE_QUEUE 0x2000
#define IFF_VNET_HDR 0x4000
#define IFF_MULTI_QUEUE 0x0100
#define IFF_NAPI 0x0010
#define IFF_NAPI_FRAGS 0x0020

#define TUNSETIFF _IOW('T', 202, int)
#define TUNGETIFF _IOR('T', 210, int)

#endif