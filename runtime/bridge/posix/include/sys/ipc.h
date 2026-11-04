#ifndef KZ_POSIX_SYS_IPC_H_
#define KZ_POSIX_SYS_IPC_H_

#include <sys/types.h>

struct ipc_perm {
    uid_t uid;
    gid_t gid;
    uid_t cuid;
    gid_t cgid;
    mode_t mode;
};

#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_NOWAIT 04000
#define IPC_PRIVATE ((key_t)0)

#define IPC_RMID 0
#define IPC_SET 1
#define IPC_STAT 2

#ifdef __cplusplus
extern "C" {
#endif

key_t ftok(const char *path, int project_id);

#ifdef __cplusplus
}
#endif

#endif