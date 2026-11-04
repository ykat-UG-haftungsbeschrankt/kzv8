#include <errno.h>
#include <unistd.h>

int execl(const char *path, const char *argument, ...) {
    (void)path;
    (void)argument;
    errno = ENOSYS;
    return -1;
}

int execle(const char *path, const char *argument, ...) {
    (void)path;
    (void)argument;
    errno = ENOSYS;
    return -1;
}

int execlp(const char *file, const char *argument, ...) {
    (void)file;
    (void)argument;
    errno = ENOSYS;
    return -1;
}
