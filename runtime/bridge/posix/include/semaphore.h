#ifndef KZ_POSIX_SEMAPHORE_H_
#define KZ_POSIX_SEMAPHORE_H_

#include <fcntl.h>
#include <stdint.h>
#include <time.h>

typedef struct {
    uint32_t __value;
} sem_t;

#define SEM_FAILED ((sem_t *)-1)
#define SEM_VALUE_MAX 32767

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int sem_clockwait(sem_t *KZ_POSIX_RESTRICT semaphore, clockid_t clock_id,
                  const struct timespec *KZ_POSIX_RESTRICT timeout);
int sem_close(sem_t *semaphore);
int sem_destroy(sem_t *semaphore);
int sem_getvalue(sem_t *KZ_POSIX_RESTRICT semaphore,
                 int *KZ_POSIX_RESTRICT value);
int sem_init(sem_t *semaphore, int process_shared, unsigned int value);
sem_t *sem_open(const char *name, int flags, ...);
int sem_post(sem_t *semaphore);
int sem_timedwait(sem_t *KZ_POSIX_RESTRICT semaphore,
                  const struct timespec *KZ_POSIX_RESTRICT timeout);
int sem_trywait(sem_t *semaphore);
int sem_unlink(const char *name);
int sem_wait(sem_t *semaphore);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif