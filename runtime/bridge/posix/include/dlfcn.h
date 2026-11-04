#ifndef KZ_POSIX_DLFCN_H_
#define KZ_POSIX_DLFCN_H_

#define RTLD_LAZY 0x0001
#define RTLD_NOW 0x0002
#define RTLD_GLOBAL 0x0100
#define RTLD_LOCAL 0x0000

typedef struct {
    const char *dli_fname;
    void *dli_fbase;
    const char *dli_sname;
    void *dli_saddr;
} Dl_info_t;

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

int dladdr(const void *KZ_POSIX_RESTRICT address,
           Dl_info_t *KZ_POSIX_RESTRICT information);
int dlclose(void *handle);
char *dlerror(void);
void *dlopen(const char *file, int mode);
void *dlsym(void *KZ_POSIX_RESTRICT handle,
            const char *KZ_POSIX_RESTRICT symbol);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif