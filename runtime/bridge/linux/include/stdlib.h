#ifndef KZ_STDLIB_H
#define KZ_STDLIB_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

void *malloc(size_t size);
void *aligned_alloc(size_t alignment, size_t size);
int posix_memalign(void **output, size_t alignment, size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *pointer, size_t size);
void free(void *pointer);
size_t malloc_usable_size(void *pointer);
char *getenv(const char *name);
unsigned long strtoul(const char *text, char **end, int base);
unsigned long long strtoull(const char *text, char **end, int base);
double strtod(const char *text, char **end);
float strtof(const char *text, char **end);

#ifdef __cplusplus
}
#endif

#endif