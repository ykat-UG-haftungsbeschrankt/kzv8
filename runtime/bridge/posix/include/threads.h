#ifndef KZ_POSIX_THREADS_H_
#define KZ_POSIX_THREADS_H_

#include <pthread.h>
#include <sched.h>
#include <time.h>

typedef pthread_t thrd_t;
typedef pthread_once_t once_flag;
typedef pthread_key_t tss_t;
typedef pthread_mutex_t mtx_t;
typedef pthread_cond_t cnd_t;
typedef void (*tss_dtor_t)(void *);
typedef int (*thrd_start_t)(void *);

#define ONCE_FLAG_INIT PTHREAD_ONCE_INIT
#define TSS_DTOR_ITERATIONS 4

#define thrd_success 0
#define thrd_busy 1
#define thrd_error 2
#define thrd_nomem 3
#define thrd_timedout 4

#define mtx_plain 0
#define mtx_timed 1
#define mtx_recursive 2

#ifdef __cplusplus
extern "C" {
#endif

int thrd_create(thrd_t *thread, thrd_start_t start_routine, void *argument);
thrd_t thrd_current(void);
int thrd_detach(thrd_t thread);
int thrd_equal(thrd_t left, thrd_t right);
_Noreturn void thrd_exit(int result);
int thrd_join(thrd_t thread, int *result);
int thrd_sleep(const struct timespec *duration, struct timespec *remaining);
void thrd_yield(void);

void call_once(once_flag *flag, void (*function)(void));

int mtx_init(mtx_t *mutex, int type);
int mtx_lock(mtx_t *mutex);
int mtx_timedlock(mtx_t *mutex, const struct timespec *timeout);
int mtx_trylock(mtx_t *mutex);
int mtx_unlock(mtx_t *mutex);
void mtx_destroy(mtx_t *mutex);

int cnd_init(cnd_t *condition);
int cnd_broadcast(cnd_t *condition);
void cnd_destroy(cnd_t *condition);
int cnd_signal(cnd_t *condition);
int cnd_timedwait(cnd_t *condition, mtx_t *mutex, const struct timespec *timeout);
int cnd_wait(cnd_t *condition, mtx_t *mutex);

int tss_create(tss_t *key, tss_dtor_t destructor);
void tss_delete(tss_t key);
void *tss_get(tss_t key);
int tss_set(tss_t key, void *value);

#ifdef __cplusplus
}
#endif

#endif