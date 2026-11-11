#ifndef KZ_POSIX_NET_IF_H_
#define KZ_POSIX_NET_IF_H_

#include <sys/socket.h>

/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#define IF_NAMESIZE 16
#define IFNAMSIZ IF_NAMESIZE

struct ifreq {
    char ifr_name[IF_NAMESIZE];
    union {
        short ifru_flags;
        int ifru_mtu;
        struct sockaddr ifru_addr;
        char ifru_data[24];
    } ifr_ifru;
};

#define ifr_flags ifr_ifru.ifru_flags
#define ifr_mtu ifr_ifru.ifru_mtu
#define ifr_addr ifr_ifru.ifru_addr
#define ifr_data ifr_ifru.ifru_data

#define IFF_UP 0x0001
#define IFF_RUNNING 0x0040

#define SIOCGIFFLAGS 0x8913
#define SIOCSIFFLAGS 0x8914
#define SIOCGIFADDR 0x8915
#define SIOCSIFADDR 0x8916
#define SIOCGIFNETMASK 0x891b
#define SIOCSIFNETMASK 0x891c
#define SIOCGIFMTU 0x8921
#define SIOCGIFHWADDR 0x8927

struct if_nameindex {
    unsigned int if_index;
    char *if_name;
};

#ifdef __cplusplus
extern "C" {
#endif

void if_freenameindex(struct if_nameindex *interfaces);
char *if_indextoname(unsigned int interface_index, char *name);
struct if_nameindex *if_nameindex(void);
unsigned int if_nametoindex(const char *name);

#ifdef __cplusplus
}
#endif

#endif