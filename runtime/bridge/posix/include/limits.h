#ifndef KZ_POSIX_LIMITS_H_
#define KZ_POSIX_LIMITS_H_

#include_next <limits.h>

#ifndef AIO_LISTIO_MAX
#define AIO_LISTIO_MAX 2
#endif
#ifndef AIO_MAX
#define AIO_MAX 1
#endif
#ifndef AIO_PRIO_DELTA_MAX
#define AIO_PRIO_DELTA_MAX 0
#endif
#ifndef ARG_MAX
#define ARG_MAX 4096
#endif
#ifndef ATEXIT_MAX
#define ATEXIT_MAX 32
#endif
#ifndef CHILD_MAX
#define CHILD_MAX 25
#endif
#ifndef DELAYTIMER_MAX
#define DELAYTIMER_MAX 32
#endif
#ifndef GETENTROPY_MAX
#define GETENTROPY_MAX 256
#endif
#ifndef HOST_NAME_MAX
#define HOST_NAME_MAX 255
#endif
#ifndef IOV_MAX
#define IOV_MAX 16
#endif
#ifndef LOGIN_NAME_MAX
#define LOGIN_NAME_MAX 9
#endif
#ifndef MQ_OPEN_MAX
#define MQ_OPEN_MAX 8
#endif
#ifndef MQ_PRIO_MAX
#define MQ_PRIO_MAX 32
#endif
#ifndef OPEN_MAX
#define OPEN_MAX 20
#endif
#ifndef PAGESIZE
#define PAGESIZE 4096
#endif
#ifndef PAGE_SIZE
#define PAGE_SIZE PAGESIZE
#endif
#ifndef PTHREAD_DESTRUCTOR_ITERATIONS
#define PTHREAD_DESTRUCTOR_ITERATIONS 4
#endif
#ifndef PTHREAD_KEYS_MAX
#define PTHREAD_KEYS_MAX 128
#endif
#ifndef PTHREAD_STACK_MIN
#define PTHREAD_STACK_MIN 16384
#endif
#ifndef PTHREAD_THREADS_MAX
#define PTHREAD_THREADS_MAX 64
#endif
#ifndef SEM_NSEMS_MAX
#define SEM_NSEMS_MAX 256
#endif
#ifndef SEM_VALUE_MAX
#define SEM_VALUE_MAX 32767
#endif
#ifndef SIGQUEUE_MAX
#define SIGQUEUE_MAX 32
#endif
#ifndef SSIZE_MAX
#define SSIZE_MAX __PTRDIFF_MAX__
#endif
#ifndef STREAM_MAX
#define STREAM_MAX 16
#endif
#ifndef SYMLOOP_MAX
#define SYMLOOP_MAX 8
#endif
#ifndef TIMER_MAX
#define TIMER_MAX 32
#endif
#ifndef TTY_NAME_MAX
#define TTY_NAME_MAX 9
#endif
#ifndef TZNAME_MAX
#define TZNAME_MAX 6
#endif

#ifndef _POSIX_AIO_LISTIO_MAX
#define _POSIX_AIO_LISTIO_MAX 2
#endif
#ifndef _POSIX_AIO_MAX
#define _POSIX_AIO_MAX 1
#endif
#ifndef _POSIX_ARG_MAX
#define _POSIX_ARG_MAX 4096
#endif
#ifndef _POSIX_CHILD_MAX
#define _POSIX_CHILD_MAX 25
#endif
#ifndef _POSIX_DELAYTIMER_MAX
#define _POSIX_DELAYTIMER_MAX 32
#endif
#ifndef _POSIX_HOST_NAME_MAX
#define _POSIX_HOST_NAME_MAX 255
#endif
#ifndef _POSIX_LINK_MAX
#define _POSIX_LINK_MAX 8
#endif
#ifndef _POSIX_LOGIN_NAME_MAX
#define _POSIX_LOGIN_NAME_MAX 9
#endif
#ifndef _POSIX_MAX_CANON
#define _POSIX_MAX_CANON 255
#endif
#ifndef _POSIX_MAX_INPUT
#define _POSIX_MAX_INPUT 255
#endif
#ifndef _POSIX_MQ_OPEN_MAX
#define _POSIX_MQ_OPEN_MAX 8
#endif
#ifndef _POSIX_MQ_PRIO_MAX
#define _POSIX_MQ_PRIO_MAX 32
#endif
#ifndef _POSIX_NAME_MAX
#define _POSIX_NAME_MAX 14
#endif
#ifndef _POSIX_NGROUPS_MAX
#define _POSIX_NGROUPS_MAX 8
#endif
#ifndef _POSIX_OPEN_MAX
#define _POSIX_OPEN_MAX 20
#endif
#ifndef _POSIX_PATH_MAX
#define _POSIX_PATH_MAX 256
#endif
#ifndef _POSIX_PIPE_BUF
#define _POSIX_PIPE_BUF 512
#endif
#ifndef _POSIX_RE_DUP_MAX
#define _POSIX_RE_DUP_MAX 255
#endif
#ifndef _POSIX_RTSIG_MAX
#define _POSIX_RTSIG_MAX 8
#endif
#ifndef _POSIX_SEM_NSEMS_MAX
#define _POSIX_SEM_NSEMS_MAX 256
#endif
#ifndef _POSIX_SEM_VALUE_MAX
#define _POSIX_SEM_VALUE_MAX 32767
#endif
#ifndef _POSIX_SIGQUEUE_MAX
#define _POSIX_SIGQUEUE_MAX 32
#endif
#ifndef _POSIX_SSIZE_MAX
#define _POSIX_SSIZE_MAX 32767
#endif
#ifndef _POSIX_STREAM_MAX
#define _POSIX_STREAM_MAX 8
#endif
#ifndef _POSIX_SYMLINK_MAX
#define _POSIX_SYMLINK_MAX 255
#endif
#ifndef _POSIX_SYMLOOP_MAX
#define _POSIX_SYMLOOP_MAX 8
#endif
#ifndef _POSIX_THREAD_DESTRUCTOR_ITERATIONS
#define _POSIX_THREAD_DESTRUCTOR_ITERATIONS 4
#endif
#ifndef _POSIX_THREAD_KEYS_MAX
#define _POSIX_THREAD_KEYS_MAX 128
#endif
#ifndef _POSIX_THREAD_THREADS_MAX
#define _POSIX_THREAD_THREADS_MAX 64
#endif
#ifndef _POSIX_TIMER_MAX
#define _POSIX_TIMER_MAX 32
#endif
#ifndef _POSIX_TTY_NAME_MAX
#define _POSIX_TTY_NAME_MAX 9
#endif
#ifndef _POSIX_TZNAME_MAX
#define _POSIX_TZNAME_MAX 6
#endif

#ifndef NAME_MAX
#define NAME_MAX 255
#endif
#ifndef PATH_MAX
#define PATH_MAX 4096
#endif
#ifndef PIPE_BUF
#define PIPE_BUF 512
#endif
#ifndef NL_ARGMAX
#define NL_ARGMAX 9
#endif
#ifndef NL_LANGMAX
#define NL_LANGMAX 14
#endif
#ifndef NL_MSGMAX
#define NL_MSGMAX 32767
#endif
#ifndef NL_SETMAX
#define NL_SETMAX 255
#endif
#ifndef NL_TEXTMAX
#define NL_TEXTMAX 2048
#endif
#ifndef NSIG_MAX
#define NSIG_MAX 128
#endif
#ifndef NZERO
#define NZERO 20
#endif
#ifndef TEXTDOMAIN_MAX
#define TEXTDOMAIN_MAX (_POSIX_NAME_MAX - 3)
#endif

#endif