#ifndef KZ_POSIX_FCNTL_H_
#define KZ_POSIX_FCNTL_H_

#include <sys/types.h>

#define O_RDONLY 00000000
#define O_WRONLY 00000001
#define O_RDWR 00000002
#define O_ACCMODE 00000003
#define O_CREAT 00000100
#define O_EXCL 00000200
#define O_NOCTTY 00000400
#define O_TRUNC 00001000
#define O_APPEND 00002000
#define O_NONBLOCK 00004000
#define O_DSYNC 00010000
#define O_SYNC 04010000
#define O_RSYNC O_SYNC
#define O_CLOEXEC 02000000
#define O_DIRECTORY 00200000
#define O_NOFOLLOW 00400000
#define O_CLOFORK 010000000
#define O_EXEC 020000000
#define O_SEARCH O_EXEC
#define O_TTY_INIT 0

#define FD_CLOEXEC 1
#define FD_CLOFORK 2

#define F_DUPFD 0
#define F_GETFD 1
#define F_SETFD 2
#define F_GETFL 3
#define F_SETFL 4
#define F_GETLK 5
#define F_SETLK 6
#define F_SETLKW 7
#define F_SETOWN 8
#define F_GETOWN 9
#define F_SETOWN_EX 15
#define F_GETOWN_EX 16
#define F_OFD_GETLK 36
#define F_OFD_SETLK 37
#define F_OFD_SETLKW 38
#define F_DUPFD_CLOEXEC 1030
#define F_DUPFD_CLOFORK 1031

#define F_RDLCK 0
#define F_WRLCK 1
#define F_UNLCK 2
#define F_OWNER_PID 1
#define F_OWNER_PGRP 2

#define AT_FDCWD (-100)
#define AT_EACCESS 0x200
#define AT_SYMLINK_NOFOLLOW 0x100
#define AT_SYMLINK_FOLLOW 0x400
#define AT_REMOVEDIR 0x200

#define POSIX_FADV_NORMAL 0
#define POSIX_FADV_RANDOM 1
#define POSIX_FADV_SEQUENTIAL 2
#define POSIX_FADV_WILLNEED 3
#define POSIX_FADV_DONTNEED 4
#define POSIX_FADV_NOREUSE 5

struct flock {
    short l_type;
    short l_whence;
    off_t l_start;
    off_t l_len;
    pid_t l_pid;
};

struct f_owner_ex {
    int type;
    pid_t pid;
};

#ifdef __cplusplus
extern "C" {
#endif

int creat(const char *path, mode_t mode);
int fcntl(int file_descriptor, int command, ...);
int open(const char *path, int flags, ...);
int openat(int directory_descriptor, const char *path, int flags, ...);
int posix_fadvise(int file_descriptor, off_t offset, off_t length, int advice);
int posix_fallocate(int file_descriptor, off_t offset, off_t length);

#ifdef __cplusplus
}
#endif

#endif