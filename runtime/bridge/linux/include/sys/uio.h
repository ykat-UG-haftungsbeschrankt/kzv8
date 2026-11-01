#ifndef KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_SYS_UIO_H_
#define KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_SYS_UIO_H_

#include <stddef.h>
#include <stdint.h>

typedef intptr_t ssize_t;

struct iovec {
    void* iov_base;
    size_t iov_len;
};

#ifdef __cplusplus
extern "C" {
#endif

ssize_t writev(int file_descriptor, const struct iovec* vectors, int count);

#ifdef __cplusplus
}
#endif

#endif