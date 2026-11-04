#ifndef KZ_POSIX_SETJMP_H_
#define KZ_POSIX_SETJMP_H_

#include <stdint.h>

typedef struct {
    uintptr_t __opaque[32];
} __kz_jmp_buf[1];
typedef __kz_jmp_buf jmp_buf;

typedef struct {
    uintptr_t __opaque[34];
} __kz_sigjmp_buf[1];
typedef __kz_sigjmp_buf sigjmp_buf;

#ifdef __cplusplus
extern "C" {
#endif

int setjmp(jmp_buf environment) __attribute__((returns_twice));
_Noreturn void longjmp(jmp_buf environment, int value);
int sigsetjmp(sigjmp_buf environment, int save_signal_mask) __attribute__((returns_twice));
_Noreturn void siglongjmp(sigjmp_buf environment, int value);

#ifdef __cplusplus
}
#endif

#endif