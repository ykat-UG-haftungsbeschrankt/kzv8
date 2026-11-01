#include <stdint.h>

double nearbyint(double value) {
    if (value != value || value >= 0x1p52 || value <= -0x1p52) return value;
    const long long integral = (long long)value;
    const double fraction = value - (double)integral;
    if (fraction > 0.5 || (fraction == 0.5 && (integral & 1) != 0)) {
        return (double)(integral + 1);
    }
    if (fraction < -0.5 || (fraction == -0.5 && (integral & 1) != 0)) {
        return (double)(integral - 1);
    }
    return (double)integral;
}

float nearbyintf(float value) {
    if (value != value || value >= 0x1p23f || value <= -0x1p23f) return value;
    const int integral = (int)value;
    const float fraction = value - (float)integral;
    if (fraction > 0.5f || (fraction == 0.5f && (integral & 1) != 0)) {
        return (float)(integral + 1);
    }
    if (fraction < -0.5f || (fraction == -0.5f && (integral & 1) != 0)) {
        return (float)(integral - 1);
    }
    return (float)integral;
}

double modf(double value, double* integral_part) {
    if (integral_part == 0) return value;
    if (value != value || value >= 0x1p63 || value < -0x1p63) {
        *integral_part = value;
        return 0.0;
    }
    const long long integral = (long long)value;
    *integral_part = (double)integral;
    return value - *integral_part;
}

float modff(float value, float* integral_part) {
    if (integral_part == 0) return value;
    if (value != value || value >= 0x1p31f || value < -0x1p31f) {
        *integral_part = value;
        return 0.0f;
    }
    const int integral = (int)value;
    *integral_part = (float)integral;
    return value - *integral_part;
}

double nextafter(double value, double direction) {
    if (value != value || direction != direction) return value + direction;
    if (value == direction) return direction;
    if (value == 0.0) {
        const uint64_t sign = direction < 0.0 ? 1ULL << 63 : 0;
        union { uint64_t bits; double value; } result = {1 | sign};
        return result.value;
    }
    union { uint64_t bits; double value; } result = {.value = value};
    if ((value < direction) == (value > 0.0)) result.bits++;
    else result.bits--;
    return result.value;
}

float nextafterf(float value, float direction) {
    if (value != value || direction != direction) return value + direction;
    if (value == direction) return direction;
    if (value == 0.0f) {
        const uint32_t sign = direction < 0.0f ? 1U << 31 : 0;
        union { uint32_t bits; float value; } result = {1U | sign};
        return result.value;
    }
    union { uint32_t bits; float value; } result = {.value = value};
    if ((value < direction) == (value > 0.0f)) result.bits++;
    else result.bits--;
    return result.value;
}