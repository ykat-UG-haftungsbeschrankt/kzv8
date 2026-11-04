#ifndef KZ_POSIX_STRINGS_H_
#define KZ_POSIX_STRINGS_H_

#include <locale.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

int ffs(int value);
int ffsl(long value);
int ffsll(long long value);
int strcasecmp(const char *left, const char *right);
int strcasecmp_l(const char *left, const char *right, locale_t locale);
int strncasecmp(const char *left, const char *right, size_t count);
int strncasecmp_l(const char *left, const char *right, size_t count, locale_t locale);

#ifdef __cplusplus
}
#endif

#endif