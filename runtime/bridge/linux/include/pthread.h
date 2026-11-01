#ifndef KZ_PTHREAD_H
#define KZ_PTHREAD_H

#include <stddef.h>
#include <stdint.h>

typedef uintptr_t pthread_t;
typedef unsigned int pthread_key_t;
typedef unsigned int pthread_once_t;
typedef struct {
    size_t stack_size;
    int detach_state;
    void *stack_address;
} pthread_attr_t;
typedef struct {
    uintptr_t owner;
} pthread_mutex_t;
typedef struct {
    uintptr_t generation;
} pthread_cond_t;
typedef struct {
    uintptr_t opaque;
} pthread_mutexattr_t;
typedef struct {
    uintptr_t opaque;
} pthread_condattr_t;

#define PTHREAD_CREATE_JOINABLE 0
#define PTHREAD_CREATE_DETACHED 1
#define PTHREAD_STACK_MIN (16 * 1024)
#define PTHREAD_MUTEX_INITIALIZER { 0 }
#define PTHREAD_COND_INITIALIZER { 0 }
#define PTHREAD_ONCE_INIT 0

#ifndef EAGAIN
#define EAGAIN 11
#endif
#ifndef EBUSY
#define EBUSY 16
#endif
#ifndef EDEADLK
#define EDEADLK 35
#endif
#ifndef EINVAL
#define EINVAL 22
#endif
#ifndef EPERM
#define EPERM 1
#endif
#ifndef ESRCH
#define ESRCH 3
#endif

#ifdef __cplusplus
extern "C" {
#endif

int pthread_attr_init(pthread_attr_t *attributes);
int pthread_attr_destroy(pthread_attr_t *attributes);
int pthread_attr_setdetachstate(pthread_attr_t *attributes, int detach_state);
int pthread_attr_getdetachstate(const pthread_attr_t *attributes, int *detach_state);
int pthread_attr_setstacksize(pthread_attr_t *attributes, size_t stack_size);
int pthread_attr_getstacksize(const pthread_attr_t *attributes, size_t *stack_size);
int pthread_attr_getstack(const pthread_attr_t *attributes, void **stack_address,
                          size_t *stack_size);
int pthread_getattr_np(pthread_t thread, pthread_attr_t *attributes);
int pthread_kill(pthread_t thread, int signal_number);
int pthread_create(pthread_t *thread, const pthread_attr_t *attributes,
                   void *(*start_routine)(void *), void *argument);
int pthread_join(pthread_t thread, void **result);
int pthread_detach(pthread_t thread);
pthread_t pthread_self(void);
int pthread_equal(pthread_t first, pthread_t second);
int pthread_key_create(pthread_key_t *key, void (*destructor)(void *));
int pthread_key_delete(pthread_key_t key);
void *pthread_getspecific(pthread_key_t key);
int pthread_setspecific(pthread_key_t key, const void *value);
int pthread_once(pthread_once_t *once, void (*initializer)(void));
int pthread_getschedparam(pthread_t thread, int *policy, void *parameters);
void pthread_exit(void *result) __attribute__((noreturn));
int pthread_yield(void);
int sched_yield(void);

int pthread_mutex_init(pthread_mutex_t *mutex, const pthread_mutexattr_t *attributes);
int pthread_mutex_destroy(pthread_mutex_t *mutex);
int pthread_mutex_lock(pthread_mutex_t *mutex);
int pthread_mutex_trylock(pthread_mutex_t *mutex);
int pthread_mutex_unlock(pthread_mutex_t *mutex);

int pthread_cond_init(pthread_cond_t *condition, const pthread_condattr_t *attributes);
int pthread_cond_destroy(pthread_cond_t *condition);
int pthread_cond_wait(pthread_cond_t *condition, pthread_mutex_t *mutex);
int pthread_cond_signal(pthread_cond_t *condition);
int pthread_cond_broadcast(pthread_cond_t *condition);

#ifdef __cplusplus
}
#endif

#endif