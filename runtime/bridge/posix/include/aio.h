#ifndef KZ_POSIX_AIO_H_
#define KZ_POSIX_AIO_H_

#include <signal.h>

struct aiocb {
    int aio_fildes;
    off_t aio_offset;
    volatile void *aio_buf;
    size_t aio_nbytes;
    int aio_reqprio;
    struct sigevent aio_sigevent;
    int aio_lio_opcode;
};

#define AIO_CANCELED 0
#define AIO_NOTCANCELED 1
#define AIO_ALLDONE 2

#define LIO_READ 0
#define LIO_WRITE 1
#define LIO_NOP 2
#define LIO_WAIT 0
#define LIO_NOWAIT 1

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int aio_cancel(int file_descriptor, struct aiocb *control_block);
int aio_error(const struct aiocb *control_block);
int aio_fsync(int operation, struct aiocb *control_block);
int aio_read(struct aiocb *control_block);
ssize_t aio_return(struct aiocb *control_block);
int aio_suspend(const struct aiocb *const list[], int count,
                const struct timespec *timeout);
int aio_write(struct aiocb *control_block);
int lio_listio(int mode, struct aiocb *KZ_POSIX_RESTRICT const list[KZ_POSIX_RESTRICT],
               int count, struct sigevent *KZ_POSIX_RESTRICT event);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif