#ifndef KZ_POSIX_STDIO_H_
#define KZ_POSIX_STDIO_H_

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

typedef struct __kz_FILE FILE;
typedef int64_t fpos_t;

#define EOF (-1)
#define BUFSIZ 8192
#define FOPEN_MAX 16
#define FILENAME_MAX 4096
#define L_ctermid 9
#define L_tmpnam 20
#define TMP_MAX 25

#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

#ifndef SEEK_SET
#define SEEK_SET 0
#endif
#ifndef SEEK_CUR
#define SEEK_CUR 1
#endif
#ifndef SEEK_END
#define SEEK_END 2
#endif

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

extern FILE *stdin;
extern FILE *stdout;
extern FILE *stderr;

void clearerr(FILE *stream);
char *ctermid(char *buffer);
int dprintf(int file_descriptor, const char *KZ_POSIX_RESTRICT format, ...);
int fclose(FILE *stream);
FILE *fdopen(int file_descriptor, const char *mode);
int feof(FILE *stream);
int ferror(FILE *stream);
int fflush(FILE *stream);
int fgetc(FILE *stream);
int fgetpos(FILE *KZ_POSIX_RESTRICT stream, fpos_t *KZ_POSIX_RESTRICT position);
char *fgets(char *KZ_POSIX_RESTRICT buffer, int size, FILE *KZ_POSIX_RESTRICT stream);
int fileno(FILE *stream);
FILE *fmemopen(void *KZ_POSIX_RESTRICT buffer, size_t size,
               const char *KZ_POSIX_RESTRICT mode);
FILE *fopen(const char *KZ_POSIX_RESTRICT path, const char *KZ_POSIX_RESTRICT mode);
int fprintf(FILE *KZ_POSIX_RESTRICT stream, const char *KZ_POSIX_RESTRICT format, ...);
int fputc(int character, FILE *stream);
int fputs(const char *KZ_POSIX_RESTRICT text, FILE *KZ_POSIX_RESTRICT stream);
size_t fread(void *KZ_POSIX_RESTRICT buffer, size_t size, size_t count,
             FILE *KZ_POSIX_RESTRICT stream);
FILE *freopen(const char *KZ_POSIX_RESTRICT path, const char *KZ_POSIX_RESTRICT mode,
              FILE *KZ_POSIX_RESTRICT stream);
int fscanf(FILE *KZ_POSIX_RESTRICT stream, const char *KZ_POSIX_RESTRICT format, ...);
int fseek(FILE *stream, long offset, int origin);
int fseeko(FILE *stream, off_t offset, int origin);
int fsetpos(FILE *stream, const fpos_t *position);
long ftell(FILE *stream);
off_t ftello(FILE *stream);
size_t fwrite(const void *KZ_POSIX_RESTRICT buffer, size_t size, size_t count,
              FILE *KZ_POSIX_RESTRICT stream);
int getc(FILE *stream);
int getchar(void);
int getc_unlocked(FILE *stream);
int getchar_unlocked(void);
ssize_t getdelim(char **KZ_POSIX_RESTRICT line, size_t *KZ_POSIX_RESTRICT capacity,
                 int delimiter, FILE *KZ_POSIX_RESTRICT stream);
ssize_t getline(char **KZ_POSIX_RESTRICT line, size_t *KZ_POSIX_RESTRICT capacity,
                FILE *KZ_POSIX_RESTRICT stream);
FILE *open_memstream(char **buffer, size_t *size);
int pclose(FILE *stream);
void perror(const char *message);
FILE *popen(const char *command, const char *mode);
int printf(const char *KZ_POSIX_RESTRICT format, ...);
int putc(int character, FILE *stream);
int putchar(int character);
int putc_unlocked(int character, FILE *stream);
int putchar_unlocked(int character);
int puts(const char *text);
int remove(const char *path);
int rename(const char *old_path, const char *new_path);
int renameat(int old_directory, const char *old_path,
             int new_directory, const char *new_path);
void rewind(FILE *stream);
int scanf(const char *KZ_POSIX_RESTRICT format, ...);
void setbuf(FILE *KZ_POSIX_RESTRICT stream, char *KZ_POSIX_RESTRICT buffer);
int setvbuf(FILE *KZ_POSIX_RESTRICT stream, char *KZ_POSIX_RESTRICT buffer,
            int mode, size_t size);
int snprintf(char *KZ_POSIX_RESTRICT buffer, size_t size,
             const char *KZ_POSIX_RESTRICT format, ...);
int sprintf(char *KZ_POSIX_RESTRICT buffer, const char *KZ_POSIX_RESTRICT format, ...);
int sscanf(const char *KZ_POSIX_RESTRICT input, const char *KZ_POSIX_RESTRICT format, ...);
FILE *tmpfile(void);
char *tmpnam(char *buffer);
int ungetc(int character, FILE *stream);
int vdprintf(int file_descriptor, const char *KZ_POSIX_RESTRICT format, va_list arguments);
int vfprintf(FILE *KZ_POSIX_RESTRICT stream, const char *KZ_POSIX_RESTRICT format,
             va_list arguments);
int vfscanf(FILE *KZ_POSIX_RESTRICT stream, const char *KZ_POSIX_RESTRICT format,
            va_list arguments);
int vprintf(const char *KZ_POSIX_RESTRICT format, va_list arguments);
int vscanf(const char *KZ_POSIX_RESTRICT format, va_list arguments);
int vsnprintf(char *KZ_POSIX_RESTRICT buffer, size_t size,
              const char *KZ_POSIX_RESTRICT format, va_list arguments);
int vsprintf(char *KZ_POSIX_RESTRICT buffer, const char *KZ_POSIX_RESTRICT format,
             va_list arguments);
int vsscanf(const char *KZ_POSIX_RESTRICT input, const char *KZ_POSIX_RESTRICT format,
            va_list arguments);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif