#ifndef KZ_POSIX_NETDB_H_
#define KZ_POSIX_NETDB_H_

#include <inttypes.h>
#include <netinet/in.h>
#include <sys/socket.h>

#define AI_PASSIVE 0x01
#define AI_CANONNAME 0x02
#define AI_NUMERICHOST 0x04
#define AI_NUMERICSERV 0x08
#define AI_V4MAPPED 0x10
#define AI_ALL 0x20
#define AI_ADDRCONFIG 0x40

#define NI_NOFQDN 0x01
#define NI_NUMERICHOST 0x02
#define NI_NAMEREQD 0x04
#define NI_NUMERICSERV 0x08
#define NI_NUMERICSCOPE 0x10
#define NI_DGRAM 0x20

#define EAI_AGAIN -1
#define EAI_BADFLAGS -2
#define EAI_FAIL -3
#define EAI_FAMILY -4
#define EAI_MEMORY -5
#define EAI_NONAME -6
#define EAI_SERVICE -7
#define EAI_SOCKTYPE -8
#define EAI_SYSTEM -9
#define EAI_OVERFLOW -10

#define IPPORT_RESERVED 1023

struct addrinfo {
    int ai_flags;
    int ai_family;
    int ai_socktype;
    int ai_protocol;
    socklen_t ai_addrlen;
    struct sockaddr *ai_addr;
    char *ai_canonname;
    struct addrinfo *ai_next;
};

struct hostent {
    char *h_name;
    char **h_aliases;
    int h_addrtype;
    int h_length;
    char **h_addr_list;
};

struct netent {
    char *n_name;
    char **n_aliases;
    int n_addrtype;
    uint32_t n_net;
};

struct protoent {
    char *p_name;
    char **p_aliases;
    int p_proto;
};

struct servent {
    char *s_name;
    char **s_aliases;
    int s_port;
    char *s_proto;
};

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

void endhostent(void);
void endnetent(void);
void endprotoent(void);
void endservent(void);
void freeaddrinfo(struct addrinfo *results);
const char *gai_strerror(int error_code);
int getaddrinfo(const char *KZ_POSIX_RESTRICT node,
                const char *KZ_POSIX_RESTRICT service,
                const struct addrinfo *KZ_POSIX_RESTRICT hints,
                struct addrinfo **KZ_POSIX_RESTRICT results);
struct hostent *gethostent(void);
int getnameinfo(const struct sockaddr *KZ_POSIX_RESTRICT address,
                socklen_t address_length, char *KZ_POSIX_RESTRICT host,
                socklen_t host_length, char *KZ_POSIX_RESTRICT service,
                socklen_t service_length, int flags);
struct netent *getnetbyaddr(uint32_t network, int address_type);
struct netent *getnetbyname(const char *name);
struct netent *getnetent(void);
struct protoent *getprotobyname(const char *name);
struct protoent *getprotobynumber(int protocol);
struct protoent *getprotoent(void);
struct servent *getservbyname(const char *name, const char *protocol);
struct servent *getservbyport(int port, const char *protocol);
struct servent *getservent(void);
void sethostent(int stay_open);
void setnetent(int stay_open);
void setprotoent(int stay_open);
void setservent(int stay_open);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif