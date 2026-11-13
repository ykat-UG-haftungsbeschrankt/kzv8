/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_PTHREAD_H_
#define KZ_POSIX_PTHREAD_H_

#include <sched.h>
#include <sys/types.h>
#include <time.h>

#define PTHREAD_BARRIER_SERIAL_THREAD (-1)
#define PTHREAD_CANCEL_ASYNCHRONOUS 1
#define PTHREAD_CANCEL_DEFERRED 0
#define PTHREAD_CANCEL_ENABLE 0
#define PTHREAD_CANCEL_DISABLE 1
#define PTHREAD_CANCELED ((void *)-1)
#define PTHREAD_CREATE_DETACHED 1
#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_EXPLICIT_SCHED 1
#define PTHREAD_INHERIT_SCHED 0
#define PTHREAD_MUTEX_DEFAULT 0
#define PTHREAD_MUTEX_NORMAL 0
#define PTHREAD_MUTEX_ERRORCHECK 2
#define PTHREAD_MUTEX_RECURSIVE 1
#define PTHREAD_MUTEX_ROBUST 1
#define PTHREAD_MUTEX_STALLED 0
#define PTHREAD_ONCE_INIT 0
#define PTHREAD_PRIO_NONE 0
#define PTHREAD_PRIO_INHERIT 1
#define PTHREAD_PRIO_PROTECT 2
#define PTHREAD_PROCESS_PRIVATE 0
#define PTHREAD_PROCESS_SHARED 1
#define PTHREAD_SCOPE_PROCESS 0
#define PTHREAD_SCOPE_SYSTEM 1
#define PTHREAD_STACK_MIN 16384
#define PTHREAD_NULL ((pthread_t)0)

#define PTHREAD_MUTEX_INITIALIZER {{0}}
#define PTHREAD_COND_INITIALIZER {{0}}
#define PTHREAD_RWLOCK_INITIALIZER {{0}}

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int pthread_atfork(void (*prepare)(void), void (*parent)(void), void (*child)(void));
int pthread_attr_destroy(pthread_attr_t *attributes);
int pthread_attr_getdetachstate(const pthread_attr_t *attributes, int *detach_state);
int pthread_attr_getguardsize(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                              size_t *KZ_POSIX_RESTRICT guard_size);
int pthread_attr_getinheritsched(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                                 int *KZ_POSIX_RESTRICT inherit_schedule);
int pthread_attr_getschedparam(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                               struct sched_param *KZ_POSIX_RESTRICT parameters);
int pthread_attr_getschedpolicy(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                                int *KZ_POSIX_RESTRICT policy);
int pthread_attr_getscope(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                          int *KZ_POSIX_RESTRICT scope);
int pthread_attr_getstack(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                          void **KZ_POSIX_RESTRICT stack_address,
                          size_t *KZ_POSIX_RESTRICT stack_size);
int pthread_attr_getstacksize(const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                              size_t *KZ_POSIX_RESTRICT stack_size);
int pthread_attr_init(pthread_attr_t *attributes);
int pthread_attr_setdetachstate(pthread_attr_t *attributes, int detach_state);
int pthread_attr_setguardsize(pthread_attr_t *attributes, size_t guard_size);
int pthread_attr_setinheritsched(pthread_attr_t *attributes, int inherit_schedule);
int pthread_attr_setschedparam(pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                               const struct sched_param *KZ_POSIX_RESTRICT parameters);
int pthread_attr_setschedpolicy(pthread_attr_t *attributes, int policy);
int pthread_attr_setscope(pthread_attr_t *attributes, int scope);
int pthread_attr_setstack(pthread_attr_t *attributes, void *stack_address, size_t stack_size);
int pthread_attr_setstacksize(pthread_attr_t *attributes, size_t stack_size);

int pthread_barrier_destroy(pthread_barrier_t *barrier);
int pthread_barrier_init(pthread_barrier_t *KZ_POSIX_RESTRICT barrier,
                         const pthread_barrierattr_t *KZ_POSIX_RESTRICT attributes,
                         unsigned int count);
int pthread_barrier_wait(pthread_barrier_t *barrier);
int pthread_barrierattr_destroy(pthread_barrierattr_t *attributes);
int pthread_barrierattr_getpshared(const pthread_barrierattr_t *KZ_POSIX_RESTRICT attributes,
                                   int *KZ_POSIX_RESTRICT process_shared);
int pthread_barrierattr_init(pthread_barrierattr_t *attributes);
int pthread_barrierattr_setpshared(pthread_barrierattr_t *attributes, int process_shared);

int pthread_cancel(pthread_t thread);
int pthread_setcancelstate(int state, int *old_state);
int pthread_setcanceltype(int cancel_type, int *old_type);
int pthread_cond_broadcast(pthread_cond_t *condition);
int pthread_cond_clockwait(pthread_cond_t *KZ_POSIX_RESTRICT condition,
                           pthread_mutex_t *KZ_POSIX_RESTRICT mutex, clockid_t clock_id,
                           const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_cond_destroy(pthread_cond_t *condition);
int pthread_cond_init(pthread_cond_t *KZ_POSIX_RESTRICT condition,
                      const pthread_condattr_t *KZ_POSIX_RESTRICT attributes);
int pthread_cond_signal(pthread_cond_t *condition);
int pthread_cond_timedwait(pthread_cond_t *KZ_POSIX_RESTRICT condition,
                           pthread_mutex_t *KZ_POSIX_RESTRICT mutex,
                           const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_cond_wait(pthread_cond_t *KZ_POSIX_RESTRICT condition,
                      pthread_mutex_t *KZ_POSIX_RESTRICT mutex);
int pthread_condattr_destroy(pthread_condattr_t *attributes);
int pthread_condattr_getclock(const pthread_condattr_t *KZ_POSIX_RESTRICT attributes,
                              clockid_t *KZ_POSIX_RESTRICT clock_id);
int pthread_condattr_getpshared(const pthread_condattr_t *KZ_POSIX_RESTRICT attributes,
                                int *KZ_POSIX_RESTRICT process_shared);
int pthread_condattr_init(pthread_condattr_t *attributes);
int pthread_condattr_setclock(pthread_condattr_t *attributes, clockid_t clock_id);
int pthread_condattr_setpshared(pthread_condattr_t *attributes, int process_shared);

int pthread_create(pthread_t *KZ_POSIX_RESTRICT thread,
                   const pthread_attr_t *KZ_POSIX_RESTRICT attributes,
                   void *(*start_routine)(void *), void *KZ_POSIX_RESTRICT argument);
int pthread_detach(pthread_t thread);
int pthread_equal(pthread_t first, pthread_t second);
_Noreturn void pthread_exit(void *result);
int pthread_getcpuclockid(pthread_t thread, clockid_t *clock_id);
int pthread_getschedparam(pthread_t thread, int *KZ_POSIX_RESTRICT policy,
                          struct sched_param *KZ_POSIX_RESTRICT parameters);
pthread_t pthread_self(void);
int pthread_setschedparam(pthread_t thread, int policy,
                          const struct sched_param *parameters);
int pthread_setschedprio(pthread_t thread, int priority);
int pthread_getconcurrency(void);
int pthread_setconcurrency(int concurrency);
int pthread_join(pthread_t thread, void **result);
int pthread_key_create(pthread_key_t *key, void (*destructor)(void *));
int pthread_key_delete(pthread_key_t key);
int pthread_mutex_clocklock(pthread_mutex_t *KZ_POSIX_RESTRICT mutex, clockid_t clock_id,
                            const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_mutex_consistent(pthread_mutex_t *mutex);
int pthread_mutex_destroy(pthread_mutex_t *mutex);
int pthread_mutex_getprioceiling(const pthread_mutex_t *KZ_POSIX_RESTRICT mutex,
                                int *KZ_POSIX_RESTRICT priority_ceiling);
int pthread_mutex_init(pthread_mutex_t *KZ_POSIX_RESTRICT mutex,
                       const pthread_mutexattr_t *KZ_POSIX_RESTRICT attributes);
int pthread_mutex_lock(pthread_mutex_t *mutex);
int pthread_mutex_setprioceiling(pthread_mutex_t *KZ_POSIX_RESTRICT mutex,
                                 int priority_ceiling,
                                 int *KZ_POSIX_RESTRICT old_priority_ceiling);
int pthread_mutex_timedlock(pthread_mutex_t *KZ_POSIX_RESTRICT mutex,
                            const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_mutex_trylock(pthread_mutex_t *mutex);
int pthread_mutex_unlock(pthread_mutex_t *mutex);
int pthread_mutexattr_destroy(pthread_mutexattr_t *attributes);
int pthread_mutexattr_getprioceiling(const pthread_mutexattr_t *KZ_POSIX_RESTRICT attributes,
                                    int *KZ_POSIX_RESTRICT priority_ceiling);
int pthread_mutexattr_getprotocol(const pthread_mutexattr_t *KZ_POSIX_RESTRICT attributes,
                                  int *KZ_POSIX_RESTRICT protocol);
int pthread_mutexattr_getpshared(const pthread_mutexattr_t *KZ_POSIX_RESTRICT attributes,
                                 int *KZ_POSIX_RESTRICT process_shared);
int pthread_mutexattr_getrobust(const pthread_mutexattr_t *attributes, int *robustness);
int pthread_mutexattr_gettype(const pthread_mutexattr_t *KZ_POSIX_RESTRICT attributes,
                             int *KZ_POSIX_RESTRICT type);
int pthread_mutexattr_init(pthread_mutexattr_t *attributes);
int pthread_mutexattr_setprioceiling(pthread_mutexattr_t *attributes, int priority_ceiling);
int pthread_mutexattr_setprotocol(pthread_mutexattr_t *attributes, int protocol);
int pthread_mutexattr_setpshared(pthread_mutexattr_t *attributes, int process_shared);
int pthread_mutexattr_setrobust(pthread_mutexattr_t *attributes, int robustness);
int pthread_mutexattr_settype(pthread_mutexattr_t *attributes, int type);
int pthread_once(pthread_once_t *once, void (*initializer)(void));

int pthread_rwlock_clockrdlock(pthread_rwlock_t *KZ_POSIX_RESTRICT lock, clockid_t clock_id,
                               const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_rwlock_clockwrlock(pthread_rwlock_t *KZ_POSIX_RESTRICT lock, clockid_t clock_id,
                               const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_rwlock_destroy(pthread_rwlock_t *lock);
int pthread_rwlock_init(pthread_rwlock_t *KZ_POSIX_RESTRICT lock,
                        const pthread_rwlockattr_t *KZ_POSIX_RESTRICT attributes);
int pthread_rwlock_rdlock(pthread_rwlock_t *lock);
int pthread_rwlock_timedrdlock(pthread_rwlock_t *KZ_POSIX_RESTRICT lock,
                               const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_rwlock_timedwrlock(pthread_rwlock_t *KZ_POSIX_RESTRICT lock,
                               const struct timespec *KZ_POSIX_RESTRICT timeout);
int pthread_rwlock_tryrdlock(pthread_rwlock_t *lock);
int pthread_rwlock_trywrlock(pthread_rwlock_t *lock);
int pthread_rwlock_unlock(pthread_rwlock_t *lock);
int pthread_rwlock_wrlock(pthread_rwlock_t *lock);
int pthread_rwlockattr_destroy(pthread_rwlockattr_t *attributes);
int pthread_rwlockattr_getpshared(const pthread_rwlockattr_t *KZ_POSIX_RESTRICT attributes,
                                  int *KZ_POSIX_RESTRICT process_shared);
int pthread_rwlockattr_init(pthread_rwlockattr_t *attributes);
int pthread_rwlockattr_setpshared(pthread_rwlockattr_t *attributes, int process_shared);

int pthread_spin_destroy(pthread_spinlock_t *lock);
int pthread_spin_init(pthread_spinlock_t *lock, int process_shared);
int pthread_spin_lock(pthread_spinlock_t *lock);
int pthread_spin_trylock(pthread_spinlock_t *lock);
int pthread_spin_unlock(pthread_spinlock_t *lock);
void pthread_testcancel(void);
void *pthread_getspecific(pthread_key_t key);
int pthread_setspecific(pthread_key_t key, const void *value);
int pthread_yield(void);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif