#ifndef KZ_POSIX_FMTMSG_H_
#define KZ_POSIX_FMTMSG_H_

#define MM_HARD 0x0001L
#define MM_SOFT 0x0002L
#define MM_FIRM 0x0004L
#define MM_APPL 0x0008L
#define MM_UTIL 0x0010L
#define MM_OPSYS 0x0020L
#define MM_RECOVER 0x0040L
#define MM_NRECOV 0x0080L
#define MM_PRINT 0x0100L
#define MM_CONSOLE 0x0200L

#define MM_HALT 1
#define MM_ERROR 2
#define MM_WARNING 3
#define MM_INFO 4
#define MM_NOSEV 0

#define MM_OK 0
#define MM_NOTOK 1
#define MM_NOMSG 2
#define MM_NOCON 4

#define MM_NULLMC 0L
#define MM_NULLSEV 0
#define MM_NULLLBL ((char *)0)
#define MM_NULLTXT ((char *)0)
#define MM_NULLACT ((char *)0)
#define MM_NULLTAG ((char *)0)

#ifdef __cplusplus
extern "C" {
#endif

int fmtmsg(long classification, const char *label, int severity,
           const char *text, const char *action, const char *tag);

#ifdef __cplusplus
}
#endif

#endif