#ifndef KZ_POSIX_SYS_STATVFS_H_
#define KZ_POSIX_SYS_STATVFS_H_

#include <sys/types.h>

#define ST_RDONLY 0x01
#define ST_NOSUID 0x02

struct statvfs {
    unsigned long f_bsize;
    unsigned long f_frsize;
    fsblkcnt_t f_blocks;
    fsblkcnt_t f_bfree;
    fsblkcnt_t f_bavail;
    fsfilcnt_t f_files;
    fsfilcnt_t f_ffree;
    fsfilcnt_t f_favail;
    unsigned long f_fsid;
    unsigned long f_flag;
    unsigned long f_namemax;
};

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int fstatvfs(int file_descriptor, struct statvfs *status);
int statvfs(const char *KZ_POSIX_RESTRICT path,
            struct statvfs *KZ_POSIX_RESTRICT status);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif