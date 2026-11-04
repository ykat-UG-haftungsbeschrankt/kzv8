#ifndef KZ_POSIX_SYS_MSG_H_
#define KZ_POSIX_SYS_MSG_H_

#include <sys/ipc.h>
#include <sys/types.h>

typedef unsigned long msgqnum_t;
typedef unsigned long msglen_t;

#define MSG_NOERROR 010000

struct msqid_ds {
    struct ipc_perm msg_perm;
    msgqnum_t msg_qnum;
    msglen_t msg_qbytes;
    pid_t msg_lspid;
    pid_t msg_lrpid;
    time_t msg_stime;
    time_t msg_rtime;
    time_t msg_ctime;
};

#ifdef __cplusplus
extern "C" {
#endif

int msgctl(int queue_id, int command, struct msqid_ds *buffer);
int msgget(key_t key, int flags);
ssize_t msgrcv(int queue_id, void *message, size_t message_size,
               long message_type, int flags);
int msgsnd(int queue_id, const void *message, size_t message_size, int flags);

#ifdef __cplusplus
}
#endif

#endif