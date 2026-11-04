#include <errno.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern char *strerror(int error_number);

#if defined(__x86_64__)
#define KZ_VA_LIST_PTR(arguments) ((void *)(arguments))
#else
#define KZ_VA_LIST_PTR(arguments) ((void *)&(arguments))
#endif

extern int kz_rust_vsnprintf(char *buffer, size_t size, const char *format,
                             void *arguments);
extern int kz_rust_vfprintf(FILE *stream, const char *format, void *arguments);
extern int kz_rust_vasprintf(char **output, const char *format, void *arguments);
extern int kz_rust_sscanf(const char *input, const char *format, void *arguments);

int fprintf(FILE *restrict stream, const char *restrict format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_vfprintf(stream, format, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

int sprintf(char *restrict buffer, const char *restrict format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_vsnprintf(buffer, SIZE_MAX, format,
                                         KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

int vsprintf(char *restrict buffer, const char *restrict format,
             va_list arguments) {
    return kz_rust_vsnprintf(buffer, SIZE_MAX, format, KZ_VA_LIST_PTR(arguments));
}

int sscanf(const char *restrict input, const char *restrict format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_sscanf(input, format, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

int vsscanf(const char *restrict input, const char *restrict format,
            va_list arguments) {
    return kz_rust_sscanf(input, format, KZ_VA_LIST_PTR(arguments));
}

int fscanf(FILE *restrict stream, const char *restrict format, ...) {
    (void)stream;
    (void)format;
    errno = ENOSYS;
    return EOF;
}

int vfscanf(FILE *restrict stream, const char *restrict format,
            va_list arguments) {
    (void)stream;
    (void)format;
    (void)arguments;
    errno = ENOSYS;
    return EOF;
}

int scanf(const char *restrict format, ...) {
    (void)format;
    errno = ENOSYS;
    return EOF;
}

int vscanf(const char *restrict format, va_list arguments) {
    (void)format;
    (void)arguments;
    errno = ENOSYS;
    return EOF;
}

int vdprintf(int file_descriptor, const char *restrict format, va_list arguments) {
    char *buffer = NULL;
    const int length = kz_rust_vasprintf(&buffer, format, KZ_VA_LIST_PTR(arguments));
    if (length < 0) {
        return -1;
    }
    size_t written = 0;
    while (written < (size_t)length) {
        const ssize_t amount = write(file_descriptor, buffer + written,
                                     (size_t)length - written);
        if (amount <= 0) {
            free(buffer);
            return -1;
        }
        written += (size_t)amount;
    }
    free(buffer);
    return length;
}

int dprintf(int file_descriptor, const char *restrict format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = vdprintf(file_descriptor, format, arguments);
    va_end(arguments);
    return result;
}

void perror(const char *message) {
    const char *error_message = strerror(errno);
    if (message != NULL && message[0] != '\0') {
        (void)fprintf(stderr, "%s: %s\n", message, error_message);
    } else {
        (void)fprintf(stderr, "%s\n", error_message);
    }
}