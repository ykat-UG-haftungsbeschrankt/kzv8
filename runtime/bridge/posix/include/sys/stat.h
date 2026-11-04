#ifndef KZ_POSIX_SYS_STAT_H_
#define KZ_POSIX_SYS_STAT_H_

#include <sys/types.h>
#include <time.h>

#define S_IFMT 00170000
#define S_IFIFO 0010000
#define S_IFCHR 0020000
#define S_IFDIR 0040000
#define S_IFBLK 0060000
#define S_IFREG 0100000
#define S_IFLNK 0120000
#define S_IFSOCK 0140000

#define S_IRWXU 0000700
#define S_IRUSR 0000400
#define S_IWUSR 0000200
#define S_IXUSR 0000100
#define S_IRWXG 0000070
#define S_IRGRP 0000040
#define S_IWGRP 0000020
#define S_IXGRP 0000010
#define S_IRWXO 0000007
#define S_IROTH 0000004
#define S_IWOTH 0000002
#define S_IXOTH 0000001
#define S_ISUID 0004000
#define S_ISGID 0002000
#define S_ISVTX 0001000

#define S_ISBLK(mode) (((mode) & S_IFMT) == S_IFBLK)
#define S_ISCHR(mode) (((mode) & S_IFMT) == S_IFCHR)
#define S_ISDIR(mode) (((mode) & S_IFMT) == S_IFDIR)
#define S_ISFIFO(mode) (((mode) & S_IFMT) == S_IFIFO)
#define S_ISREG(mode) (((mode) & S_IFMT) == S_IFREG)
#define S_ISLNK(mode) (((mode) & S_IFMT) == S_IFLNK)
#define S_ISSOCK(mode) (((mode) & S_IFMT) == S_IFSOCK)

#define S_TYPEISMQ(status) 0
#define S_TYPEISSEM(status) 0
#define S_TYPEISSHM(status) 0
#define S_TYPEISTMO(status) 0

#define UTIME_NOW  ((1L << 30) - 1L)
#define UTIME_OMIT ((1L << 30) - 2L)

struct stat {
    dev_t st_dev;
    ino_t st_ino;
    nlink_t st_nlink;
    mode_t st_mode;
    uid_t st_uid;
    gid_t st_gid;
    uint32_t __pad0;
    dev_t st_rdev;
    off_t st_size;
    blksize_t st_blksize;
    blkcnt_t st_blocks;
    struct timespec st_atim;
    struct timespec st_mtim;
    struct timespec st_ctim;
    int64_t __reserved[3];
};

#define st_atime st_atim.tv_sec
#define st_mtime st_mtim.tv_sec
#define st_ctime st_ctim.tv_sec

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int chmod(const char *path, mode_t mode);
int fchmod(int file_descriptor, mode_t mode);
int fchmodat(int directory_descriptor, const char *path, mode_t mode, int flags);
int fstat(int file_descriptor, struct stat *status);
int fstatat(int directory_descriptor, const char *KZ_POSIX_RESTRICT path,
            struct stat *KZ_POSIX_RESTRICT status, int flags);
int futimens(int file_descriptor, const struct timespec times[2]);
int lstat(const char *KZ_POSIX_RESTRICT path, struct stat *KZ_POSIX_RESTRICT status);
int mkdir(const char *path, mode_t mode);
int mkdirat(int directory_descriptor, const char *path, mode_t mode);
int mkfifo(const char *path, mode_t mode);
int mkfifoat(int directory_descriptor, const char *path, mode_t mode);
int mknod(const char *path, mode_t mode, dev_t device);
int mknodat(int directory_descriptor, const char *path, mode_t mode, dev_t device);
int stat(const char *KZ_POSIX_RESTRICT path, struct stat *KZ_POSIX_RESTRICT status);
mode_t umask(mode_t mask);
int utimensat(int directory_descriptor, const char *path,
              const struct timespec times[2], int flags);
int rmdir(const char *path);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif