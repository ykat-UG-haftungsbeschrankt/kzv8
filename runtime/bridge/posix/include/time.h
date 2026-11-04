#ifndef KZ_POSIX_TIME_H_
#define KZ_POSIX_TIME_H_

#include <locale.h>
#include <sys/types.h>

struct sigevent;

struct timespec {
    time_t tv_sec;
    long tv_nsec;
};

struct itimerspec {
    struct timespec it_interval;
    struct timespec it_value;
};

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
    const char *tm_zone;
};

#define CLOCKS_PER_SEC 1000000
#define TIME_UTC 1
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCK_PROCESS_CPUTIME_ID 2
#define CLOCK_THREAD_CPUTIME_ID 3
#define CLOCK_MONOTONIC_RAW 4
#define TIMER_ABSTIME 1

#ifdef __cplusplus
extern "C" {
#define KZ_POSIX_RESTRICT __restrict
#else
#define KZ_POSIX_RESTRICT restrict
#endif

char *asctime(const struct tm *time_value);
clock_t clock(void);
int clock_getcpuclockid(pid_t process_id, clockid_t *clock_id);
int clock_getres(clockid_t clock_id, struct timespec *resolution);
int clock_gettime(clockid_t clock_id, struct timespec *time_value);
int clock_nanosleep(clockid_t clock_id, int flags,
                    const struct timespec *request, struct timespec *remaining);
int clock_settime(clockid_t clock_id, const struct timespec *time_value);
char *ctime(const time_t *time_value);
double difftime(time_t end_time, time_t start_time);
struct tm *gmtime(const time_t *time_value);
struct tm *gmtime_r(const time_t *KZ_POSIX_RESTRICT time_value,
                    struct tm *KZ_POSIX_RESTRICT result);
struct tm *localtime(const time_t *time_value);
struct tm *localtime_r(const time_t *KZ_POSIX_RESTRICT time_value,
                       struct tm *KZ_POSIX_RESTRICT result);
time_t mktime(struct tm *time_value);
int nanosleep(const struct timespec *request, struct timespec *remaining);
size_t strftime(char *KZ_POSIX_RESTRICT output, size_t output_size,
                const char *KZ_POSIX_RESTRICT format,
                const struct tm *KZ_POSIX_RESTRICT time_value);
size_t strftime_l(char *KZ_POSIX_RESTRICT output, size_t output_size,
                  const char *KZ_POSIX_RESTRICT format,
                  const struct tm *KZ_POSIX_RESTRICT time_value, locale_t locale);
char *strptime(const char *KZ_POSIX_RESTRICT input,
               const char *KZ_POSIX_RESTRICT format,
               struct tm *KZ_POSIX_RESTRICT result);
time_t time(time_t *result);
int timer_create(clockid_t clock_id,
                 struct sigevent *KZ_POSIX_RESTRICT event,
                 timer_t *KZ_POSIX_RESTRICT timer);
int timer_delete(timer_t timer);
int timer_getoverrun(timer_t timer);
int timer_gettime(timer_t timer, struct itimerspec *value);
int timer_settime(timer_t timer, int flags,
                  const struct itimerspec *KZ_POSIX_RESTRICT value,
                  struct itimerspec *KZ_POSIX_RESTRICT old_value);
int timespec_get(struct timespec *value, int base);
void tzset(void);

#undef KZ_POSIX_RESTRICT
#ifdef __cplusplus
}
#endif

#endif