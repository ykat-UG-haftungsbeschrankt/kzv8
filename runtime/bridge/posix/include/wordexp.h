#ifndef KZ_POSIX_WORDEXP_H_
#define KZ_POSIX_WORDEXP_H_

#include <stddef.h>

#define WRDE_APPEND 0x01
#define WRDE_DOOFFS 0x02
#define WRDE_NOCMD 0x04
#define WRDE_REUSE 0x08
#define WRDE_SHOWERR 0x10
#define WRDE_UNDEF 0x20
#define WRDE_BADCHAR 1
#define WRDE_BADVAL 2
#define WRDE_CMDSUB 3
#define WRDE_NOSPACE 4
#define WRDE_SYNTAX 5
#define WRDE_NOSYS 6

typedef struct {
    size_t we_wordc;
    char **we_wordv;
    size_t we_offs;
} wordexp_t;

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int wordexp(const char *KZ_POSIX_RESTRICT words,
            wordexp_t *KZ_POSIX_RESTRICT word_expansion, int flags);
void wordfree(wordexp_t *word_expansion);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif