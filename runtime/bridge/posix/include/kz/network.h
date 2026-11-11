/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_KZ_NETWORK_H_
#define KZ_POSIX_KZ_NETWORK_H_

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int kz_host_network_set_ipv4_forwarding(int enabled);
int kz_host_network_set_ipv4_ttl(uint32_t ttl);
int kz_host_network_set_receive_budget(uint32_t budget);
int kz_host_network_add_ipv4_route(const uint8_t network[4], uint32_t prefix_length,
                                   const uint8_t gateway[4]);
int kz_host_network_delete_ipv4_route(const uint8_t network[4], uint32_t prefix_length);
int kz_host_network_add_ipv4_tun_route(const uint8_t network[4], uint32_t prefix_length,
                                       const char *interface_name);

#ifdef __cplusplus
}
#endif

#endif