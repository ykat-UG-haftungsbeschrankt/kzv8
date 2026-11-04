#ifndef KZ_POSIX_TERMIOS_H_
#define KZ_POSIX_TERMIOS_H_

#include <sys/types.h>

typedef unsigned char cc_t;
typedef unsigned int speed_t;
typedef unsigned int tcflag_t;

#define NCCS 32

struct termios {
    tcflag_t c_iflag;
    tcflag_t c_oflag;
    tcflag_t c_cflag;
    tcflag_t c_lflag;
    cc_t c_cc[NCCS];
    speed_t c_ispeed;
    speed_t c_ospeed;
};

struct winsize {
    unsigned short ws_row;
    unsigned short ws_col;
    unsigned short ws_xpixel;
    unsigned short ws_ypixel;
};

#define VINTR 0
#define VQUIT 1
#define VERASE 2
#define VKILL 3
#define VEOF 4
#define VTIME 5
#define VMIN 6
#define VSTART 8
#define VSTOP 9
#define VSUSP 10
#define VEOL 11
#define VREPRINT 12
#define VDISCARD 13
#define VWERASE 14
#define VLNEXT 15
#define VEOL2 16

#define IGNBRK 0x0001
#define BRKINT 0x0002
#define IGNPAR 0x0004
#define PARMRK 0x0008
#define INPCK 0x0010
#define ISTRIP 0x0020
#define INLCR 0x0040
#define IGNCR 0x0080
#define ICRNL 0x0100
#define IXON 0x0400
#define IXANY 0x0800
#define IXOFF 0x1000

#define OPOST 0x0001
#define ONLCR 0x0004
#define OCRNL 0x0008
#define ONOCR 0x0010
#define ONLRET 0x0020
#define OFILL 0x0040
#define OFDEL 0x0080

#define CLOCAL 0x0800
#define CREAD 0x0080
#define CSIZE 0x0030
#define CS5 0x0000
#define CS6 0x0010
#define CS7 0x0020
#define CS8 0x0030
#define CSTOPB 0x0040
#define HUPCL 0x0400
#define PARENB 0x0100
#define PARODD 0x0200

#define ECHO 0x0008
#define ECHOE 0x0010
#define ECHOK 0x0020
#define ECHONL 0x0040
#define ICANON 0x0002
#define IEXTEN 0x8000
#define ISIG 0x0001
#define NOFLSH 0x0080
#define TOSTOP 0x0100

#define TCSANOW 0
#define TCSADRAIN 1
#define TCSAFLUSH 2
#define TCIFLUSH 0
#define TCOFLUSH 1
#define TCIOFLUSH 2
#define TCOOFF 0
#define TCOON 1
#define TCIOFF 2
#define TCION 3

#define B0 0
#define B50 1
#define B75 2
#define B110 3
#define B134 4
#define B150 5
#define B200 6
#define B300 7
#define B600 8
#define B1200 9
#define B1800 10
#define B2400 11
#define B4800 12
#define B9600 13
#define B19200 14
#define B38400 15

#ifdef __cplusplus
extern "C" {
#endif

speed_t cfgetispeed(const struct termios *attributes);
speed_t cfgetospeed(const struct termios *attributes);
int cfsetispeed(struct termios *attributes, speed_t speed);
int cfsetospeed(struct termios *attributes, speed_t speed);
int tcdrain(int file_descriptor);
int tcflow(int file_descriptor, int action);
int tcflush(int file_descriptor, int queue_selector);
int tcgetattr(int file_descriptor, struct termios *attributes);
pid_t tcgetsid(int file_descriptor);
int tcgetwinsize(int file_descriptor, struct winsize *size);
int tcsendbreak(int file_descriptor, int duration);
int tcsetattr(int file_descriptor, int actions, const struct termios *attributes);
int tcsetwinsize(int file_descriptor, const struct winsize *size);

#ifdef __cplusplus
}
#endif

#endif