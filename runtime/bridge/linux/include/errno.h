#ifndef KZ_ERRNO_H
#define KZ_ERRNO_H

#ifndef EINVAL
#define EINVAL 22
#endif
#ifndef ENOMEM
#define ENOMEM 12
#endif

#ifdef __cplusplus
extern "C" {
#endif

int *__errno_location(void);

#ifdef __cplusplus
}
#endif

#define errno (*__errno_location())

#endif