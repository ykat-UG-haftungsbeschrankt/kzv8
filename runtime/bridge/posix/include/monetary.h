#ifndef KZ_POSIX_MONETARY_H_
#define KZ_POSIX_MONETARY_H_

#include <locale.h>
#include <stddef.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

ssize_t strfmon(char *KZ_POSIX_RESTRICT output, size_t output_size,
                const char *KZ_POSIX_RESTRICT format, ...);
ssize_t strfmon_l(char *KZ_POSIX_RESTRICT output, size_t output_size,
                  locale_t locale, const char *KZ_POSIX_RESTRICT format, ...);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif