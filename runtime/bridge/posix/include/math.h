#ifndef KZ_POSIX_MATH_H_
#define KZ_POSIX_MATH_H_

#include <float.h>

#if FLT_EVAL_METHOD == 0
typedef float float_t;
typedef double double_t;
#elif FLT_EVAL_METHOD == 1
typedef double float_t;
typedef double double_t;
#elif FLT_EVAL_METHOD == 2
typedef long double float_t;
typedef long double double_t;
#else
typedef double float_t;
typedef double double_t;
#endif

#define HUGE_VAL __builtin_huge_val()
#define HUGE_VALF __builtin_huge_valf()
#define HUGE_VALL __builtin_huge_vall()
#define INFINITY __builtin_inff()
#define NAN __builtin_nanf("")

#define FP_INFINITE 1
#define FP_NAN 0
#define FP_NORMAL 4
#define FP_SUBNORMAL 3
#define FP_ZERO 2

#define MATH_ERRNO 1
#define MATH_ERREXCEPT 2
#define math_errhandling MATH_ERRNO
#define FP_ILOGB0 (-2147483647 - 1)
#define FP_ILOGBNAN (-2147483647 - 1)

#define fpclassify(value) \
    __builtin_fpclassify(FP_NAN, FP_INFINITE, FP_NORMAL, FP_SUBNORMAL, FP_ZERO, (value))
#define isfinite(value) __builtin_isfinite(value)
#define isinf(value) __builtin_isinf(value)
#define isnan(value) __builtin_isnan(value)
#define isnormal(value) __builtin_isnormal(value)
#define signbit(value) __builtin_signbit(value)
#define isgreater(left, right) __builtin_isgreater((left), (right))
#define isgreaterequal(left, right) __builtin_isgreaterequal((left), (right))
#define isless(left, right) __builtin_isless((left), (right))
#define islessequal(left, right) __builtin_islessequal((left), (right))
#define islessgreater(left, right) __builtin_islessgreater((left), (right))
#define isunordered(left, right) __builtin_isunordered((left), (right))

#define M_E 2.71828182845904523536
#define M_EGAMMA 0.57721566490153286061
#define M_LOG2E 1.44269504088896340736
#define M_LOG10E 0.434294481903251827651
#define M_LN2 0.693147180559945309417
#define M_LN10 2.30258509299404568402
#define M_PHI 1.61803398874989484820
#define M_PI 3.14159265358979323846
#define M_PI_2 1.57079632679489661923
#define M_PI_4 0.785398163397448309616
#define M_1_PI 0.318309886183790671538
#define M_1_SQRTPI 0.564189583547756286948
#define M_2_PI 0.636619772367581343076
#define M_2_SQRTPI 1.12837916709551257390
#define M_SQRT2 1.41421356237309504880
#define M_SQRT3 1.73205080756887729353
#define M_SQRT1_2 0.707106781186547524401
#define M_SQRT1_3 0.577350269189625764509

#define M_El ((long double)2.718281828459045235360287471352662498L)
#define M_EGAMMAl ((long double)0.577215664901532860606512090082402431L)
#define M_LOG2El ((long double)1.442695040888963407359924681001892137L)
#define M_LOG10El ((long double)0.434294481903251827651128918916605082L)
#define M_LN2l ((long double)0.693147180559945309417232121458176568L)
#define M_LN10l ((long double)2.302585092994045684017991454684364208L)
#define M_PHIl ((long double)1.618033988749894848204586834365638118L)
#define M_PIl ((long double)3.141592653589793238462643383279502884L)
#define M_PI_2l ((long double)1.570796326794896619231321691639751442L)
#define M_PI_4l ((long double)0.785398163397448309615660845819875721L)
#define M_1_PIl ((long double)0.318309886183790671537767526745028724L)
#define M_1_SQRTPIl ((long double)0.564189583547756286948079451560772586L)
#define M_2_PIl ((long double)0.636619772367581343075535053490057448L)
#define M_2_SQRTPIl ((long double)1.12837916709551257389615890312154517L)
#define M_SQRT2l ((long double)1.414213562373095048801688724209698079L)
#define M_SQRT3l ((long double)1.732050807568877293527446341505872367L)
#define M_SQRT1_2l ((long double)0.707106781186547524400844362104849039L)
#define M_SQRT1_3l ((long double)0.577350269189625764509148780501957456L)

#ifdef __cplusplus
extern "C" {
#endif

#define KZ_POSIX_MATH_UNARY(name) \
    double name(double value); \
    float name##f(float value); \
    long double name##l(long double value)

#define KZ_POSIX_MATH_BINARY(name) \
    double name(double left, double right); \
    float name##f(float left, float right); \
    long double name##l(long double left, long double right)

KZ_POSIX_MATH_UNARY(acos);
KZ_POSIX_MATH_UNARY(acosh);
KZ_POSIX_MATH_UNARY(asin);
KZ_POSIX_MATH_UNARY(asinh);
KZ_POSIX_MATH_UNARY(atan);
KZ_POSIX_MATH_UNARY(atanh);
KZ_POSIX_MATH_UNARY(cbrt);
KZ_POSIX_MATH_UNARY(ceil);
KZ_POSIX_MATH_UNARY(cos);
KZ_POSIX_MATH_UNARY(cosh);
KZ_POSIX_MATH_UNARY(erf);
KZ_POSIX_MATH_UNARY(erfc);
KZ_POSIX_MATH_UNARY(exp);
KZ_POSIX_MATH_UNARY(exp2);
KZ_POSIX_MATH_UNARY(expm1);
KZ_POSIX_MATH_UNARY(fabs);
KZ_POSIX_MATH_UNARY(floor);
KZ_POSIX_MATH_UNARY(log);
KZ_POSIX_MATH_UNARY(log10);
KZ_POSIX_MATH_UNARY(log1p);
KZ_POSIX_MATH_UNARY(log2);
KZ_POSIX_MATH_UNARY(logb);
KZ_POSIX_MATH_UNARY(nearbyint);
KZ_POSIX_MATH_UNARY(rint);
KZ_POSIX_MATH_UNARY(round);
KZ_POSIX_MATH_UNARY(sin);
KZ_POSIX_MATH_UNARY(sinh);
KZ_POSIX_MATH_UNARY(sqrt);
KZ_POSIX_MATH_UNARY(tan);
KZ_POSIX_MATH_UNARY(tanh);
KZ_POSIX_MATH_UNARY(tgamma);
KZ_POSIX_MATH_UNARY(trunc);
KZ_POSIX_MATH_UNARY(lgamma);

KZ_POSIX_MATH_BINARY(atan2);
KZ_POSIX_MATH_BINARY(copysign);
KZ_POSIX_MATH_BINARY(fdim);
KZ_POSIX_MATH_BINARY(fmax);
KZ_POSIX_MATH_BINARY(fmin);
KZ_POSIX_MATH_BINARY(fmod);
KZ_POSIX_MATH_BINARY(hypot);
KZ_POSIX_MATH_BINARY(nextafter);
KZ_POSIX_MATH_BINARY(pow);
KZ_POSIX_MATH_BINARY(remainder);

double fma(double x, double y, double z);
float fmaf(float x, float y, float z);
long double fmal(long double x, long double y, long double z);
double frexp(double value, int *exponent);
float frexpf(float value, int *exponent);
long double frexpl(long double value, int *exponent);
double ldexp(double value, int exponent);
float ldexpf(float value, int exponent);
long double ldexpl(long double value, int exponent);
double modf(double value, double *integral_part);
float modff(float value, float *integral_part);
long double modfl(long double value, long double *integral_part);
double nan(const char *tag);
float nanf(const char *tag);
long double nanl(const char *tag);
double nexttoward(double value, long double direction);
float nexttowardf(float value, long double direction);
long double nexttowardl(long double value, long double direction);
double remquo(double x, double y, int *quotient);
float remquof(float x, float y, int *quotient);
long double remquol(long double x, long double y, int *quotient);
double scalbln(double value, long exponent);
float scalblnf(float value, long exponent);
long double scalblnl(long double value, long exponent);
double scalbn(double value, int exponent);
float scalbnf(float value, int exponent);
long double scalbnl(long double value, int exponent);
int ilogb(double value);
int ilogbf(float value);
int ilogbl(long double value);
long lrint(double value);
long lrintf(float value);
long lrintl(long double value);
long long llrint(double value);
long long llrintf(float value);
long long llrintl(long double value);
long lround(double value);
long lroundf(float value);
long lroundl(long double value);
long long llround(double value);
long long llroundf(float value);
long long llroundl(long double value);

double j0(double value);
double j1(double value);
double jn(int order, double value);
double y0(double value);
double y1(double value);
double yn(int order, double value);
extern int signgam;

#undef KZ_POSIX_MATH_UNARY
#undef KZ_POSIX_MATH_BINARY

#ifdef __cplusplus
}
#endif

#endif