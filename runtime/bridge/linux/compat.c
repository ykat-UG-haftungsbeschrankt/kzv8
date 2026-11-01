#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <time.h>
#include <errno.h>

#define ERANGE 34
#define EINVAL 22
#define ENOSYS 38
#define EBADF 9
#define EFAULT 14
#define WEOF ((wchar_t)-1)
#define CAT_FD_MAX 32

struct LocaleStub {
    int active;
};

static struct LocaleStub c_locale = {1};
static int lower_table_initialized;
static int lower_table_storage[384];
static const int* lower_table = &lower_table_storage[128];

const int** __ctype_tolower_loc(void) {
    if (!lower_table_initialized) {
        for (int value = 0; value < 256; value++) {
            lower_table_storage[value + 128] =
                value >= 'A' && value <= 'Z' ? value + ('a' - 'A') : value;
        }
        lower_table_initialized = 1;
    }
    return &lower_table;
}

size_t __ctype_get_mb_cur_max(void) { return 4; }

void* newlocale(int category_mask, const char* locale_name, void* base) {
    (void)category_mask;
    (void)base;
    if (locale_name == NULL || strcmp(locale_name, "C") == 0 ||
        strcmp(locale_name, "POSIX") == 0 || strcmp(locale_name, "C.UTF-8") == 0 ||
        strcmp(locale_name, "C.utf8") == 0) {
        return &c_locale;
    }
    errno = EINVAL;
    return NULL;
}

void freelocale(void* locale) { (void)locale; }
void* uselocale(void* locale) { return locale == NULL ? &c_locale : locale; }

size_t mbrtowc(wchar_t* output, const char* input, size_t length, void* state) {
    (void)state;
    if (input == NULL) return 0;
    if (length == 0) return (size_t)-2;
    const unsigned char lead = (unsigned char)input[0];
    if (lead == 0) {
        if (output != NULL) *output = 0;
        return 0;
    }
    size_t bytes = lead < 0x80 ? 1 :
                   ((lead & 0xe0) == 0xc0 ? 2 :
                    ((lead & 0xf0) == 0xe0 ? 3 :
                     ((lead & 0xf8) == 0xf0 ? 4 : 0)));
    if (bytes == 0) return (size_t)-1;
    if (length < bytes) return (size_t)-2;
    uint32_t codepoint = bytes == 1 ? lead :
                         (bytes == 2 ? lead & 0x1f :
                          (bytes == 3 ? lead & 0x0f : lead & 0x07));
    for (size_t index = 1; index < bytes; index++) {
        const unsigned char continuation = (unsigned char)input[index];
        if ((continuation & 0xc0) != 0x80) return (size_t)-1;
        codepoint = (codepoint << 6) | (continuation & 0x3f);
    }
    if ((bytes == 2 && codepoint < 0x80) ||
        (bytes == 3 && codepoint < 0x800) ||
        (bytes == 4 && codepoint < 0x10000) || codepoint > 0x10ffff ||
        (codepoint >= 0xd800 && codepoint <= 0xdfff)) {
        return (size_t)-1;
    }
    if (output != NULL) *output = (wchar_t)codepoint;
    return bytes;
}

size_t mbrlen(const char* input, size_t length, void* state) {
    return mbrtowc(NULL, input, length, state);
}

int mbtowc(wchar_t* output, const char* input, size_t length) {
    const size_t result = mbrtowc(output, input, length, NULL);
    return result == (size_t)-1 || result == (size_t)-2 ? -1 : (int)result;
}

int mblen(const char* input, size_t length) {
    return mbtowc(NULL, input, length);
}

size_t wcrtomb(char* output, wchar_t input, void* state) {
    (void)state;
    if (output == NULL) return 1;
    const uint32_t codepoint = (uint32_t)input;
    if (codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) {
        return (size_t)-1;
    }
    if (codepoint < 0x80) {
        output[0] = (char)codepoint;
        return 1;
    }
    if (codepoint < 0x800) {
        output[0] = (char)(0xc0 | (codepoint >> 6));
        output[1] = (char)(0x80 | (codepoint & 0x3f));
        return 2;
    }
    if (codepoint < 0x10000) {
        output[0] = (char)(0xe0 | (codepoint >> 12));
        output[1] = (char)(0x80 | ((codepoint >> 6) & 0x3f));
        output[2] = (char)(0x80 | (codepoint & 0x3f));
        return 3;
    }
    output[0] = (char)(0xf0 | (codepoint >> 18));
    output[1] = (char)(0x80 | ((codepoint >> 12) & 0x3f));
    output[2] = (char)(0x80 | ((codepoint >> 6) & 0x3f));
    output[3] = (char)(0x80 | (codepoint & 0x3f));
    return 4;
}

size_t mbsrtowcs(wchar_t* output, const char** input, size_t capacity,
                 void* state) {
    if (input == NULL || *input == NULL) return 0;
    const char* cursor = *input;
    size_t converted = 0;
    while (*cursor != '\0') {
        wchar_t value;
        const size_t amount = mbrtowc(&value, cursor, 4, state);
        if (amount == (size_t)-1 || amount == (size_t)-2) return (size_t)-1;
        if (output != NULL) {
            if (converted == capacity) {
                *input = cursor;
                return converted;
            }
            output[converted] = value;
        }
        cursor += amount;
        converted++;
    }
    if (output != NULL) {
        if (converted < capacity) output[converted] = 0;
        *input = NULL;
    }
    return converted;
}

size_t mbsnrtowcs(wchar_t* output, const char** input, size_t byte_count,
                  size_t capacity, void* state) {
    if (output == NULL) return mbsrtowcs(NULL, input, capacity, state);
    if (input == NULL || *input == NULL) return 0;
    const char* cursor = *input;
    size_t consumed = 0;
    size_t converted = 0;
    while (consumed < byte_count && *cursor != '\0' && converted < capacity) {
        wchar_t value;
        const size_t amount = mbrtowc(&value, cursor, byte_count - consumed, state);
        if (amount == (size_t)-1 || amount == (size_t)-2) return (size_t)-1;
        output[converted++] = value;
        cursor += amount;
        consumed += amount;
    }
    *input = *cursor == '\0' ? NULL : cursor;
    if (*input == NULL && converted < capacity) output[converted] = 0;
    return converted;
}

size_t wcsnrtombs(char* output, const wchar_t** input, size_t byte_capacity,
                  size_t character_count, void* state) {
    if (input == NULL || *input == NULL) return 0;
    const wchar_t* cursor = *input;
    size_t bytes_written = 0;
    size_t characters = 0;
    while (characters < character_count && *cursor != 0) {
        char encoded[4];
        const size_t amount = wcrtomb(encoded, *cursor, state);
        if (amount == (size_t)-1) return (size_t)-1;
        if (output != NULL && amount > byte_capacity - bytes_written) {
            *input = cursor;
            return bytes_written;
        }
        if (output != NULL) memcpy(output + bytes_written, encoded, amount);
        bytes_written += amount;
        cursor++;
        characters++;
    }
    if (output != NULL) {
        if (*cursor == 0) *input = NULL;
        else *input = cursor;
    }
    return bytes_written;
}

unsigned int getwc(FILE* stream) {
    (void)stream;
    return (unsigned int)WEOF;
}

unsigned int ungetwc(wchar_t character, FILE* stream) {
    (void)character;
    (void)stream;
    return (unsigned int)WEOF;
}

unsigned int fputwc(wchar_t character, FILE* stream) {
    char encoded[4];
    const size_t length = wcrtomb(encoded, character, NULL);
    if (length == (size_t)-1 || fwrite(encoded, 1, length, stream) != length) {
        return WEOF;
    }
    return character;
}

int setbuf(FILE* stream, char* buffer) {
    return setvbuf(stream, buffer, _IOFBF, 0);
}

int fseek(FILE* stream, long offset, int origin) {
    (void)stream;
    (void)offset;
    (void)origin;
    errno = ENOSYS;
    return -1;
}

int fseeko64(FILE* stream, int64_t offset, int origin) {
    (void)stream;
    (void)offset;
    (void)origin;
    errno = ENOSYS;
    return -1;
}

int64_t ftello64(FILE* stream) {
    (void)stream;
    errno = ENOSYS;
    return -1;
}

int catopen(const char* name, int flag) {
    (void)name;
    (void)flag;
    return -1;
}

char* catgets(int catalog, int set_number, int message, const char* text) {
    (void)catalog;
    (void)set_number;
    (void)message;
    return (char*)text;
}

int catclose(int catalog) {
    (void)catalog;
    return 0;
}

size_t strftime_l(char* output, size_t capacity, const char* format,
                  const struct tm* time, void* locale) {
    (void)format;
    (void)time;
    (void)locale;
    if (capacity != 0 && output != NULL) output[0] = '\0';
    return 0;
}

int __isoc23_sscanf(const char* input, const char* format, ...) {
    (void)input;
    (void)format;
    return 0;
}