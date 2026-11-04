#ifndef KZ_POSIX_UCHAR_H_
#define KZ_POSIX_UCHAR_H_

#include <stddef.h>
#include <stdint.h>
#include <wchar.h>

#ifndef __cplusplus
typedef uint_least16_t char16_t;
typedef uint_least32_t char32_t;
#endif

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

size_t c16rtomb(char *KZ_POSIX_RESTRICT output, char16_t character,
                mbstate_t *KZ_POSIX_RESTRICT state);
size_t c32rtomb(char *KZ_POSIX_RESTRICT output, char32_t character,
                mbstate_t *KZ_POSIX_RESTRICT state);
size_t mbrtoc16(char16_t *KZ_POSIX_RESTRICT output,
                const char *KZ_POSIX_RESTRICT input, size_t size,
                mbstate_t *KZ_POSIX_RESTRICT state);
size_t mbrtoc32(char32_t *KZ_POSIX_RESTRICT output,
                const char *KZ_POSIX_RESTRICT input, size_t size,
                mbstate_t *KZ_POSIX_RESTRICT state);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif