#ifndef KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_TIME_H_
#define KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_TIME_H_

struct timespec {
    long tv_sec;
    long tv_nsec;
};

typedef long time_t;
struct tm {
    int tm_sec;
    int tm_min;
    int tm_hour;
    int tm_mday;
    int tm_mon;
    int tm_year;
    int tm_wday;
    int tm_yday;
    int tm_isdst;
    long tm_gmtoff;
    const char* tm_zone;
};

#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1

#ifdef __cplusplus
extern "C" {
#endif

int clock_gettime(int clock_id, struct timespec* time);
struct tm* localtime_r(const time_t* time, struct tm* result);

#ifdef __cplusplus
}
#endif

#endif