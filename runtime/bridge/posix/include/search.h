#ifndef KZ_POSIX_SEARCH_H_
#define KZ_POSIX_SEARCH_H_

#include <sys/types.h>

typedef struct {
    char *key;
    void *data;
} ENTRY;

typedef enum { FIND, ENTER } ACTION;
typedef enum { preorder, postorder, endorder, leaf } VISIT;
typedef void posix_tnode;

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int hcreate(size_t table_size);
void hdestroy(void);
ENTRY *hsearch(ENTRY item, ACTION action);
void insque(void *element, void *predecessor);
void *lfind(const void *key, const void *base, size_t *element_count,
            size_t element_size, int (*compare)(const void *, const void *));
void *lsearch(const void *key, void *base, size_t *element_count,
              size_t element_size, int (*compare)(const void *, const void *));
void remque(void *element);
void *tdelete(const void *KZ_POSIX_RESTRICT key,
              posix_tnode **KZ_POSIX_RESTRICT root,
              int (*compare)(const void *, const void *));
posix_tnode *tfind(const void *key, posix_tnode *const *root,
                   int (*compare)(const void *, const void *));
posix_tnode *tsearch(const void *key, posix_tnode **root,
                     int (*compare)(const void *, const void *));
void twalk(const posix_tnode *root,
           void (*action)(const posix_tnode *, VISIT, int));

#ifdef __cplusplus
}
#endif

#endif