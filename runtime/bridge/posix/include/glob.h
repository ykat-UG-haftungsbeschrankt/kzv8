#ifndef KZ_POSIX_GLOB_H_
#define KZ_POSIX_GLOB_H_

#include <sys/types.h>

typedef struct {
    size_t gl_pathc;
    char **gl_pathv;
    size_t gl_offs;
} glob_t;

#define GLOB_APPEND 0x01
#define GLOB_DOOFFS 0x02
#define GLOB_ERR 0x04
#define GLOB_MARK 0x08
#define GLOB_NOCHECK 0x10
#define GLOB_NOESCAPE 0x20
#define GLOB_NOSORT 0x40

#define GLOB_ABORTED 1
#define GLOB_NOMATCH 2
#define GLOB_NOSPACE 3

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int glob(const char *KZ_POSIX_RESTRICT pattern, int flags,
         int (*error_function)(const char *path, int error),
         glob_t *KZ_POSIX_RESTRICT results);
void globfree(glob_t *results);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif