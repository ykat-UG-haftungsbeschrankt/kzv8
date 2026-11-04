#include <errno.h>
#include <monetary.h>

#include <stddef.h>

static ssize_t unsupported(char *output, size_t output_size) {
    if (output != NULL && output_size != 0) {
        output[0] = '\0';
    }
    errno = ENOSYS;
    return -1;
}

ssize_t strfmon(char *restrict output, size_t output_size,
                const char *restrict format, ...) {
    (void)format;
    return unsupported(output, output_size);
}

ssize_t strfmon_l(char *restrict output, size_t output_size, locale_t locale,
                  const char *restrict format, ...) {
    (void)locale;
    (void)format;
    return unsupported(output, output_size);
}