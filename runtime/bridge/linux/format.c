#include <limits.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include <stdio.h>
#include <string.h>
#include <unistd.h>

struct format_output {
    char* buffer;
    size_t capacity;
    size_t length;
};

static void emit_character(struct format_output* output, char character) {
    if (output->capacity != 0 && output->length < output->capacity - 1) {
        output->buffer[output->length] = character;
    }
    if (output->length < (size_t)INT_MAX) {
        output->length++;
    }
}

static void emit_repeat(struct format_output* output, char character,
                       size_t count) {
    while (count-- != 0) emit_character(output, character);
}

static void emit_text(struct format_output* output, const char* text,
                      size_t length) {
    for (size_t index = 0; index < length; index++) {
        emit_character(output, text[index]);
    }
}

static size_t unsigned_digits(unsigned long long value, unsigned base,
                              int uppercase, char* reversed) {
    const char* digits = uppercase ? "0123456789ABCDEF" : "0123456789abcdef";
    size_t count = 0;
    do {
        reversed[count++] = digits[value % base];
        value /= base;
    } while (value != 0);
    return count;
}

static void emit_number(struct format_output* output,
                        unsigned long long value, int negative,
                        unsigned base, int uppercase, int alternate,
                        int left, int zero, int plus, int space, int precision,
                        int width, int pointer) {
    char reversed[sizeof(value) * CHAR_BIT];
    size_t digit_count = value == 0 && precision == 0
                             ? 0
                             : unsigned_digits(value, base, uppercase, reversed);
    size_t precision_zeroes = precision > 0 && (size_t)precision > digit_count
                                  ? (size_t)precision - digit_count
                                  : 0;
    char sign = negative ? '-' : (plus ? '+' : (space ? ' ' : '\0'));
    char prefix[2];
    size_t prefix_length = 0;
    if (pointer || (alternate && base == 16 && value != 0)) {
        prefix[0] = '0';
        prefix[1] = uppercase ? 'X' : 'x';
        prefix_length = 2;
    } else if (alternate && base == 8 && (digit_count == 0 || reversed[digit_count - 1] != '0')) {
        prefix[0] = '0';
        prefix_length = 1;
    }

    const size_t content_length = digit_count + precision_zeroes + prefix_length +
                                  (sign != '\0');
    const size_t padding = width > 0 && (size_t)width > content_length
                               ? (size_t)width - content_length
                               : 0;
    if (!left && (!zero || precision >= 0)) emit_repeat(output, ' ', padding);
    if (sign != '\0') emit_character(output, sign);
    emit_text(output, prefix, prefix_length);
    if (!left && zero && precision < 0) emit_repeat(output, '0', padding);
    emit_repeat(output, '0', precision_zeroes);
    while (digit_count != 0) emit_character(output, reversed[--digit_count]);
    if (left) emit_repeat(output, ' ', padding);
}

static int format_output(struct format_output* output, const char* format,
                         va_list arguments) {
    if (format == NULL || (output->capacity != 0 && output->buffer == NULL)) {
        return -1;
    }

    while (*format != '\0') {
        if (*format != '%') {
            emit_character(output, *format++);
            continue;
        }
        format++;
        if (*format == '%') {
            emit_character(output, *format++);
            continue;
        }

        int left = 0;
        int zero = 0;
        int plus = 0;
        int space = 0;
        int alternate = 0;
        for (;;) {
            if (*format == '-') left = 1;
            else if (*format == '0') zero = 1;
            else if (*format == '+') plus = 1;
            else if (*format == ' ') space = 1;
            else if (*format == '#') alternate = 1;
            else break;
            format++;
        }

        int width = 0;
        if (*format == '*') {
            width = va_arg(arguments, int);
            format++;
            if (width < 0) {
                left = 1;
                width = -width;
            }
        } else {
            while (*format >= '0' && *format <= '9') {
                if (width < INT_MAX / 10) width = width * 10 + (*format - '0');
                format++;
            }
        }

        int precision = -1;
        if (*format == '.') {
            format++;
            precision = 0;
            if (*format == '*') {
                precision = va_arg(arguments, int);
                format++;
                if (precision < 0) precision = -1;
            } else {
                while (*format >= '0' && *format <= '9') {
                    if (precision < INT_MAX / 10) precision = precision * 10 + (*format - '0');
                    format++;
                }
            }
        }

        enum { LEN_DEFAULT, LEN_LONG, LEN_LONG_LONG, LEN_SIZE } length = LEN_DEFAULT;
        if (*format == 'h') {
            format++;
            if (*format == 'h') format++;
        } else if (*format == 'l') {
            format++;
            length = LEN_LONG;
            if (*format == 'l') {
                format++;
                length = LEN_LONG_LONG;
            }
        } else if (*format == 'z' || *format == 't') {
            format++;
            length = LEN_SIZE;
        }

        const char specifier = *format;
        if (*format != '\0') format++;
        if (specifier == 's') {
            const char* text = va_arg(arguments, const char*);
            if (text == NULL) text = "(null)";
            size_t length = strlen(text);
            if (precision >= 0 && length > (size_t)precision) length = (size_t)precision;
            size_t padding = width > 0 && (size_t)width > length ? (size_t)width - length : 0;
            if (!left) emit_repeat(output, ' ', padding);
            emit_text(output, text, length);
            if (left) emit_repeat(output, ' ', padding);
            continue;
        }
        if (specifier == 'c') {
            const char character = (char)va_arg(arguments, int);
            const size_t padding = width > 1 ? (size_t)width - 1 : 0;
            if (!left) emit_repeat(output, ' ', padding);
            emit_character(output, character);
            if (left) emit_repeat(output, ' ', padding);
            continue;
        }
        if (specifier == 'd' || specifier == 'i') {
            long long value = length == LEN_LONG_LONG ? va_arg(arguments, long long) :
                              (length == LEN_LONG ? va_arg(arguments, long) :
                               (length == LEN_SIZE ? va_arg(arguments, ptrdiff_t) : va_arg(arguments, int)));
            const int negative = value < 0;
            const unsigned long long magnitude = negative
                ? (unsigned long long)(-(value + 1)) + 1
                : (unsigned long long)value;
            emit_number(output, magnitude, negative, 10, 0, 0, left, zero,
                        plus, space, precision, width, 0);
            continue;
        }
        if (specifier == 'u' || specifier == 'o' || specifier == 'x' ||
            specifier == 'X' || specifier == 'p') {
            unsigned long long value;
            if (specifier == 'p') {
                value = (uintptr_t)va_arg(arguments, void*);
            } else if (length == LEN_LONG_LONG) {
                value = va_arg(arguments, unsigned long long);
            } else if (length == LEN_LONG) {
                value = va_arg(arguments, unsigned long);
            } else if (length == LEN_SIZE) {
                value = va_arg(arguments, size_t);
            } else {
                value = va_arg(arguments, unsigned int);
            }
            const unsigned base = specifier == 'o' ? 8 :
                                 (specifier == 'u' ? 10 : 16);
            emit_number(output, value, 0, base, specifier == 'X', alternate,
                        left, zero, 0, 0, precision, width, specifier == 'p');
            continue;
        }
        emit_character(output, '%');
        if (specifier != '\0') emit_character(output, specifier);
    }

    if (output->capacity != 0) {
        const size_t terminator = output->length < output->capacity
                                      ? output->length
                                      : output->capacity - 1;
        output->buffer[terminator] = '\0';
    }
    return output->length > (size_t)INT_MAX ? -1 : (int)output->length;
}

int vsnprintf(char* buffer, size_t size, const char* format, va_list arguments) {
    struct format_output output = {buffer, size, 0};
    va_list copy;
    va_copy(copy, arguments);
    const int result = format_output(&output, format, copy);
    va_end(copy);
    return result;
}

int snprintf(char* buffer, size_t size, const char* format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = vsnprintf(buffer, size, format, arguments);
    va_end(arguments);
    return result;
}

size_t fwrite(const void* buffer, size_t size, size_t count, FILE* stream) {
    if (stream == NULL || (buffer == NULL && size != 0 && count != 0) ||
        (size != 0 && count > SIZE_MAX / size)) return 0;
    const size_t bytes = size * count;
    ssize_t written = write(stream->descriptor, buffer, bytes);
    return written < 0 || size == 0 ? 0 : (size_t)written / size;
}

int vfprintf(FILE* stream, const char* format, va_list arguments) {
    char buffer[1024];
    const int length = vsnprintf(buffer, sizeof(buffer), format, arguments);
    if (length < 0) return -1;
    const size_t count = (size_t)length < sizeof(buffer) ? (size_t)length : sizeof(buffer) - 1;
    return fwrite(buffer, 1, count, stream) == count ? (int)count : -1;
}