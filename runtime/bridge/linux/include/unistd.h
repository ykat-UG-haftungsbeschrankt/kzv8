#ifndef KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_UNISTD_H_
#define KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_UNISTD_H_

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <time.h>

#define _SC_PAGESIZE 30
#define _SC_PAGE_SIZE _SC_PAGESIZE
#define _SC_NPROCESSORS_CONF 83
#define _SC_NPROCESSORS_ONLN 84
#define _SC_CLK_TCK 2
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

#ifdef __cplusplus
extern "C" {
#endif

ssize_t write(int file_descriptor, const void* buffer, size_t length);
ssize_t read(int file_descriptor, void* buffer, size_t length);
int close(int file_descriptor);
off_t lseek(int file_descriptor, off_t offset, int origin);
int unlink(const char* path);
char* getcwd(char* buffer, size_t size);
long sysconf(int name);
long syscall(long number, ...);
long getpagesize(void);
int getpid(void);
int usleep(unsigned int microseconds);
int nanosleep(const struct timespec* request, struct timespec* remaining);
int prctl(int option, ...);
void _exit(int status) __attribute__((noreturn));

#ifdef __cplusplus
}
#endif

#endif