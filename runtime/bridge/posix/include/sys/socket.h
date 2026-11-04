#ifndef KZ_POSIX_SYS_SOCKET_H_
#define KZ_POSIX_SYS_SOCKET_H_

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <sys/uio.h>

typedef uint32_t socklen_t;
typedef uint16_t sa_family_t;

struct sockaddr {
    sa_family_t sa_family;
    char sa_data[14];
};

struct sockaddr_storage {
    sa_family_t ss_family;
    uint64_t __ss_align;
    unsigned char __ss_padding[112];
};

struct msghdr {
    void *msg_name;
    socklen_t msg_namelen;
    struct iovec *msg_iov;
    int msg_iovlen;
    void *msg_control;
    socklen_t msg_controllen;
    int msg_flags;
};

struct cmsghdr {
    socklen_t cmsg_len;
    int cmsg_level;
    int cmsg_type;
};

struct linger {
    int l_onoff;
    int l_linger;
};

#define AF_UNSPEC 0
#define AF_UNIX 1
#define AF_LOCAL AF_UNIX
#define AF_INET 2
#define AF_INET6 10

#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_SEQPACKET 5
#define SOCK_NONBLOCK 0x00000800
#define SOCK_CLOEXEC 0x00080000
#define SOCK_CLOFORK 0x00100000

#define SOL_SOCKET 1
#define SO_DEBUG 1
#define SO_REUSEADDR 2
#define SO_TYPE 3
#define SO_ERROR 4
#define SO_DONTROUTE 5
#define SO_BROADCAST 6
#define SO_SNDBUF 7
#define SO_RCVBUF 8
#define SO_KEEPALIVE 9
#define SO_OOBINLINE 10
#define SO_LINGER 13
#define SO_RCVLOWAT 18
#define SO_SNDLOWAT 19
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21
#define SO_ACCEPTCONN 30
#define SO_PROTOCOL 38
#define SO_DOMAIN 39
#define SOMAXCONN 128

#define SCM_RIGHTS 1

#define MSG_OOB 0x0001
#define MSG_PEEK 0x0002
#define MSG_DONTROUTE 0x0004
#define MSG_CTRUNC 0x0008
#define MSG_TRUNC 0x0020
#define MSG_EOR 0x0080
#define MSG_WAITALL 0x0100
#define MSG_NOSIGNAL 0x4000
#define MSG_CMSG_CLOEXEC 0x40000000
#define MSG_CMSG_CLOFORK 0x80000000U

#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2

#define CMSG_ALIGN(length) \
    (((length) + sizeof(size_t) - 1) & ~(sizeof(size_t) - 1))
#define CMSG_DATA(control) \
    ((unsigned char *)(control) + CMSG_ALIGN(sizeof(struct cmsghdr)))
#define CMSG_SPACE(length) \
    (CMSG_ALIGN(sizeof(struct cmsghdr)) + CMSG_ALIGN(length))
#define CMSG_LEN(length) \
    (CMSG_ALIGN(sizeof(struct cmsghdr)) + (length))

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

static inline struct cmsghdr *kz_posix_cmsg_firsthdr(const struct msghdr *message) {
    if (message == NULL || message->msg_control == NULL ||
        message->msg_controllen < sizeof(struct cmsghdr)) {
        return NULL;
    }
    return (struct cmsghdr *)message->msg_control;
}

static inline struct cmsghdr *kz_posix_cmsg_nxthdr(const struct msghdr *message,
                                                    const struct cmsghdr *control) {
    if (control == NULL) {
        return kz_posix_cmsg_firsthdr(message);
    }
    if (message == NULL || message->msg_control == NULL) {
        return NULL;
    }
    uintptr_t base = (uintptr_t)message->msg_control;
    uintptr_t end = base + message->msg_controllen;
    uintptr_t current = (uintptr_t)control;
    if (end < base || current < base || current > end ||
        end - current < sizeof(struct cmsghdr) ||
        control->cmsg_len < sizeof(struct cmsghdr)) {
        return NULL;
    }
    uintptr_t next = current + CMSG_ALIGN(control->cmsg_len);
    if (next < current || next > end || end - next < sizeof(struct cmsghdr)) {
        return NULL;
    }
    return (struct cmsghdr *)next;
}

#define CMSG_FIRSTHDR(message) kz_posix_cmsg_firsthdr(message)
#define CMSG_NXTHDR(message, control) kz_posix_cmsg_nxthdr((message), (control))

int accept(int socket, struct sockaddr *KZ_POSIX_RESTRICT address,
           socklen_t *KZ_POSIX_RESTRICT address_length);
int accept4(int socket, struct sockaddr *KZ_POSIX_RESTRICT address,
            socklen_t *KZ_POSIX_RESTRICT address_length, int flags);
int bind(int socket, const struct sockaddr *address, socklen_t address_length);
int connect(int socket, const struct sockaddr *address, socklen_t address_length);
int getpeername(int socket, struct sockaddr *KZ_POSIX_RESTRICT address,
                socklen_t *KZ_POSIX_RESTRICT address_length);
int getsockname(int socket, struct sockaddr *KZ_POSIX_RESTRICT address,
                socklen_t *KZ_POSIX_RESTRICT address_length);
int getsockopt(int socket, int level, int option_name, void *KZ_POSIX_RESTRICT option_value,
               socklen_t *KZ_POSIX_RESTRICT option_length);
int listen(int socket, int backlog);
ssize_t recv(int socket, void *buffer, size_t length, int flags);
ssize_t recvfrom(int socket, void *KZ_POSIX_RESTRICT buffer, size_t length, int flags,
                 struct sockaddr *KZ_POSIX_RESTRICT address,
                 socklen_t *KZ_POSIX_RESTRICT address_length);
ssize_t recvmsg(int socket, struct msghdr *message, int flags);
ssize_t send(int socket, const void *buffer, size_t length, int flags);
ssize_t sendmsg(int socket, const struct msghdr *message, int flags);
ssize_t sendto(int socket, const void *buffer, size_t length, int flags,
               const struct sockaddr *address, socklen_t address_length);
int setsockopt(int socket, int level, int option_name, const void *option_value,
               socklen_t option_length);
int shutdown(int socket, int how);
int sockatmark(int socket);
int socket(int domain, int type, int protocol);
int socketpair(int domain, int type, int protocol, int sockets[2]);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif