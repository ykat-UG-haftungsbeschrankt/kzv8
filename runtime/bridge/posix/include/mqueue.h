#ifndef KZ_POSIX_MQUEUE_H_
#define KZ_POSIX_MQUEUE_H_

#include <fcntl.h>
#include <sys/types.h>
#include <time.h>

typedef int mqd_t;

struct sigevent;

struct mq_attr {
    long mq_flags;
    long mq_maxmsg;
    long mq_msgsize;
    long mq_curmsgs;
};

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int mq_close(mqd_t descriptor);
int mq_getattr(mqd_t descriptor, struct mq_attr *attributes);
int mq_notify(mqd_t descriptor, const struct sigevent *event);
mqd_t mq_open(const char *name, int flags, ...);
ssize_t mq_receive(mqd_t descriptor, char *message, size_t message_size,
                   unsigned int *priority);
int mq_send(mqd_t descriptor, const char *message, size_t message_size,
            unsigned int priority);
int mq_setattr(mqd_t descriptor, const struct mq_attr *KZ_POSIX_RESTRICT attributes,
               struct mq_attr *KZ_POSIX_RESTRICT old_attributes);
ssize_t mq_timedreceive(mqd_t descriptor, char *KZ_POSIX_RESTRICT message,
                        size_t message_size, unsigned int *KZ_POSIX_RESTRICT priority,
                        const struct timespec *KZ_POSIX_RESTRICT timeout);
int mq_timedsend(mqd_t descriptor, const char *message, size_t message_size,
                 unsigned int priority, const struct timespec *timeout);
int mq_unlink(const char *name);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif