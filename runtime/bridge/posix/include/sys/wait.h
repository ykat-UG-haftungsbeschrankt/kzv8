#ifndef KZ_POSIX_SYS_WAIT_H_
#define KZ_POSIX_SYS_WAIT_H_

#include <signal.h>
#include <sys/types.h>

typedef enum {
    P_ALL = 0,
    P_PID = 1,
    P_PGID = 2
} idtype_t;

#define WNOHANG 0x0001
#define WUNTRACED 0x0002
#define WCONTINUED 0x0008
#define WEXITED 0x0004
#define WNOWAIT 0x01000000
#define WSTOPPED WUNTRACED

#define WEXITSTATUS(status) (((status) & 0xff00) >> 8)
#define WTERMSIG(status) ((status) & 0x7f)
#define WSTOPSIG(status) WEXITSTATUS(status)
#define WIFEXITED(status) (WTERMSIG(status) == 0)
#define WIFSIGNALED(status) \
    (WTERMSIG(status) != 0 && WTERMSIG(status) != 0x7f)
#define WIFSTOPPED(status) (WTERMSIG(status) == 0x7f)
#define WIFCONTINUED(status) ((status) == 0xffff)
#define WCOREDUMP(status) (((status) & 0x80) != 0)

#ifdef __cplusplus
extern "C" {
#endif

pid_t wait(int *status);
int waitid(idtype_t type, id_t identifier, siginfo_t *information, int options);
pid_t waitpid(pid_t process_id, int *status, int options);

#ifdef __cplusplus
}
#endif

#endif