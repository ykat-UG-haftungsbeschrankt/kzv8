#ifndef KZ_POSIX_STRING_H_
#define KZ_POSIX_STRING_H_

#include <locale.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

void *memccpy(void *KZ_POSIX_RESTRICT destination,
              const void *KZ_POSIX_RESTRICT source, int character, size_t count);
void *memchr(const void *memory, int character, size_t count);
int memcmp(const void *left, const void *right, size_t count);
void *memcpy(void *KZ_POSIX_RESTRICT destination,
             const void *KZ_POSIX_RESTRICT source, size_t count);
void *memmem(const void *haystack, size_t haystack_size,
             const void *needle, size_t needle_size);
void *memmove(void *destination, const void *source, size_t count);
void *memset(void *destination, int value, size_t count);
char *stpcpy(char *KZ_POSIX_RESTRICT destination,
             const char *KZ_POSIX_RESTRICT source);
char *stpncpy(char *KZ_POSIX_RESTRICT destination,
              const char *KZ_POSIX_RESTRICT source, size_t count);
char *strcat(char *KZ_POSIX_RESTRICT destination,
             const char *KZ_POSIX_RESTRICT source);
char *strchr(const char *text, int character);
int strcmp(const char *left, const char *right);
int strcoll(const char *left, const char *right);
int strcoll_l(const char *left, const char *right, locale_t locale);
char *strcpy(char *KZ_POSIX_RESTRICT destination,
             const char *KZ_POSIX_RESTRICT source);
size_t strcspn(const char *text, const char *reject);
char *strdup(const char *text);
char *strerror(int error_number);
char *strerror_l(int error_number, locale_t locale);
int strerror_r(int error_number, char *buffer, size_t buffer_size);
size_t strlen(const char *text);
char *strncat(char *KZ_POSIX_RESTRICT destination,
              const char *KZ_POSIX_RESTRICT source, size_t count);
int strncmp(const char *left, const char *right, size_t count);
char *strncpy(char *KZ_POSIX_RESTRICT destination,
              const char *KZ_POSIX_RESTRICT source, size_t count);
char *strndup(const char *text, size_t count);
size_t strnlen(const char *text, size_t maximum_length);
char *strpbrk(const char *text, const char *accept);
char *strrchr(const char *text, int character);
char *strsignal(int signal_number);
size_t strspn(const char *text, const char *accept);
char *strstr(const char *haystack, const char *needle);
char *strtok(char *KZ_POSIX_RESTRICT text, const char *KZ_POSIX_RESTRICT delimiters);
char *strtok_r(char *KZ_POSIX_RESTRICT text, const char *KZ_POSIX_RESTRICT delimiters,
               char **KZ_POSIX_RESTRICT state);
size_t strxfrm(char *KZ_POSIX_RESTRICT destination,
               const char *KZ_POSIX_RESTRICT source, size_t destination_size);
size_t strxfrm_l(char *KZ_POSIX_RESTRICT destination,
                 const char *KZ_POSIX_RESTRICT source, size_t destination_size,
                 locale_t locale);
size_t strlcat(char *KZ_POSIX_RESTRICT destination,
               const char *KZ_POSIX_RESTRICT source, size_t destination_size);
size_t strlcpy(char *KZ_POSIX_RESTRICT destination,
               const char *KZ_POSIX_RESTRICT source, size_t destination_size);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif