#include <errno.h>
#include <sys/sem.h>

int semctl(int semaphore_id, int semaphore_number, int command, ...) {
    (void)semaphore_id;
    (void)semaphore_number;
    (void)command;
    errno = ENOSYS;
    return -1;
}