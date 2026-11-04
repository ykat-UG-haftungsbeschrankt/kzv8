#ifndef KZ_POSIX_LIBGEN_H_
#define KZ_POSIX_LIBGEN_H_

#ifdef __cplusplus
extern "C" {
#endif

char *basename(char *path);
char *dirname(char *path);

#ifdef __cplusplus
}
#endif

#endif