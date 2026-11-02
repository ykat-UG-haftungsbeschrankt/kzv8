#ifndef KZ_SYS_STAT_H
#define KZ_SYS_STAT_H

#include <stdint.h>
#include <sys/types.h>
#include <time.h>

#define S_IFMT 00170000
#define S_IFDIR 0040000
#define S_IFCHR 0020000
#define S_IFREG 0100000
#define S_ISDIR(mode) (((mode) & S_IFMT) == S_IFDIR)
#define S_ISCHR(mode) (((mode) & S_IFMT) == S_IFCHR)
#define S_ISREG(mode) (((mode) & S_IFMT) == S_IFREG)

struct stat {
    dev_t st_dev;
    ino_t st_ino;
    nlink_t st_nlink;
    mode_t st_mode;
    uint32_t st_uid;
    uint32_t st_gid;
    uint32_t __pad0;
    dev_t st_rdev;
    off_t st_size;
    int64_t st_blksize;
    int64_t st_blocks;
    struct timespec st_atim;
    struct timespec st_mtim;
    struct timespec st_ctim;
    int64_t __reserved[3];
};

struct stat64 {
    dev_t st_dev;
    ino_t st_ino;
    nlink_t st_nlink;
    mode_t st_mode;
    uint32_t st_uid;
    uint32_t st_gid;
    uint32_t __pad0;
    dev_t st_rdev;
    off_t st_size;
    int64_t st_blksize;
    int64_t st_blocks;
    struct timespec st_atim;
    struct timespec st_mtim;
    struct timespec st_ctim;
    int64_t __reserved[3];
};

#ifdef __cplusplus
extern "C" {
#endif

int fstat(int file_descriptor, struct stat* status);
int fstat64(int file_descriptor, struct stat64* status);
int stat(const char* path, struct stat* status);
int stat64(const char* path, struct stat64* status);
int mkdir(const char* path, mode_t mode);
int rmdir(const char* path);

#ifdef __cplusplus
}
#endif

#endif