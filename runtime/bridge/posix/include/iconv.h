#ifndef KZ_POSIX_ICONV_H_
#define KZ_POSIX_ICONV_H_

#include <sys/types.h>

typedef void *iconv_t;

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

size_t iconv(iconv_t conversion,
             char **KZ_POSIX_RESTRICT input, size_t *KZ_POSIX_RESTRICT input_left,
             char **KZ_POSIX_RESTRICT output, size_t *KZ_POSIX_RESTRICT output_left);
int iconv_close(iconv_t conversion);
iconv_t iconv_open(const char *to_code, const char *from_code);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif