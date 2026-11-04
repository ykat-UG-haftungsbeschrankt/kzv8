#ifndef KZ_POSIX_SYS_SHM_H_
#define KZ_POSIX_SYS_SHM_H_

#include <stdint.h>
#include <sys/ipc.h>
#include <sys/types.h>

typedef unsigned long shmatt_t;

#define SHM_RDONLY 010000
#define SHM_RND 020000
#define SHMLBA 4096
#define SHM_FAILED ((void *)(intptr_t)-1)

struct shmid_ds {
    struct ipc_perm shm_perm;
    size_t shm_segsz;
    pid_t shm_lpid;
    pid_t shm_cpid;
    shmatt_t shm_nattch;
    time_t shm_atime;
    time_t shm_dtime;
    time_t shm_ctime;
};

#ifdef __cplusplus
extern "C" {
#endif

void *shmat(int shared_memory_id, const void *address, int flags);
int shmctl(int shared_memory_id, int command, struct shmid_ds *buffer);
int shmdt(const void *address);
int shmget(key_t key, size_t size, int flags);

#ifdef __cplusplus
}
#endif

#endif