#ifndef KZ_POSIX_SYS_TYPES_H_
#define KZ_POSIX_SYS_TYPES_H_

#include <stddef.h>
#include <stdint.h>

typedef int64_t blkcnt_t;
typedef int64_t blksize_t;
typedef int64_t clock_t;
typedef int32_t clockid_t;
typedef uint64_t dev_t;
typedef uint64_t fsblkcnt_t;
typedef uint64_t fsfilcnt_t;
typedef uint32_t gid_t;
typedef uint32_t id_t;
typedef uint64_t ino_t;
typedef int32_t key_t;
typedef uint32_t mode_t;
typedef uint64_t nlink_t;
typedef int64_t off_t;
typedef int32_t pid_t;
typedef uintptr_t pthread_t;
typedef uint32_t pthread_key_t;
typedef uint32_t pthread_once_t;
typedef struct { uintptr_t __opaque[8]; } pthread_attr_t;
typedef struct { uintptr_t __opaque[8]; } pthread_barrier_t;
typedef struct { uintptr_t __opaque[4]; } pthread_barrierattr_t;
typedef struct { uintptr_t __opaque[8]; } pthread_cond_t;
typedef struct { uintptr_t __opaque[4]; } pthread_condattr_t;
typedef struct { uintptr_t __opaque[4]; } pthread_mutex_t;
typedef struct { uintptr_t __opaque[4]; } pthread_mutexattr_t;
typedef struct { uintptr_t __opaque[8]; } pthread_rwlock_t;
typedef struct { uintptr_t __opaque[4]; } pthread_rwlockattr_t;
typedef int32_t pthread_spinlock_t;
typedef uint64_t reclen_t;
typedef intptr_t ssize_t;
typedef int64_t suseconds_t;
typedef int64_t time_t;
typedef uintptr_t timer_t;
typedef uint32_t uid_t;

#endif