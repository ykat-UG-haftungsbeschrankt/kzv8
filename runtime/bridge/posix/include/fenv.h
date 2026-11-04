#ifndef KZ_POSIX_FENV_H_
#define KZ_POSIX_FENV_H_

#include <stdint.h>

typedef struct {
    uint64_t __opaque[4];
} fenv_t;
typedef uint32_t fexcept_t;

#define FE_INVALID 0x01
#define FE_DIVBYZERO 0x04
#define FE_OVERFLOW 0x08
#define FE_UNDERFLOW 0x10
#define FE_INEXACT 0x20
#define FE_ALL_EXCEPT 0x3d

#define FE_TONEAREST 0x000
#define FE_DOWNWARD 0x400
#define FE_UPWARD 0x800
#define FE_TOWARDZERO 0xc00

#define FE_DFL_ENV ((const fenv_t *)-1)

#ifdef __cplusplus
extern "C" {
#endif

int feclearexcept(int exceptions);
int fegetenv(fenv_t *environment);
int fegetexceptflag(fexcept_t *flag, int exceptions);
int fegetround(void);
int feholdexcept(fenv_t *environment);
int feraiseexcept(int exceptions);
int fesetenv(const fenv_t *environment);
int fesetexceptflag(const fexcept_t *flag, int exceptions);
int fesetround(int rounding_direction);
int fetestexcept(int exceptions);
int feupdateenv(const fenv_t *environment);

#ifdef __cplusplus
}
#endif

#endif