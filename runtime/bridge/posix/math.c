#include <complex.h>
#include <errno.h>
#include <math.h>

static long double unsupported_long_double(void) {
    errno = ENOSYS;
    return __builtin_nanl("");
}

static double unsupported_double(void) {
    errno = ENOSYS;
    return __builtin_nan("");
}

static float unsupported_float(void) {
    errno = ENOSYS;
    return __builtin_nanf("");
}

static void unsupported_math(void) {
    errno = ENOSYS;
}

#define DEFINE_UNARY_LONG(name) \
    long double name##l(long double value) { \
        (void)value; \
        return unsupported_long_double(); \
    }

#define DEFINE_BINARY_LONG(name) \
    long double name##l(long double left, long double right) { \
        (void)left; \
        (void)right; \
        return unsupported_long_double(); \
    }

DEFINE_UNARY_LONG(acos)
DEFINE_UNARY_LONG(acosh)
DEFINE_UNARY_LONG(asin)
DEFINE_UNARY_LONG(asinh)
DEFINE_UNARY_LONG(atan)
DEFINE_UNARY_LONG(atanh)
DEFINE_UNARY_LONG(cbrt)
DEFINE_UNARY_LONG(ceil)
DEFINE_UNARY_LONG(cos)
DEFINE_UNARY_LONG(cosh)
DEFINE_UNARY_LONG(erf)
DEFINE_UNARY_LONG(erfc)
DEFINE_UNARY_LONG(exp)
DEFINE_UNARY_LONG(exp2)
DEFINE_UNARY_LONG(expm1)
DEFINE_UNARY_LONG(fabs)
DEFINE_UNARY_LONG(floor)
DEFINE_UNARY_LONG(log)
DEFINE_UNARY_LONG(log10)
DEFINE_UNARY_LONG(log1p)
DEFINE_UNARY_LONG(log2)
DEFINE_UNARY_LONG(logb)
DEFINE_UNARY_LONG(nearbyint)
DEFINE_UNARY_LONG(rint)
DEFINE_UNARY_LONG(round)
DEFINE_UNARY_LONG(sin)
DEFINE_UNARY_LONG(sinh)
DEFINE_UNARY_LONG(sqrt)
DEFINE_UNARY_LONG(tan)
DEFINE_UNARY_LONG(tanh)
DEFINE_UNARY_LONG(tgamma)
DEFINE_UNARY_LONG(trunc)
DEFINE_UNARY_LONG(lgamma)

DEFINE_BINARY_LONG(atan2)
DEFINE_BINARY_LONG(copysign)
DEFINE_BINARY_LONG(fdim)
DEFINE_BINARY_LONG(fmax)
DEFINE_BINARY_LONG(fmin)
DEFINE_BINARY_LONG(fmod)
DEFINE_BINARY_LONG(hypot)
DEFINE_BINARY_LONG(nextafter)
DEFINE_BINARY_LONG(pow)
DEFINE_BINARY_LONG(remainder)

long double fmal(long double x, long double y, long double z) {
    (void)x;
    (void)y;
    (void)z;
    return unsupported_long_double();
}

long double frexpl(long double value, int *exponent) {
    (void)value;
    if (exponent != 0) {
        *exponent = 0;
    }
    return unsupported_long_double();
}

long double ldexpl(long double value, int exponent) {
    (void)value;
    (void)exponent;
    return unsupported_long_double();
}

long double modfl(long double value, long double *integral_part) {
    (void)value;
    if (integral_part != 0) {
        *integral_part = unsupported_long_double();
    }
    return unsupported_long_double();
}

long double nanl(const char *tag) {
    (void)tag;
    return __builtin_nanl("");
}

double nexttoward(double value, long double direction) {
    (void)value;
    (void)direction;
    return unsupported_double();
}

float nexttowardf(float value, long double direction) {
    (void)value;
    (void)direction;
    return unsupported_float();
}

long double nexttowardl(long double value, long double direction) {
    (void)value;
    (void)direction;
    return unsupported_long_double();
}

long double remquol(long double x, long double y, int *quotient) {
    (void)x;
    (void)y;
    if (quotient != 0) {
        *quotient = 0;
    }
    return unsupported_long_double();
}

long double scalblnl(long double value, long exponent) {
    (void)value;
    (void)exponent;
    return unsupported_long_double();
}

long double scalbnl(long double value, int exponent) {
    (void)value;
    (void)exponent;
    return unsupported_long_double();
}

int ilogbl(long double value) {
    (void)value;
    unsupported_math();
    return 0;
}

long lrintl(long double value) {
    (void)value;
    unsupported_math();
    return 0;
}

long long llrintl(long double value) {
    (void)value;
    unsupported_math();
    return 0;
}

long lroundl(long double value) {
    (void)value;
    unsupported_math();
    return 0;
}

long long llroundl(long double value) {
    (void)value;
    unsupported_math();
    return 0;
}

#define DEFINE_COMPLEX_UNARY(name) \
    double _Complex name(double _Complex value) { \
        (void)value; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nan(""), __builtin_nan("")); \
    } \
    float _Complex name##f(float _Complex value) { \
        (void)value; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nanf(""), __builtin_nanf("")); \
    } \
    long double _Complex name##l(long double _Complex value) { \
        (void)value; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nanl(""), __builtin_nanl("")); \
    }

#define DEFINE_COMPLEX_BINARY(name) \
    double _Complex name(double _Complex left, double _Complex right) { \
        (void)left; \
        (void)right; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nan(""), __builtin_nan("")); \
    } \
    float _Complex name##f(float _Complex left, float _Complex right) { \
        (void)left; \
        (void)right; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nanf(""), __builtin_nanf("")); \
    } \
    long double _Complex name##l(long double _Complex left, long double _Complex right) { \
        (void)left; \
        (void)right; \
        errno = ENOSYS; \
        return __builtin_complex(__builtin_nanl(""), __builtin_nanl("")); \
    }

DEFINE_COMPLEX_UNARY(cacos)
DEFINE_COMPLEX_UNARY(cacosh)
DEFINE_COMPLEX_UNARY(casin)
DEFINE_COMPLEX_UNARY(casinh)
DEFINE_COMPLEX_UNARY(catan)
DEFINE_COMPLEX_UNARY(catanh)
DEFINE_COMPLEX_UNARY(ccos)
DEFINE_COMPLEX_UNARY(ccosh)
DEFINE_COMPLEX_UNARY(cexp)
DEFINE_COMPLEX_UNARY(clog)
DEFINE_COMPLEX_BINARY(cpow)
DEFINE_COMPLEX_UNARY(csin)
DEFINE_COMPLEX_UNARY(csinh)
DEFINE_COMPLEX_UNARY(csqrt)
DEFINE_COMPLEX_UNARY(ctan)
DEFINE_COMPLEX_UNARY(ctanh)

#define DEFINE_COMPLEX_SCALAR(name) \
    double name(double _Complex value) { (void)value; return unsupported_double(); } \
    float name##f(float _Complex value) { (void)value; return unsupported_float(); } \
    long double name##l(long double _Complex value) { \
        (void)value; \
        return unsupported_long_double(); \
    }

DEFINE_COMPLEX_SCALAR(cabs)
DEFINE_COMPLEX_SCALAR(carg)

#undef DEFINE_COMPLEX_SCALAR

#define DEFINE_COMPLEX_COMPONENT(name, operation) \
    double name(double _Complex value) { return operation value; } \
    float name##f(float _Complex value) { return operation value; } \
    long double name##l(long double _Complex value) { return operation value; }

#define REAL_PART __real__
#define IMAGINARY_PART __imag__

DEFINE_COMPLEX_COMPONENT(creal, REAL_PART)
DEFINE_COMPLEX_COMPONENT(cimag, IMAGINARY_PART)

#undef DEFINE_COMPLEX_COMPONENT
#undef REAL_PART
#undef IMAGINARY_PART

#define DEFINE_COMPLEX_CONJUGATE(name) \
    double _Complex name(double _Complex value) { \
        return __builtin_complex(__real__ value, -__imag__ value); \
    } \
    float _Complex name##f(float _Complex value) { \
        return __builtin_complex(__real__ value, -__imag__ value); \
    } \
    long double _Complex name##l(long double _Complex value) { \
        return __builtin_complex(__real__ value, -__imag__ value); \
    }

DEFINE_COMPLEX_CONJUGATE(conj)

#undef DEFINE_COMPLEX_CONJUGATE

#define DEFINE_COMPLEX_PROJECTION(name, type, infinity, copysign_fn) \
    type _Complex name(type _Complex value) { \
        if (__builtin_isinf(__real__ value) || __builtin_isinf(__imag__ value)) { \
            return __builtin_complex(infinity, copysign_fn((type)0, __imag__ value)); \
        } \
        return value; \
    }

DEFINE_COMPLEX_PROJECTION(cproj, double, __builtin_inf(), __builtin_copysign)
DEFINE_COMPLEX_PROJECTION(cprojf, float, __builtin_inff(), __builtin_copysignf)
DEFINE_COMPLEX_PROJECTION(cprojl, long double, __builtin_infl(), __builtin_copysignl)

#undef DEFINE_COMPLEX_PROJECTION
