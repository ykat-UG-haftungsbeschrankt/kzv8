#ifndef KZ_POSIX_SYS_SELECT_H_
#define KZ_POSIX_SYS_SELECT_H_

#include <signal.h>
#include <sys/types.h>
#include <time.h>

#define FD_SETSIZE 1024
#define __KZ_FD_WORD_BITS (8 * sizeof(unsigned long))
#define __KZ_FD_WORD_COUNT (FD_SETSIZE / __KZ_FD_WORD_BITS)

typedef struct {
    unsigned long __bits[__KZ_FD_WORD_COUNT];
} fd_set;

struct timeval {
    time_t tv_sec;
    suseconds_t tv_usec;
};

static inline void kz_fd_zero(fd_set *set) {
    for (size_t index = 0; index < __KZ_FD_WORD_COUNT; ++index) {
        set->__bits[index] = 0;
    }
}

static inline void kz_fd_set(int descriptor, fd_set *set) {
    if (descriptor >= 0 && descriptor < FD_SETSIZE) {
        unsigned int value = (unsigned int)descriptor;
        set->__bits[value / __KZ_FD_WORD_BITS] |=
            1UL << (value % __KZ_FD_WORD_BITS);
    }
}

static inline void kz_fd_clr(int descriptor, fd_set *set) {
    if (descriptor >= 0 && descriptor < FD_SETSIZE) {
        unsigned int value = (unsigned int)descriptor;
        set->__bits[value / __KZ_FD_WORD_BITS] &=
            ~(1UL << (value % __KZ_FD_WORD_BITS));
    }
}

static inline int kz_fd_isset(int descriptor, const fd_set *set) {
    if (descriptor < 0 || descriptor >= FD_SETSIZE) {
        return 0;
    }
    unsigned int value = (unsigned int)descriptor;
    return (set->__bits[value / __KZ_FD_WORD_BITS] &
            (1UL << (value % __KZ_FD_WORD_BITS))) != 0;
}

#define FD_ZERO(set) kz_fd_zero(set)
#define FD_SET(descriptor, set) kz_fd_set((descriptor), (set))
#define FD_CLR(descriptor, set) kz_fd_clr((descriptor), (set))
#define FD_ISSET(descriptor, set) kz_fd_isset((descriptor), (set))

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int select(int descriptor_count, fd_set *KZ_POSIX_RESTRICT read_set,
           fd_set *KZ_POSIX_RESTRICT write_set, fd_set *KZ_POSIX_RESTRICT error_set,
           struct timeval *KZ_POSIX_RESTRICT timeout);
int pselect(int descriptor_count, fd_set *KZ_POSIX_RESTRICT read_set,
            fd_set *KZ_POSIX_RESTRICT write_set, fd_set *KZ_POSIX_RESTRICT error_set,
            const struct timespec *KZ_POSIX_RESTRICT timeout,
            const sigset_t *KZ_POSIX_RESTRICT signal_mask);

#undef KZ_POSIX_RESTRICT
#undef __KZ_FD_WORD_BITS
#undef __KZ_FD_WORD_COUNT
#ifdef __cplusplus
}
#endif

#endif