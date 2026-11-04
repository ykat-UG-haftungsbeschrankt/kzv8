#ifndef KZ_POSIX_SYS_TIME_H_
#define KZ_POSIX_SYS_TIME_H_

#include <sys/select.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int gettimeofday(struct timeval *KZ_POSIX_RESTRICT value,
                 void *KZ_POSIX_RESTRICT timezone);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif