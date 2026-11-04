#ifndef KZ_POSIX_ARPA_INET_H_
#define KZ_POSIX_ARPA_INET_H_

#include <sys/socket.h>
#include <netinet/in.h>

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

uint32_t htonl(uint32_t value);
uint16_t htons(uint16_t value);
uint32_t ntohl(uint32_t value);
uint16_t ntohs(uint16_t value);
in_addr_t inet_addr(const char *text);
char *inet_ntoa(struct in_addr address);
const char *inet_ntop(int address_family, const void *KZ_POSIX_RESTRICT source,
                      char *KZ_POSIX_RESTRICT destination, socklen_t destination_size);
int inet_pton(int address_family, const char *KZ_POSIX_RESTRICT source,
              void *KZ_POSIX_RESTRICT destination);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif