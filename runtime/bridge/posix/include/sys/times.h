#ifndef KZ_POSIX_SYS_TIMES_H_
#define KZ_POSIX_SYS_TIMES_H_

#include <sys/types.h>

struct tms {
    clock_t tms_utime;
    clock_t tms_stime;
    clock_t tms_cutime;
    clock_t tms_cstime;
};

#ifdef __cplusplus
extern "C" {
#endif

clock_t times(struct tms *buffer);

#ifdef __cplusplus
}
#endif

#endif