#include <errno.h>
#include <mqueue.h>

mqd_t mq_open(const char *name, int flags, ...) {
    (void)name;
    (void)flags;
    errno = ENOSYS;
    return (mqd_t)-1;
}