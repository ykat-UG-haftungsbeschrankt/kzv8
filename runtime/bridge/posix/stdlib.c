#include <errno.h>
#include <stdlib.h>

/* FIXME: The runtime has no long double numeric parser. */
long double strtold(const char *text, char **end) {
    (void)text;
    (void)end;
    errno = ENOSYS;
    return 0.0L;
}
