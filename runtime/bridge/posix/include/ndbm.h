#ifndef KZ_POSIX_NDBM_H_
#define KZ_POSIX_NDBM_H_

#include <stddef.h>
#include <sys/types.h>

typedef struct kz_posix_dbm DBM;

typedef struct {
    void *dptr;
    size_t dsize;
} datum;

#define DBM_INSERT 0
#define DBM_REPLACE 1

#ifdef __cplusplus
extern "C" {
#endif

int dbm_clearerr(DBM *database);
void dbm_close(DBM *database);
int dbm_delete(DBM *database, datum key);
int dbm_error(DBM *database);
datum dbm_fetch(DBM *database, datum key);
datum dbm_firstkey(DBM *database);
datum dbm_nextkey(DBM *database);
DBM *dbm_open(const char *file, int flags, mode_t mode);
int dbm_store(DBM *database, datum key, datum content, int mode);

#ifdef __cplusplus
}
#endif

#endif