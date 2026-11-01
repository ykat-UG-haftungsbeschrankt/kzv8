#ifndef KZ_STDLIB_H
#define KZ_STDLIB_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void *malloc(size_t size);
void *aligned_alloc(size_t alignment, size_t size);
int posix_memalign(void **output, size_t alignment, size_t size);
void free(void *pointer);

#ifdef __cplusplus
}
#endif

#endif