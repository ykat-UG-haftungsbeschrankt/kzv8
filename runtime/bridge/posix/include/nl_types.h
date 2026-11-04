#ifndef KZ_POSIX_NL_TYPES_H_
#define KZ_POSIX_NL_TYPES_H_

typedef int nl_catd;
typedef int nl_item;

#define NL_SETD 1
#define NL_CAT_LOCALE 1

#ifdef __cplusplus
extern "C" {
#endif

int catclose(nl_catd catalog);
char *catgets(nl_catd catalog, int set_id, int message_id,
              const char *default_message);
nl_catd catopen(const char *name, int flags);

#ifdef __cplusplus
}
#endif

#endif