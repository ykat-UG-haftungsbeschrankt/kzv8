#ifndef KZ_POSIX_POLL_H_
#define KZ_POSIX_POLL_H_

#include <signal.h>
#include <time.h>
#include <sys/types.h>

typedef unsigned long nfds_t;

struct pollfd {
    int fd;
    short events;
    short revents;
};

#define POLLIN 0x0001
#define POLLPRI 0x0002
#define POLLOUT 0x0004
#define POLLERR 0x0008
#define POLLHUP 0x0010
#define POLLNVAL 0x0020
#define POLLRDNORM 0x0040
#define POLLRDBAND 0x0080
#define POLLWRNORM POLLOUT
#define POLLWRBAND 0x0100

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int poll(struct pollfd descriptors[], nfds_t descriptor_count, int timeout);
int ppoll(struct pollfd descriptors[], nfds_t descriptor_count,
          const struct timespec *KZ_POSIX_RESTRICT timeout,
          const sigset_t *KZ_POSIX_RESTRICT signal_mask);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif