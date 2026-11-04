#ifndef KZ_POSIX_UTMPX_H_
#define KZ_POSIX_UTMPX_H_

#include <sys/time.h>
#include <sys/types.h>

#define EMPTY 0
#define BOOT_TIME 2
#define OLD_TIME 4
#define NEW_TIME 3
#define USER_PROCESS 7
#define INIT_PROCESS 5
#define LOGIN_PROCESS 6
#define DEAD_PROCESS 8

struct utmpx {
    char ut_user[32];
    char ut_id[4];
    char ut_line[32];
    pid_t ut_pid;
    short ut_type;
    struct timeval ut_tv;
};

#ifdef __cplusplus
extern "C" {
#endif

void endutxent(void);
struct utmpx *getutxent(void);
struct utmpx *getutxid(const struct utmpx *entry);
struct utmpx *getutxline(const struct utmpx *entry);
struct utmpx *pututxline(const struct utmpx *entry);
void setutxent(void);

#ifdef __cplusplus
}
#endif

#endif