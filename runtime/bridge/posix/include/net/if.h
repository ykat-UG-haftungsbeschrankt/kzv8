#ifndef KZ_POSIX_NET_IF_H_
#define KZ_POSIX_NET_IF_H_

#define IF_NAMESIZE 16

struct if_nameindex {
    unsigned int if_index;
    char *if_name;
};

#ifdef __cplusplus
extern "C" {
#endif

void if_freenameindex(struct if_nameindex *interfaces);
char *if_indextoname(unsigned int interface_index, char *name);
struct if_nameindex *if_nameindex(void);
unsigned int if_nametoindex(const char *name);

#ifdef __cplusplus
}
#endif

#endif