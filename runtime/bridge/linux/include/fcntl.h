#ifndef KZ_FCNTL_H
#define KZ_FCNTL_H

#include <sys/types.h>

#define O_ACCMODE 00000003
#define O_RDONLY 00000000
#define O_WRONLY 00000001
#define O_RDWR 00000002
#define O_CREAT 00000100
#define O_EXCL 00000200
#define O_NOCTTY 00000400
#define O_TRUNC 00001000
#define O_APPEND 00002000
#define O_NONBLOCK 00004000
#define O_LARGEFILE 00100000
#define O_DIRECTORY 00200000
#define O_NOFOLLOW 00400000
#define O_CLOEXEC 02000000

#ifdef __cplusplus
extern "C" {
#endif

int open(const char* path, int flags, ...);
int open64(const char* path, int flags, ...);
int __open64_2(const char* path, int flags);

#ifdef __cplusplus
}
#endif

#endif