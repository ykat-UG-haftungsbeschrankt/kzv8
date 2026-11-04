#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <wchar.h>

static int unsupported_wide_format(void) {
    errno = ENOSYS;
    return -1;
}

int fwprintf(FILE *stream, const wchar_t *format, ...) {
    (void)stream;
    (void)format;
    return unsupported_wide_format();
}

int fwscanf(FILE *stream, const wchar_t *format, ...) {
    (void)stream;
    (void)format;
    return unsupported_wide_format();
}

int swprintf(wchar_t *output, size_t size, const wchar_t *format, ...) {
    (void)output;
    (void)size;
    (void)format;
    return unsupported_wide_format();
}

int swscanf(const wchar_t *input, const wchar_t *format, ...) {
    (void)input;
    (void)format;
    return unsupported_wide_format();
}

int wprintf(const wchar_t *format, ...) {
    (void)format;
    return unsupported_wide_format();
}

int wscanf(const wchar_t *format, ...) {
    (void)format;
    return unsupported_wide_format();
}

int vfwprintf(FILE *stream, const wchar_t *format, va_list arguments) {
    (void)stream;
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

int vfwscanf(FILE *stream, const wchar_t *format, va_list arguments) {
    (void)stream;
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

int vswprintf(wchar_t *output, size_t size, const wchar_t *format,
              va_list arguments) {
    (void)output;
    (void)size;
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

int vswscanf(const wchar_t *input, const wchar_t *format, va_list arguments) {
    (void)input;
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

int vwprintf(const wchar_t *format, va_list arguments) {
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

int vwscanf(const wchar_t *format, va_list arguments) {
    (void)format;
    (void)arguments;
    return unsupported_wide_format();
}

long double wcstold(const wchar_t *text, wchar_t **end) {
    (void)text;
    (void)end;
    errno = ENOSYS;
    return 0.0L;
}