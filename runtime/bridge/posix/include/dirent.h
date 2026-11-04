#ifndef KZ_POSIX_DIRENT_H_
#define KZ_POSIX_DIRENT_H_

#include <sys/types.h>

typedef struct kz_posix_DIR DIR;

struct dirent {
    ino_t d_ino;
    char d_name[256];
};

struct posix_dent {
    ino_t d_ino;
    reclen_t d_reclen;
    unsigned char d_type;
    char d_name[256];
};

#define DT_UNKNOWN 0
#define DT_FIFO 1
#define DT_CHR 2
#define DT_DIR 3
#define DT_BLK 4
#define DT_REG 5
#define DT_LNK 6
#define DT_SOCK 7
#define DT_MQ 8
#define DT_SEM 9
#define DT_SHM 10
#define DT_TMO 11

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int alphasort(const struct dirent **left, const struct dirent **right);
int closedir(DIR *directory);
int dirfd(DIR *directory);
DIR *fdopendir(int file_descriptor);
DIR *opendir(const char *path);
ssize_t posix_getdents(int file_descriptor, void *buffer, size_t buffer_size, int flags);
struct dirent *readdir(DIR *directory);
int readdir_r(DIR *KZ_POSIX_RESTRICT directory,
              struct dirent *KZ_POSIX_RESTRICT entry,
              struct dirent **KZ_POSIX_RESTRICT result);
void rewinddir(DIR *directory);
int scandir(const char *path, struct dirent ***entries,
            int (*select_entry)(const struct dirent *),
            int (*compare_entries)(const struct dirent **, const struct dirent **));
void seekdir(DIR *directory, long position);
long telldir(DIR *directory);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif