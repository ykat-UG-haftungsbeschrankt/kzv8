#ifndef KZ_POSIX_COMPLEX_H_
#define KZ_POSIX_COMPLEX_H_

#define complex _Complex
#define _Complex_I __builtin_complex(0.0F, 1.0F)
#define I _Complex_I

#define CMPLX(real_part, imaginary_part) \
    __builtin_complex((double)(real_part), (double)(imaginary_part))
#define CMPLXF(real_part, imaginary_part) \
    __builtin_complex((float)(real_part), (float)(imaginary_part))
#define CMPLXL(real_part, imaginary_part) \
    __builtin_complex((long double)(real_part), (long double)(imaginary_part))

#ifdef __cplusplus
extern "C" {
#endif

double cabs(double complex value);
float cabsf(float complex value);
long double cabsl(long double complex value);
double complex cacos(double complex value);
float complex cacosf(float complex value);
long double complex cacosl(long double complex value);
double complex cacosh(double complex value);
float complex cacoshf(float complex value);
long double complex cacoshl(long double complex value);
double carg(double complex value);
float cargf(float complex value);
long double cargl(long double complex value);
double complex casin(double complex value);
float complex casinf(float complex value);
long double complex casinl(long double complex value);
double complex casinh(double complex value);
float complex casinhf(float complex value);
long double complex casinhl(long double complex value);
double complex catan(double complex value);
float complex catanf(float complex value);
long double complex catanl(long double complex value);
double complex catanh(double complex value);
float complex catanhf(float complex value);
long double complex catanhl(long double complex value);
double complex ccos(double complex value);
float complex ccosf(float complex value);
long double complex ccosl(long double complex value);
double complex ccosh(double complex value);
float complex ccoshf(float complex value);
long double complex ccoshl(long double complex value);
double complex cexp(double complex value);
float complex cexpf(float complex value);
long double complex cexpl(long double complex value);
double cimag(double complex value);
float cimagf(float complex value);
long double cimagl(long double complex value);
double complex clog(double complex value);
float complex clogf(float complex value);
long double complex clogl(long double complex value);
double complex conj(double complex value);
float complex conjf(float complex value);
long double complex conjl(long double complex value);
double complex cpow(double complex base, double complex exponent);
float complex cpowf(float complex base, float complex exponent);
long double complex cpowl(long double complex base, long double complex exponent);
double complex cproj(double complex value);
float complex cprojf(float complex value);
long double complex cprojl(long double complex value);
double creal(double complex value);
float crealf(float complex value);
long double creall(long double complex value);
double complex csin(double complex value);
float complex csinf(float complex value);
long double complex csinl(long double complex value);
double complex csinh(double complex value);
float complex csinhf(float complex value);
long double complex csinhl(long double complex value);
double complex csqrt(double complex value);
float complex csqrtf(float complex value);
long double complex csqrtl(long double complex value);
double complex ctan(double complex value);
float complex ctanf(float complex value);
long double complex ctanl(long double complex value);
double complex ctanh(double complex value);
float complex ctanhf(float complex value);
long double complex ctanhl(long double complex value);

#ifdef __cplusplus
}
#endif

#endif