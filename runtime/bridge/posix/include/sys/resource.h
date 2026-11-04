#ifndef KZ_POSIX_SYS_RESOURCE_H_
#define KZ_POSIX_SYS_RESOURCE_H_

#include <sys/time.h>
#include <sys/types.h>

typedef uint64_t rlim_t;

#define RLIM_INFINITY ((rlim_t)-1)
#define RLIM_SAVED_MAX RLIM_INFINITY
#define RLIM_SAVED_CUR RLIM_INFINITY

#define RLIMIT_CPU 0
#define RLIMIT_FSIZE 1
#define RLIMIT_DATA 2
#define RLIMIT_STACK 3
#define RLIMIT_CORE 4
#define RLIMIT_NOFILE 5
#define RLIMIT_AS 9

#define RUSAGE_SELF 0
#define RUSAGE_CHILDREN (-1)

#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

struct rlimit {
    rlim_t rlim_cur;
    rlim_t rlim_max;
};

struct rusage {
    struct timeval ru_utime;
    struct timeval ru_stime;
    long ru_maxrss;
    long ru_ixrss;
    long ru_idrss;
    long ru_isrss;
    long ru_minflt;
    long ru_majflt;
    long ru_nswap;
    long ru_inblock;
    long ru_oublock;
    long ru_msgsnd;
    long ru_msgrcv;
    long ru_nsignals;
    long ru_nvcsw;
    long ru_nivcsw;
};

#ifdef __cplusplus
extern "C" {
#endif

int getpriority(int which, id_t who);
int getrlimit(int resource, struct rlimit *limit);
int getrusage(int who, struct rusage *usage);
int setpriority(int which, id_t who, int priority);
int setrlimit(int resource, const struct rlimit *limit);

#ifdef __cplusplus
}
#endif

#endif