#ifndef KZ_POSIX_SIGNAL_H_
#define KZ_POSIX_SIGNAL_H_

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <time.h>

typedef int sig_atomic_t;
typedef struct {
    uint64_t __bits[16];
} sigset_t;

union sigval {
    int sival_int;
    void *sival_ptr;
};

struct sigevent {
    int sigev_notify;
    int sigev_signo;
    union sigval sigev_value;
    void (*sigev_notify_function)(union sigval);
    pthread_attr_t *sigev_notify_attributes;
};

typedef struct {
    int si_signo;
    int si_code;
    int si_errno;
    pid_t si_pid;
    uid_t si_uid;
    void *si_addr;
    int si_status;
    union sigval si_value;
} siginfo_t;

typedef struct {
    void *ss_sp;
    size_t ss_size;
    int ss_flags;
} stack_t;

struct sigaction {
    void (*sa_handler)(int);
    sigset_t sa_mask;
    int sa_flags;
    void (*sa_sigaction)(int, siginfo_t *, void *);
};

#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)
#define SIG_ERR ((void (*)(int))-1)

#define SIGHUP 1
#define SIGINT 2
#define SIGQUIT 3
#define SIGILL 4
#define SIGTRAP 5
#define SIGABRT 6
#define SIGBUS 7
#define SIGFPE 8
#define SIGKILL 9
#define SIGUSR1 10
#define SIGSEGV 11
#define SIGUSR2 12
#define SIGPIPE 13
#define SIGALRM 14
#define SIGTERM 15
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define SIGTSTP 20
#define SIGTTIN 21
#define SIGTTOU 22
#define SIGURG 23
#define SIGXCPU 24
#define SIGXFSZ 25
#define SIGVTALRM 26
#define SIGPROF 27
#define SIGWINCH 28
#define SIGPOLL 29
#define SIGSYS 31
#define SIGRTMIN 34
#define SIGRTMAX 63
#define SIG2STR_MAX 16

#define SIG_BLOCK 0
#define SIG_UNBLOCK 1
#define SIG_SETMASK 2

#define SA_NOCLDSTOP 0x00000001
#define SA_ONSTACK 0x08000000
#define SA_RESETHAND 0x80000000U
#define SA_RESTART 0x10000000
#define SA_SIGINFO 0x00000004
#define SA_NOCLDWAIT 0x00000002
#define SA_NODEFER 0x40000000

#define SS_ONSTACK 1
#define SS_DISABLE 2
#define MINSIGSTKSZ 2048
#define SIGSTKSZ 8192

#define SIGEV_SIGNAL 0
#define SIGEV_NONE 1
#define SIGEV_THREAD 2

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int kill(pid_t process_id, int signal_number);
void psiginfo(const siginfo_t *information, const char *message);
void psignal(int signal_number, const char *message);
int pthread_kill(pthread_t thread, int signal_number);
int pthread_sigmask(int how, const sigset_t *KZ_POSIX_RESTRICT set,
                    sigset_t *KZ_POSIX_RESTRICT old_set);
int raise(int signal_number);
int sig2str(int signal_number, char *string);
int sigaction(int signal_number, const struct sigaction *KZ_POSIX_RESTRICT action,
              struct sigaction *KZ_POSIX_RESTRICT old_action);
int sigaddset(sigset_t *set, int signal_number);
int sigaltstack(const stack_t *KZ_POSIX_RESTRICT stack,
                stack_t *KZ_POSIX_RESTRICT old_stack);
int sigdelset(sigset_t *set, int signal_number);
int sigemptyset(sigset_t *set);
int sigfillset(sigset_t *set);
int sigismember(const sigset_t *set, int signal_number);
int sigpending(sigset_t *set);
int sigprocmask(int how, const sigset_t *KZ_POSIX_RESTRICT set,
                sigset_t *KZ_POSIX_RESTRICT old_set);
int sigqueue(pid_t process_id, int signal_number, union sigval value);
int sigsuspend(const sigset_t *set);
int sigtimedwait(const sigset_t *KZ_POSIX_RESTRICT set,
                 siginfo_t *KZ_POSIX_RESTRICT information,
                 const struct timespec *KZ_POSIX_RESTRICT timeout);
int sigwait(const sigset_t *KZ_POSIX_RESTRICT set, int *KZ_POSIX_RESTRICT signal_number);
int sigwaitinfo(const sigset_t *KZ_POSIX_RESTRICT set,
                siginfo_t *KZ_POSIX_RESTRICT information);
int str2sig(const char *KZ_POSIX_RESTRICT string, int *KZ_POSIX_RESTRICT signal_number);
void (*signal(int signal_number, void (*handler)(int)))(int);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif