#ifndef KZ_POSIX_DEVCTL_H_
#define KZ_POSIX_DEVCTL_H_

#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int posix_devctl(int file_descriptor, int command,
                 void *KZ_POSIX_RESTRICT control_data, size_t control_size,
                 int *KZ_POSIX_RESTRICT device_information);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif