/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_STDLIB_H_
#define KZ_POSIX_STDLIB_H_

#include <fcntl.h>
#include <limits.h>
#include <math.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <sys/wait.h>

#define EXIT_FAILURE 1
#define EXIT_SUCCESS 0
#define RAND_MAX 32767
#define MB_CUR_MAX 4

typedef struct { int quot; int rem; } div_t;
typedef struct { long quot; long rem; } ldiv_t;
typedef struct { long long quot; long long rem; } lldiv_t;

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#define KZ_POSIX_NORETURN [[noreturn]]
#else
extern char **environ;
#define KZ_POSIX_RESTRICT restrict
#define KZ_POSIX_NORETURN _Noreturn
#endif

KZ_POSIX_NORETURN void _Exit(int status);
KZ_POSIX_NORETURN void abort(void);
int abs(int value);
void *aligned_alloc(size_t alignment, size_t size);
int at_quick_exit(void (*function)(void));
int atexit(void (*function)(void));
double atof(const char *text);
int atoi(const char *text);
long atol(const char *text);
long long atoll(const char *text);
void *bsearch(const void *key, const void *base, size_t count, size_t size,
              int (*compare)(const void *, const void *));
void *calloc(size_t count, size_t size);
div_t div(int numerator, int denominator);
KZ_POSIX_NORETURN void exit(int status);
void free(void *pointer);
char *getenv(const char *name);
int getsubopt(char **option_pointer, char *const *tokens, char **value);
long labs(long value);
ldiv_t ldiv(long numerator, long denominator);
long long llabs(long long value);
lldiv_t lldiv(long long numerator, long long denominator);
void *malloc(size_t size);
int mblen(const char *input, size_t size);
size_t mbstowcs(wchar_t *KZ_POSIX_RESTRICT output,
                const char *KZ_POSIX_RESTRICT input, size_t count);
int mbtowc(wchar_t *KZ_POSIX_RESTRICT output,
           const char *KZ_POSIX_RESTRICT input, size_t size);
char *mkdtemp(char *template_name);
int mkostemp(char *template_name, int flags);
int mkstemp(char *template_name);
int posix_memalign(void **output, size_t alignment, size_t size);
int posix_openpt(int flags);
int grantpt(int file_descriptor);
int unlockpt(int file_descriptor);
char *ptsname(int file_descriptor);
int ptsname_r(int file_descriptor, char *buffer, size_t buffer_size);
void *realloc(void *pointer, size_t size);
void *reallocarray(void *pointer, size_t count, size_t size);
char *realpath(const char *KZ_POSIX_RESTRICT path, char *KZ_POSIX_RESTRICT resolved_path);
long random(void);
int rand(void);
void qsort(void *base, size_t count, size_t size,
           int (*compare)(const void *, const void *));
void qsort_r(void *base, size_t count, size_t size,
             int (*compare)(const void *, const void *, void *), void *context);
int rand_r(unsigned int *seed);
int setenv(const char *name, const char *value, int overwrite);
char *secure_getenv(const char *name);
void srand(unsigned int seed);
int system(const char *command);
long strtol(const char *KZ_POSIX_RESTRICT text, char **KZ_POSIX_RESTRICT end, int base);
long long strtoll(const char *KZ_POSIX_RESTRICT text,
                  char **KZ_POSIX_RESTRICT end, int base);
unsigned long strtoul(const char *KZ_POSIX_RESTRICT text,
                      char **KZ_POSIX_RESTRICT end, int base);
unsigned long long strtoull(const char *KZ_POSIX_RESTRICT text,
                            char **KZ_POSIX_RESTRICT end, int base);
double strtod(const char *KZ_POSIX_RESTRICT text, char **KZ_POSIX_RESTRICT end);
float strtof(const char *KZ_POSIX_RESTRICT text, char **KZ_POSIX_RESTRICT end);
long double strtold(const char *KZ_POSIX_RESTRICT text, char **KZ_POSIX_RESTRICT end);
int unsetenv(const char *name);
size_t wcstombs(char *KZ_POSIX_RESTRICT output,
                const wchar_t *KZ_POSIX_RESTRICT input, size_t count);
int wctomb(char *output, wchar_t character);

#undef KZ_POSIX_RESTRICT
#undef KZ_POSIX_NORETURN
#ifdef __cplusplus
}
#endif

#endif