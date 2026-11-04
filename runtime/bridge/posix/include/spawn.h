#ifndef KZ_POSIX_SPAWN_H_
#define KZ_POSIX_SPAWN_H_

#include <sched.h>
#include <signal.h>
#include <sys/types.h>

typedef struct { uintptr_t __opaque[8]; } posix_spawnattr_t;
typedef struct { uintptr_t __opaque[8]; } posix_spawn_file_actions_t;

#define POSIX_SPAWN_RESETIDS 0x01
#define POSIX_SPAWN_SETPGROUP 0x02
#define POSIX_SPAWN_SETSIGDEF 0x04
#define POSIX_SPAWN_SETSIGMASK 0x08
#define POSIX_SPAWN_SETSCHEDPARAM 0x10
#define POSIX_SPAWN_SETSCHEDULER 0x20
#define POSIX_SPAWN_SETSID 0x40

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int posix_spawn(pid_t *KZ_POSIX_RESTRICT process_id,
                const char *KZ_POSIX_RESTRICT path,
                const posix_spawn_file_actions_t *KZ_POSIX_RESTRICT file_actions,
                const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                char *const arguments[], char *const environment[]);
int posix_spawnp(pid_t *KZ_POSIX_RESTRICT process_id,
                 const char *KZ_POSIX_RESTRICT file,
                 const posix_spawn_file_actions_t *KZ_POSIX_RESTRICT file_actions,
                 const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                 char *const arguments[], char *const environment[]);

int posix_spawn_file_actions_addchdir(posix_spawn_file_actions_t *actions,
                                      const char *path);
int posix_spawn_file_actions_addclose(posix_spawn_file_actions_t *actions,
                                      int file_descriptor);
int posix_spawn_file_actions_adddup2(posix_spawn_file_actions_t *actions,
                                     int source, int destination);
int posix_spawn_file_actions_addfchdir(posix_spawn_file_actions_t *actions,
                                       int file_descriptor);
int posix_spawn_file_actions_addopen(posix_spawn_file_actions_t *actions,
                                     int file_descriptor, const char *path,
                                     int flags, mode_t mode);
int posix_spawn_file_actions_destroy(posix_spawn_file_actions_t *actions);
int posix_spawn_file_actions_init(posix_spawn_file_actions_t *actions);

int posix_spawnattr_destroy(posix_spawnattr_t *attributes);
int posix_spawnattr_getflags(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                             short *KZ_POSIX_RESTRICT flags);
int posix_spawnattr_getpgroup(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                              pid_t *KZ_POSIX_RESTRICT process_group);
int posix_spawnattr_getschedparam(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                                  struct sched_param *KZ_POSIX_RESTRICT parameters);
int posix_spawnattr_getschedpolicy(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                                   int *KZ_POSIX_RESTRICT policy);
int posix_spawnattr_getsigdefault(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                                  sigset_t *KZ_POSIX_RESTRICT signal_set);
int posix_spawnattr_getsigmask(const posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                               sigset_t *KZ_POSIX_RESTRICT signal_set);
int posix_spawnattr_init(posix_spawnattr_t *attributes);
int posix_spawnattr_setflags(posix_spawnattr_t *attributes, short flags);
int posix_spawnattr_setpgroup(posix_spawnattr_t *attributes, pid_t process_group);
int posix_spawnattr_setschedparam(posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                                  const struct sched_param *KZ_POSIX_RESTRICT parameters);
int posix_spawnattr_setschedpolicy(posix_spawnattr_t *attributes, int policy);
int posix_spawnattr_setsigdefault(posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                                  const sigset_t *KZ_POSIX_RESTRICT signal_set);
int posix_spawnattr_setsigmask(posix_spawnattr_t *KZ_POSIX_RESTRICT attributes,
                               const sigset_t *KZ_POSIX_RESTRICT signal_set);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif