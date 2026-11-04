#ifndef KZ_POSIX_NETINET_IN_H_
#define KZ_POSIX_NETINET_IN_H_

#include <stdint.h>
#include <sys/socket.h>
#include <sys/types.h>

typedef uint16_t in_port_t;
typedef uint32_t in_addr_t;

struct in_addr {
    in_addr_t s_addr;
};

struct sockaddr_in {
    sa_family_t sin_family;
    in_port_t sin_port;
    struct in_addr sin_addr;
};

struct in6_addr {
    uint8_t s6_addr[16];
};

struct sockaddr_in6 {
    sa_family_t sin6_family;
    in_port_t sin6_port;
    uint32_t sin6_flowinfo;
    struct in6_addr sin6_addr;
    uint32_t sin6_scope_id;
};

struct ipv6_mreq {
    struct in6_addr ipv6mr_multiaddr;
    unsigned int ipv6mr_interface;
};

#define INADDR_ANY ((in_addr_t)0x00000000U)
#define INADDR_BROADCAST ((in_addr_t)0xffffffffU)
#define INET_ADDRSTRLEN 16
#define INET6_ADDRSTRLEN 46
#define IN6ADDR_ANY_INIT {{0}}
#define IN6ADDR_LOOPBACK_INIT {{0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1}}

#define IPPROTO_IP 0
#define IPPROTO_ICMP 1
#define IPPROTO_TCP 6
#define IPPROTO_UDP 17
#define IPPROTO_IPV6 41
#define IPPROTO_RAW 255

#define IPV6_JOIN_GROUP 20
#define IPV6_LEAVE_GROUP 21
#define IPV6_MULTICAST_IF 17
#define IPV6_MULTICAST_HOPS 18
#define IPV6_MULTICAST_LOOP 19
#define IPV6_UNICAST_HOPS 16
#define IPV6_V6ONLY 26

#ifdef __cplusplus
extern "C" {
#endif

extern const struct in6_addr in6addr_any;
extern const struct in6_addr in6addr_loopback;

#ifdef __cplusplus
}
#endif

static inline int kz_posix_in6_is_zero(const struct in6_addr *address,
                                       unsigned int length) {
    for (unsigned int index = 0; index < length; ++index) {
        if (address->s6_addr[index] != 0) {
            return 0;
        }
    }
    return 1;
}

static inline int kz_posix_in6_is_v4mapped(const struct in6_addr *address) {
    return kz_posix_in6_is_zero(address, 10) &&
           address->s6_addr[10] == 0xff && address->s6_addr[11] == 0xff;
}

static inline int kz_posix_in6_is_v4compat(const struct in6_addr *address) {
    if (!kz_posix_in6_is_zero(address, 12)) {
        return 0;
    }
    uint32_t tail = ((uint32_t)address->s6_addr[12] << 24) |
                    ((uint32_t)address->s6_addr[13] << 16) |
                    ((uint32_t)address->s6_addr[14] << 8) | address->s6_addr[15];
    return tail > 1;
}

#define IN6_IS_ADDR_UNSPECIFIED(address) kz_posix_in6_is_zero((address), 16)
#define IN6_IS_ADDR_LOOPBACK(address) \
    (kz_posix_in6_is_zero((address), 15) && (address)->s6_addr[15] == 1)
#define IN6_IS_ADDR_MULTICAST(address) ((address)->s6_addr[0] == 0xff)
#define IN6_IS_ADDR_LINKLOCAL(address) \
    ((address)->s6_addr[0] == 0xfe && ((address)->s6_addr[1] & 0xc0) == 0x80)
#define IN6_IS_ADDR_SITELOCAL(address) \
    ((address)->s6_addr[0] == 0xfe && ((address)->s6_addr[1] & 0xc0) == 0xc0)
#define IN6_IS_ADDR_V4MAPPED(address) kz_posix_in6_is_v4mapped((address))
#define IN6_IS_ADDR_V4COMPAT(address) kz_posix_in6_is_v4compat((address))
#define IN6_IS_ADDR_MC_NODELOCAL(address) \
    (IN6_IS_ADDR_MULTICAST(address) && (((address)->s6_addr[1] & 0x0f) == 1))
#define IN6_IS_ADDR_MC_LINKLOCAL(address) \
    (IN6_IS_ADDR_MULTICAST(address) && (((address)->s6_addr[1] & 0x0f) == 2))
#define IN6_IS_ADDR_MC_SITELOCAL(address) \
    (IN6_IS_ADDR_MULTICAST(address) && (((address)->s6_addr[1] & 0x0f) == 5))
#define IN6_IS_ADDR_MC_ORGLOCAL(address) \
    (IN6_IS_ADDR_MULTICAST(address) && (((address)->s6_addr[1] & 0x0f) == 8))
#define IN6_IS_ADDR_MC_GLOBAL(address) \
    (IN6_IS_ADDR_MULTICAST(address) && (((address)->s6_addr[1] & 0x0f) == 14))

#endif