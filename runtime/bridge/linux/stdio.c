#include <limits.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include <stdio.h>

#define OUTPUT_BUFFER_SIZE 512

struct output_buffer {
    char bytes[OUTPUT_BUFFER_SIZE];
    size_t length;
    size_t total;
    int failed;
};

extern int kz_host_console_write(const char* data, size_t length);

static int flush_output(struct output_buffer* output) {
    if (output->length == 0) {
        return 0;
    }
    if (kz_host_console_write(output->bytes, output->length) !=
        (int)output->length) {
        output->failed = 1;
        return -1;
    }
    output->length = 0;
    return 0;
}

static int emit_character(struct output_buffer* output, char character) {
    if (output->total >= INT_MAX) {
        output->failed = 1;
        return -1;
    }
    if (output->length == OUTPUT_BUFFER_SIZE && flush_output(output) != 0) {
        return -1;
    }
    output->bytes[output->length++] = character;
    output->total++;
    return 0;
}

static int emit_string(struct output_buffer* output, const char* text) {
    if (text == NULL) {
        text = "(null)";
    }
    while (*text != '\0') {
        if (emit_character(output, *text++) != 0) {
            return -1;
        }
    }
    return 0;
}

static int emit_unsigned(struct output_buffer* output,
                         unsigned long long value, unsigned base,
                         int uppercase) {
    const char* digits = uppercase ? "0123456789ABCDEF" : "0123456789abcdef";
    char reversed[sizeof(value) * CHAR_BIT];
    size_t count = 0;

    do {
        reversed[count++] = digits[value % base];
        value /= base;
    } while (value != 0);

    while (count != 0) {
        if (emit_character(output, reversed[--count]) != 0) {
            return -1;
        }
    }
    return 0;
}

static int emit_signed(struct output_buffer* output, long long value) {
    unsigned long long magnitude;
    if (value < 0) {
        if (emit_character(output, '-') != 0) {
            return -1;
        }
        magnitude = (unsigned long long)(-(value + 1)) + 1;
    } else {
        magnitude = (unsigned long long)value;
    }
    return emit_unsigned(output, magnitude, 10, 0);
}

int putchar(int character) {
    const char byte = (char)(unsigned char)character;
    return kz_host_console_write(&byte, 1) == 1 ? (unsigned char)byte : EOF;
}

int puts(const char* text) {
    if (text == NULL) {
        return EOF;
    }

    struct output_buffer output = {{0}, 0, 0, 0};
    if (emit_string(&output, text) != 0 ||
        emit_character(&output, '\n') != 0 || flush_output(&output) != 0 ||
        output.failed) {
        return EOF;
    }
    return (int)output.total;
}

int printf(const char* format, ...) {
    if (format == NULL) {
        return EOF;
    }

    struct output_buffer output = {{0}, 0, 0, 0};
    va_list arguments;
    va_start(arguments, format);

    while (*format != '\0' && !output.failed) {
        if (*format != '%') {
            if (emit_character(&output, *format++) != 0) {
                break;
            }
            continue;
        }

        format++;
        if (*format == '%') {
            emit_character(&output, *format++);
            continue;
        }

        enum { LENGTH_DEFAULT, LENGTH_LONG, LENGTH_LONG_LONG, LENGTH_SIZE };
        int length = LENGTH_DEFAULT;
        if (*format == 'h') {
            format++;
            if (*format == 'h') {
                format++;
            }
        } else if (*format == 'l') {
            format++;
            length = LENGTH_LONG;
            if (*format == 'l') {
                format++;
                length = LENGTH_LONG_LONG;
            }
        } else if (*format == 'z') {
            format++;
            length = LENGTH_SIZE;
        }

        const char specifier = *format;
        if (specifier != '\0') {
            format++;
        }

        switch (specifier) {
            case 'c':
                emit_character(&output, (char)va_arg(arguments, int));
                break;
            case 's':
                emit_string(&output, va_arg(arguments, const char*));
                break;
            case 'd':
            case 'i':
                if (length == LENGTH_LONG_LONG) {
                    emit_signed(&output, va_arg(arguments, long long));
                } else if (length == LENGTH_LONG) {
                    emit_signed(&output, va_arg(arguments, long));
                } else if (length == LENGTH_SIZE) {
                    emit_signed(&output, va_arg(arguments, ptrdiff_t));
                } else {
                    emit_signed(&output, va_arg(arguments, int));
                }
                break;
            case 'u':
            case 'o':
            case 'x':
            case 'X': {
                unsigned long long value;
                if (length == LENGTH_LONG_LONG) {
                    value = va_arg(arguments, unsigned long long);
                } else if (length == LENGTH_LONG) {
                    value = va_arg(arguments, unsigned long);
                } else if (length == LENGTH_SIZE) {
                    value = va_arg(arguments, size_t);
                } else {
                    value = va_arg(arguments, unsigned int);
                }
                const unsigned base = specifier == 'o' ? 8 :
                                      (specifier == 'u' ? 10 : 16);
                emit_unsigned(&output, value, base, specifier == 'X');
                break;
            }
            case 'p':
                emit_string(&output, "0x");
                emit_unsigned(&output,
                              (unsigned long long)(uintptr_t)va_arg(arguments, void*),
                              16, 0);
                break;
            case '\0':
                emit_character(&output, '%');
                break;
            default:
                emit_character(&output, '%');
                emit_character(&output, specifier);
                break;
        }
    }

    va_end(arguments);
    if (flush_output(&output) != 0 || output.failed) {
        return EOF;
    }
    return (int)output.total;
}