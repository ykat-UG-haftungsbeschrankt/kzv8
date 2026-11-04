#ifndef KZ_POSIX_FNMATCH_H_
#define KZ_POSIX_FNMATCH_H_

#define FNM_NOMATCH 1
#define FNM_PATHNAME 0x01
#define FNM_NOESCAPE 0x02
#define FNM_PERIOD 0x04
#define FNM_CASEFOLD 0x08
#define FNM_IGNORECASE FNM_CASEFOLD

#ifdef __cplusplus
extern "C" {
#endif

int fnmatch(const char *pattern, const char *string, int flags);

#ifdef __cplusplus
}
#endif

#endif