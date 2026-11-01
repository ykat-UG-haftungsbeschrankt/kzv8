#ifndef KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_STDIO_H_
#define KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_STDIO_H_

#include <stddef.h>
#include <stdarg.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EOF (-1)

typedef struct kz_file {
	int descriptor;
} FILE;
extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

int putchar(int character);
int puts(const char* text);
int printf(const char* format, ...);
int snprintf(char* buffer, size_t size, const char* format, ...);
int fclose(FILE* stream);
int fflush(FILE* stream);
int fputc(int character, FILE* stream);
int fputs(const char* text, FILE* stream);
size_t fread(void* buffer, size_t size, size_t count, FILE* stream);
size_t fwrite(const void* buffer, size_t size, size_t count, FILE* stream);
int fileno(FILE* stream);
int vfprintf(FILE* stream, const char* format, va_list arguments);
int snprintf(char* buffer, size_t size, const char* format, ...);
FILE* fopen64(const char* path, const char* mode);

#ifdef __cplusplus
}
#endif

#endif