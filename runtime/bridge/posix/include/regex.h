#ifndef KZ_POSIX_REGEX_H_
#define KZ_POSIX_REGEX_H_

#include <stddef.h>
#include <sys/types.h>

typedef intptr_t regoff_t;

typedef struct {
    size_t re_nsub;
    void *__opaque[8];
} regex_t;

typedef struct {
    regoff_t rm_so;
    regoff_t rm_eo;
} regmatch_t;

#define REG_EXTENDED 0x01
#define REG_ICASE 0x02
#define REG_NOSUB 0x04
#define REG_NEWLINE 0x08
#define REG_MINIMAL 0x10

#define REG_NOTBOL 0x01
#define REG_NOTEOL 0x02

#define REG_NOMATCH 1
#define REG_BADPAT 2
#define REG_ECOLLATE 3
#define REG_ECTYPE 4
#define REG_EESCAPE 5
#define REG_ESUBREG 6
#define REG_EBRACK 7
#define REG_EPAREN 8
#define REG_EBRACE 9
#define REG_BADBR 10
#define REG_ERANGE 11
#define REG_ESPACE 12
#define REG_BADRPT 13
#define REG_ENOSYS 14

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int regcomp(regex_t *KZ_POSIX_RESTRICT expression,
            const char *KZ_POSIX_RESTRICT pattern, int flags);
size_t regerror(int error_code, const regex_t *KZ_POSIX_RESTRICT expression,
                char *KZ_POSIX_RESTRICT buffer, size_t buffer_size);
int regexec(const regex_t *KZ_POSIX_RESTRICT expression,
            const char *KZ_POSIX_RESTRICT string, size_t match_count,
            regmatch_t matches[restrict], int flags);
void regfree(regex_t *expression);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif