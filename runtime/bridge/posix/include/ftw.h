#ifndef KZ_POSIX_FTW_H_
#define KZ_POSIX_FTW_H_

#include <sys/stat.h>

struct FTW {
    int base;
    int level;
};

#define FTW_F 0
#define FTW_D 1
#define FTW_DNR 2
#define FTW_DP 3
#define FTW_NS 4
#define FTW_SL 5
#define FTW_SLN 6

#define FTW_PHYS 0x01
#define FTW_MOUNT 0x02
#define FTW_XDEV 0x04
#define FTW_DEPTH 0x08
#define FTW_CHDIR 0x10

#ifdef __cplusplus
extern "C" {
#endif

int nftw(const char *path,
         int (*callback)(const char *path, const struct stat *status,
                         int type, struct FTW *walk_info),
         int descriptor_limit, int flags);

#ifdef __cplusplus
}
#endif

#endif