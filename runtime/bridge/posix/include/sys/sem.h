#ifndef KZ_POSIX_SYS_SEM_H_
#define KZ_POSIX_SYS_SEM_H_

#include <sys/ipc.h>
#include <sys/types.h>

#define SEM_UNDO 0x1000

#define GETNCNT 3
#define GETPID 11
#define GETVAL 12
#define GETALL 13
#define GETZCNT 15
#define SETVAL 16
#define SETALL 17

struct semid_ds {
    struct ipc_perm sem_perm;
    unsigned short sem_nsems;
    time_t sem_otime;
    time_t sem_ctime;
};

struct sembuf {
    unsigned short sem_num;
    short sem_op;
    short sem_flg;
};

#ifdef __cplusplus
extern "C" {
#endif

int semctl(int semaphore_id, int semaphore_number, int command, ...);
int semget(key_t key, int semaphore_count, int flags);
int semop(int semaphore_id, struct sembuf *operations, size_t operation_count);

#ifdef __cplusplus
}
#endif

#endif