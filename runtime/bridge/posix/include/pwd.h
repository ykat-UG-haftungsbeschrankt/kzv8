#ifndef KZ_POSIX_PWD_H_
#define KZ_POSIX_PWD_H_

#include <sys/types.h>

struct passwd {
    char *pw_name;
    uid_t pw_uid;
    gid_t pw_gid;
    char *pw_dir;
    char *pw_shell;
};

#ifdef __cplusplus
extern "C" {
#endif

void endpwent(void);
struct passwd *getpwent(void);
struct passwd *getpwnam(const char *name);
int getpwnam_r(const char *name, struct passwd *buffer, char *scratch,
               size_t scratch_size, struct passwd **result);
struct passwd *getpwuid(uid_t user_id);
int getpwuid_r(uid_t user_id, struct passwd *buffer, char *scratch,
               size_t scratch_size, struct passwd **result);
void setpwent(void);

#ifdef __cplusplus
}
#endif

#endif