#include <errno.h>
#include <semaphore.h>

sem_t *sem_open(const char *name, int flags, ...) {
    (void)name;
    (void)flags;
    errno = ENOSYS;
    return SEM_FAILED;
}