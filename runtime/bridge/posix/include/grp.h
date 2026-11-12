/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#ifndef KZ_POSIX_GRP_H_
#define KZ_POSIX_GRP_H_

#include <sys/types.h>

struct group {
    char *gr_name;
    gid_t gr_gid;
    char **gr_mem;
};

#ifdef __cplusplus
extern "C" {
#endif

void endgrent(void);
struct group *getgrent(void);
struct group *getgrgid(gid_t group_id);
int getgrgid_r(gid_t group_id, struct group *buffer, char *scratch,
               size_t scratch_size, struct group **result);
struct group *getgrnam(const char *name);
int getgrnam_r(const char *name, struct group *buffer, char *scratch,
               size_t scratch_size, struct group **result);
void setgrent(void);
int setgroups(size_t group_count, const gid_t groups[]);

#ifdef __cplusplus
}
#endif

#endif