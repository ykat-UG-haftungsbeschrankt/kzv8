#ifndef KZ_POSIX_SCHED_H_
#define KZ_POSIX_SCHED_H_

#include <sys/types.h>
#include <time.h>

struct sched_param {
    int sched_priority;
};

#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_SPORADIC 3

#ifdef __cplusplus
extern "C" {
#endif

int sched_get_priority_max(int policy);
int sched_get_priority_min(int policy);
int sched_getparam(pid_t process_id, struct sched_param *parameters);
int sched_getscheduler(pid_t process_id);
int sched_rr_get_interval(pid_t process_id, struct timespec *interval);
int sched_setparam(pid_t process_id, const struct sched_param *parameters);
int sched_setscheduler(pid_t process_id, int policy,
                       const struct sched_param *parameters);
int sched_yield(void);

#ifdef __cplusplus
}
#endif

#endif