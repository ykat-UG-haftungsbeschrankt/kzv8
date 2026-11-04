#ifndef KZ_POSIX_TAR_H_
#define KZ_POSIX_TAR_H_

#define TMAGIC "ustar"
#define TMAGLEN 6
#define TVERSION "00"
#define TVERSLEN 2

#define REGTYPE '0'
#define AREGTYPE '\0'
#define LNKTYPE '1'
#define SYMTYPE '2'
#define CHRTYPE '3'
#define BLKTYPE '4'
#define DIRTYPE '5'
#define FIFOTYPE '6'
#define CONTTYPE '7'

#define TSUID 004000
#define TSGID 002000
#define TSVTX 001000
#define TUREAD 000400
#define TUWRITE 000200
#define TUEXEC 000100
#define TGREAD 000040
#define TGWRITE 000020
#define TGEXEC 000010
#define TOREAD 000004
#define TOWRITE 000002
#define TOEXEC 000001

#endif