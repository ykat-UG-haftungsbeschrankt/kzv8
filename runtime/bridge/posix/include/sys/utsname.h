#ifndef KZ_POSIX_SYS_UTSNAME_H_
#define KZ_POSIX_SYS_UTSNAME_H_

#define _SYS_NMLN 65

struct utsname {
    char sysname[_SYS_NMLN];
    char nodename[_SYS_NMLN];
    char release[_SYS_NMLN];
    char version[_SYS_NMLN];
    char machine[_SYS_NMLN];
    char domainname[_SYS_NMLN];
};

#ifdef __cplusplus
extern "C" {
#endif

int uname(struct utsname *name);

#ifdef __cplusplus
}
#endif

#endif