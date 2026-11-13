/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_SCHED_H_
#define KZ_POSIX_SCHED_H_

#include <sys/types.h>
#include <time.h>

struct sched_param {
    int sched_priority;
    int sched_ss_low_priority;
    struct timespec sched_ss_repl_period;
    struct timespec sched_ss_init_budget;
    int sched_ss_max_repl;
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