#ifndef KZ_POSIX_WCHAR_H_
#define KZ_POSIX_WCHAR_H_

#include <locale.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <time.h>

typedef struct {
    uint32_t __opaque[8];
} mbstate_t;
typedef uint32_t wint_t;

#define WEOF ((wint_t)-1)

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

wint_t btowc(int character);
wint_t fgetwc(FILE *stream);
wchar_t *fgetws(wchar_t *KZ_POSIX_RESTRICT output, int size,
                FILE *KZ_POSIX_RESTRICT stream);
wint_t fputwc(wchar_t character, FILE *stream);
int fputws(const wchar_t *KZ_POSIX_RESTRICT text, FILE *KZ_POSIX_RESTRICT stream);
int fwide(FILE *stream, int mode);
int fwprintf(FILE *KZ_POSIX_RESTRICT stream,
             const wchar_t *KZ_POSIX_RESTRICT format, ...);
int fwscanf(FILE *KZ_POSIX_RESTRICT stream,
            const wchar_t *KZ_POSIX_RESTRICT format, ...);
wint_t getwc(FILE *stream);
wint_t getwchar(void);
size_t mbrlen(const char *KZ_POSIX_RESTRICT input, size_t size,
              mbstate_t *KZ_POSIX_RESTRICT state);
size_t mbrtowc(wchar_t *KZ_POSIX_RESTRICT output,
               const char *KZ_POSIX_RESTRICT input, size_t size,
               mbstate_t *KZ_POSIX_RESTRICT state);
size_t mbsnrtowcs(wchar_t *KZ_POSIX_RESTRICT output,
                  const char **KZ_POSIX_RESTRICT input, size_t byte_count,
                  size_t character_count, mbstate_t *KZ_POSIX_RESTRICT state);
size_t mbsrtowcs(wchar_t *KZ_POSIX_RESTRICT output,
                 const char **KZ_POSIX_RESTRICT input, size_t count,
                 mbstate_t *KZ_POSIX_RESTRICT state);
int mbsinit(const mbstate_t *state);
size_t mbsrtowcs(wchar_t *KZ_POSIX_RESTRICT output,
                 const char **KZ_POSIX_RESTRICT input, size_t count,
                 mbstate_t *KZ_POSIX_RESTRICT state);
FILE *open_wmemstream(wchar_t **buffer, size_t *size);
wint_t putwc(wchar_t character, FILE *stream);
wint_t putwchar(wchar_t character);
int swprintf(wchar_t *KZ_POSIX_RESTRICT output, size_t size,
             const wchar_t *KZ_POSIX_RESTRICT format, ...);
int swscanf(const wchar_t *KZ_POSIX_RESTRICT input,
            const wchar_t *KZ_POSIX_RESTRICT format, ...);
wint_t ungetwc(wint_t character, FILE *stream);
int vfwprintf(FILE *KZ_POSIX_RESTRICT stream,
              const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);
int vfwscanf(FILE *KZ_POSIX_RESTRICT stream,
             const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);
int vswprintf(wchar_t *KZ_POSIX_RESTRICT output, size_t size,
              const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);
int vswscanf(const wchar_t *KZ_POSIX_RESTRICT input,
             const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);
int vwprintf(const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);
int vwscanf(const wchar_t *KZ_POSIX_RESTRICT format, va_list arguments);

wchar_t *wcpcpy(wchar_t *KZ_POSIX_RESTRICT destination,
                const wchar_t *KZ_POSIX_RESTRICT source);
wchar_t *wcpncpy(wchar_t *KZ_POSIX_RESTRICT destination,
                 const wchar_t *KZ_POSIX_RESTRICT source, size_t count);
size_t wcrtomb(char *KZ_POSIX_RESTRICT output, wchar_t character,
               mbstate_t *KZ_POSIX_RESTRICT state);
int wcscasecmp(const wchar_t *left, const wchar_t *right);
int wcscasecmp_l(const wchar_t *left, const wchar_t *right, locale_t locale);
wchar_t *wcscat(wchar_t *KZ_POSIX_RESTRICT destination,
                const wchar_t *KZ_POSIX_RESTRICT source);
wchar_t *wcschr(const wchar_t *text, wchar_t character);
int wcscmp(const wchar_t *left, const wchar_t *right);
int wcscoll(const wchar_t *left, const wchar_t *right);
int wcscoll_l(const wchar_t *left, const wchar_t *right, locale_t locale);
wchar_t *wcscpy(wchar_t *KZ_POSIX_RESTRICT destination,
                const wchar_t *KZ_POSIX_RESTRICT source);
size_t wcscspn(const wchar_t *text, const wchar_t *reject);
wchar_t *wcsdup(const wchar_t *text);
size_t wcsftime(wchar_t *KZ_POSIX_RESTRICT output, size_t size,
                const wchar_t *KZ_POSIX_RESTRICT format,
                const struct tm *KZ_POSIX_RESTRICT time_value);
size_t wcslcat(wchar_t *KZ_POSIX_RESTRICT destination,
               const wchar_t *KZ_POSIX_RESTRICT source, size_t size);
size_t wcslcpy(wchar_t *KZ_POSIX_RESTRICT destination,
               const wchar_t *KZ_POSIX_RESTRICT source, size_t size);
size_t wcslen(const wchar_t *text);
int wcsncasecmp(const wchar_t *left, const wchar_t *right, size_t count);
int wcsncasecmp_l(const wchar_t *left, const wchar_t *right, size_t count,
                  locale_t locale);
wchar_t *wcsncat(wchar_t *KZ_POSIX_RESTRICT destination,
                 const wchar_t *KZ_POSIX_RESTRICT source, size_t count);
int wcsncmp(const wchar_t *left, const wchar_t *right, size_t count);
wchar_t *wcsncpy(wchar_t *KZ_POSIX_RESTRICT destination,
                 const wchar_t *KZ_POSIX_RESTRICT source, size_t count);
size_t wcsnlen(const wchar_t *text, size_t maximum_length);
size_t wcsnrtombs(char *KZ_POSIX_RESTRICT output,
                  const wchar_t **KZ_POSIX_RESTRICT input, size_t character_count,
                  size_t byte_count, mbstate_t *KZ_POSIX_RESTRICT state);
wchar_t *wcspbrk(const wchar_t *text, const wchar_t *accept);
wchar_t *wcsrchr(const wchar_t *text, wchar_t character);
size_t wcsrtombs(char *KZ_POSIX_RESTRICT output,
                 const wchar_t **KZ_POSIX_RESTRICT input, size_t byte_count,
                 mbstate_t *KZ_POSIX_RESTRICT state);
size_t wcsspn(const wchar_t *text, const wchar_t *accept);
wchar_t *wcsstr(const wchar_t *KZ_POSIX_RESTRICT haystack,
                const wchar_t *KZ_POSIX_RESTRICT needle);
double wcstod(const wchar_t *KZ_POSIX_RESTRICT text, wchar_t **KZ_POSIX_RESTRICT end);
float wcstof(const wchar_t *KZ_POSIX_RESTRICT text, wchar_t **KZ_POSIX_RESTRICT end);
wchar_t *wcstok(wchar_t *KZ_POSIX_RESTRICT text,
                const wchar_t *KZ_POSIX_RESTRICT delimiters,
                wchar_t **KZ_POSIX_RESTRICT state);
long wcstol(const wchar_t *KZ_POSIX_RESTRICT text, wchar_t **KZ_POSIX_RESTRICT end,
            int base);
long double wcstold(const wchar_t *KZ_POSIX_RESTRICT text,
                    wchar_t **KZ_POSIX_RESTRICT end);
long long wcstoll(const wchar_t *KZ_POSIX_RESTRICT text,
                  wchar_t **KZ_POSIX_RESTRICT end, int base);
unsigned long wcstoul(const wchar_t *KZ_POSIX_RESTRICT text,
                      wchar_t **KZ_POSIX_RESTRICT end, int base);
unsigned long long wcstoull(const wchar_t *KZ_POSIX_RESTRICT text,
                            wchar_t **KZ_POSIX_RESTRICT end, int base);
int wcswidth(const wchar_t *text, size_t count);
size_t wcsxfrm(wchar_t *KZ_POSIX_RESTRICT destination,
               const wchar_t *KZ_POSIX_RESTRICT source, size_t count);
size_t wcsxfrm_l(wchar_t *KZ_POSIX_RESTRICT destination,
                 const wchar_t *KZ_POSIX_RESTRICT source, size_t count,
                 locale_t locale);
int wctob(wint_t character);
int wcwidth(wchar_t character);
wchar_t *wmemchr(const wchar_t *memory, wchar_t character, size_t count);
int wmemcmp(const wchar_t *left, const wchar_t *right, size_t count);
wchar_t *wmemcpy(wchar_t *KZ_POSIX_RESTRICT destination,
                 const wchar_t *KZ_POSIX_RESTRICT source, size_t count);
wchar_t *wmemmove(wchar_t *destination, const wchar_t *source, size_t count);
wchar_t *wmemset(wchar_t *memory, wchar_t value, size_t count);
int wprintf(const wchar_t *KZ_POSIX_RESTRICT format, ...);
int wscanf(const wchar_t *KZ_POSIX_RESTRICT format, ...);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif