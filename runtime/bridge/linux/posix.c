#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>

#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>
#include <sys/uio.h>
#include <sys/mman.h>

#define SYS_GETPID 39
#define SYS_SCHED_YIELD 24
#define SYS_FUTEX 202
#define SYS_GETTID 186
#define SYS_CLOCK_GETTIME 228
#define FUTEX_WAIT 0
#define FUTEX_WAKE 1
#define FUTEX_PRIVATE_FLAG 128
#define ENOSYS 38
#define EBADF 9
#define EFAULT 14
#define ERANGE 34
#define EIO 5
#define ENOENT 2
#define EOVERFLOW 75
#define EAGAIN 11
#define EBUSY 16
#define SIGSET_SIZE 128
#define PR_SET_NAME 15
#define PR_GET_NAME 16
#define PR_SET_VMA 0x53564d41
#define PR_SET_VMA_ANON_NAME 0
#define MREMAP_MAYMOVE 1

extern int kz_host_console_write(const char* data, size_t length);
extern uint64_t kz_host_time_ns(void);
extern int64_t kz_host_wall_time_ms(void);

int clock_gettime(int clock_id, struct timespec* time) {
    if (time == NULL) {
        errno = EFAULT;
        return -1;
    }
    int64_t milliseconds;
    if (clock_id == CLOCK_MONOTONIC) {
        uint64_t nanoseconds = kz_host_time_ns();
        time->tv_sec = (long)(nanoseconds / 1000000000ULL);
        time->tv_nsec = (long)(nanoseconds % 1000000000ULL);
        return 0;
    }
    if (clock_id == CLOCK_REALTIME) {
        milliseconds = kz_host_wall_time_ms();
        time->tv_sec = (long)(milliseconds / 1000);
        time->tv_nsec = (long)((milliseconds % 1000) * 1000000);
        return 0;
    }
    errno = EINVAL;
    return -1;
}

ssize_t write(int file_descriptor, const void* buffer, size_t length) {
    if (file_descriptor != 1 && file_descriptor != 2) {
        errno = EBADF;
        return -1;
    }
    if (buffer == NULL && length != 0) {
        errno = EFAULT;
        return -1;
    }
    const char* bytes = (const char*)buffer;
    size_t written = 0;
    while (written < length) {
        size_t chunk = length - written;
        if (chunk > 512) chunk = 512;
        int result = kz_host_console_write(bytes + written, chunk);
        if (result <= 0) {
            if (written == 0) {
                errno = EIO;
                return -1;
            }
            break;
        }
        written += (size_t)result;
    }
    return (ssize_t)written;
}

ssize_t writev(int file_descriptor, const struct iovec* vectors, int count) {
    if (count < 0 || (vectors == NULL && count != 0)) {
        errno = EINVAL;
        return -1;
    }
    ssize_t total = 0;
    for (int index = 0; index < count; index++) {
        ssize_t result = write(file_descriptor, vectors[index].iov_base,
                               vectors[index].iov_len);
        if (result < 0) return total == 0 ? -1 : total;
        total += result;
        if ((size_t)result != vectors[index].iov_len) break;
    }
    return total;
}

char* getcwd(char* buffer, size_t size) {
    if (buffer == NULL) {
        errno = EFAULT;
        return NULL;
    }
    if (size < 2) {
        errno = ERANGE;
        return NULL;
    }
    buffer[0] = '/';
    buffer[1] = '\0';
    return buffer;
}

long sysconf(int name) {
    switch (name) {
        case _SC_PAGESIZE:
            return 4096;
        case _SC_NPROCESSORS_CONF:
        case _SC_NPROCESSORS_ONLN:
            return 1;
        case _SC_CLK_TCK:
            return 100;
        default:
            errno = EINVAL;
            return -1;
    }
}

long getpagesize(void) { return 4096; }
int getpid(void) { return 1; }

int sigfillset(void* set) {
    if (set == NULL) return EFAULT;
    memset(set, 0xff, SIGSET_SIZE);
    return 0;
}

int pthread_sigmask(int how, const void* set, void* old_set) {
    (void)set;
    if (how != 0 && how != 1 && how != 2) return EINVAL;
    if (old_set != NULL) memset(old_set, 0, SIGSET_SIZE);
    return 0;
}

int nanosleep(const struct timespec* request, struct timespec* remaining) {
    if (request == NULL || request->tv_sec < 0 || request->tv_nsec < 0 ||
        request->tv_nsec >= 1000000000L) {
        errno = EINVAL;
        return -1;
    }
    const uint64_t now = kz_host_time_ns();
    const uint64_t seconds = (uint64_t)request->tv_sec;
    uint64_t duration = UINT64_MAX;
    if (seconds <= (UINT64_MAX - (uint64_t)request->tv_nsec) / 1000000000ULL) {
        duration = seconds * 1000000000ULL + (uint64_t)request->tv_nsec;
    }
    const uint64_t deadline = duration > UINT64_MAX - now
                                  ? UINT64_MAX
                                  : now + duration;
    while (kz_host_time_ns() < deadline) pthread_yield();
    if (remaining != NULL) {
        remaining->tv_sec = 0;
        remaining->tv_nsec = 0;
    }
    return 0;
}

int usleep(unsigned int microseconds) {
    struct timespec request = {
        (long)(microseconds / 1000000U),
        (long)(microseconds % 1000000U) * 1000L,
    };
    return nanosleep(&request, NULL);
}

int prctl(int option, ...) {
    va_list arguments;
    va_start(arguments, option);
    switch (option) {
        case PR_SET_NAME:
            (void)va_arg(arguments, const char*);
            break;
        case PR_GET_NAME: {
            char* name = va_arg(arguments, char*);
            if (name == NULL) {
                va_end(arguments);
                errno = EFAULT;
                return -1;
            }
            memcpy(name, "kz", 3);
            break;
        }
        case PR_SET_VMA:
            (void)va_arg(arguments, unsigned long);
            (void)va_arg(arguments, void*);
            (void)va_arg(arguments, unsigned long);
            (void)va_arg(arguments, const char*);
            break;
        default:
            va_end(arguments);
            errno = EINVAL;
            return -1;
    }
    va_end(arguments);
    return 0;
}

void _exit(int status) {
    (void)status;
    __builtin_trap();
}

int sem_init(void* semaphore, int process_shared, unsigned int value) {
    if (semaphore == NULL || process_shared != 0 || value > 0x7fffffffU) {
        return EINVAL;
    }
    __atomic_store_n((uint32_t*)semaphore, value, __ATOMIC_RELEASE);
    return 0;
}

int sem_destroy(void* semaphore) { return semaphore == NULL ? EINVAL : 0; }

int sem_post(void* semaphore) {
    if (semaphore == NULL) return EINVAL;
    uint32_t* count = (uint32_t*)semaphore;
    uint32_t current = __atomic_load_n(count, __ATOMIC_RELAXED);
    for (;;) {
        if (current >= 0x7fffffffU) return EOVERFLOW;
        if (__atomic_compare_exchange_n(count, &current, current + 1, 0,
                                        __ATOMIC_RELEASE, __ATOMIC_RELAXED)) {
            return 0;
        }
    }
}

int sem_wait(void* semaphore) {
    if (semaphore == NULL) return EINVAL;
    uint32_t* count = (uint32_t*)semaphore;
    for (;;) {
        uint32_t current = __atomic_load_n(count, __ATOMIC_ACQUIRE);
        if (current != 0 && __atomic_compare_exchange_n(
                                count, &current, current - 1, 0,
                                __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            return 0;
        }
        pthread_yield();
    }
}

int sem_trywait(void* semaphore) {
    if (semaphore == NULL) return EINVAL;
    uint32_t* count = (uint32_t*)semaphore;
    uint32_t current = __atomic_load_n(count, __ATOMIC_ACQUIRE);
    while (current != 0) {
        if (__atomic_compare_exchange_n(count, &current, current - 1, 0,
                                        __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            return 0;
        }
    }
    errno = EAGAIN;
    return -1;
}

FILE* fopen64(const char* path, const char* mode) {
    (void)path;
    (void)mode;
    errno = ENOENT;
    return NULL;
}

size_t fread(void* buffer, size_t size, size_t count, FILE* stream) {
    (void)buffer;
    (void)size;
    (void)count;
    (void)stream;
    return 0;
}

int fileno(FILE* stream) {
    if (stream == stdout) return 1;
    if (stream == stderr) return 2;
    errno = EBADF;
    return -1;
}

int fstat64(int file_descriptor, void* status) {
    (void)file_descriptor;
    (void)status;
    errno = ENOSYS;
    return -1;
}

struct tm* localtime_r(const time_t* time, struct tm* result) {
    if (time == NULL || result == NULL) {
        errno = EFAULT;
        return NULL;
    }
    memset(result, 0, sizeof(*result));
    result->tm_mday = 1;
    result->tm_year = 70;
    result->tm_wday = 4;
    result->tm_zone = "UTC";
    return result;
}

long syscall(long number, ...) {
    va_list arguments;
    va_start(arguments, number);
    long result;
    switch (number) {
        case SYS_GETPID:
            result = 1;
            break;
        case SYS_GETTID:
            result = (long)pthread_self();
            break;
        case SYS_SCHED_YIELD:
            result = pthread_yield();
            break;
        case SYS_CLOCK_GETTIME: {
            int clock_id = va_arg(arguments, int);
            struct timespec* time = va_arg(arguments, struct timespec*);
            result = clock_gettime(clock_id, time);
            break;
        }
        case SYS_FUTEX: {
            volatile uint32_t* address = va_arg(arguments, volatile uint32_t*);
            int operation = va_arg(arguments, int) & ~FUTEX_PRIVATE_FLAG;
            uint32_t expected = va_arg(arguments, uint32_t);
            if (address == NULL) {
                errno = EFAULT;
                result = -1;
            } else if (operation == FUTEX_WAKE) {
                result = expected == 0 ? 0 : 1;
            } else if (operation == FUTEX_WAIT && *address != expected) {
                errno = 11;
                result = -1;
            } else {
                result = 0;
            }
            break;
        }
        default:
            errno = ENOSYS;
            result = -1;
            break;
    }
    va_end(arguments);
    return result;
}

int __xpg_strerror_r(int error, char* buffer, size_t length) {
    if (buffer == NULL || length == 0) return ERANGE;
    const char* text = "unknown error";
    switch (error) {
        case 0: text = "success"; break;
        case EINVAL: text = "invalid argument"; break;
        case ENOMEM: text = "out of memory"; break;
        case EAGAIN: text = "try again"; break;
        case EBUSY: text = "resource busy"; break;
        case EIO: text = "I/O error"; break;
        case EBADF: text = "bad file descriptor"; break;
        case ENOSYS: text = "function not implemented"; break;
    }
    size_t index = 0;
    while (text[index] != '\0' && index + 1 < length) {
        buffer[index] = text[index];
        index++;
    }
    buffer[index] = '\0';
    return text[index] == '\0' ? 0 : ERANGE;
}

char* getenv(const char* name) {
    (void)name;
    return NULL;
}

static int digit_value(char character) {
    if (character >= '0' && character <= '9') return character - '0';
    if (character >= 'a' && character <= 'z') return character - 'a' + 10;
    if (character >= 'A' && character <= 'Z') return character - 'A' + 10;
    return -1;
}

static unsigned long long parse_unsigned(const char* text, char** end,
                                         int base) {
    const char* start = text;
    while (*text == ' ' || (*text >= '\t' && *text <= '\r')) text++;
    int negative = 0;
    if (*text == '+' || *text == '-') {
        negative = *text == '-';
        text++;
    }
    if ((base == 0 || base == 16) && text[0] == '0' &&
        (text[1] == 'x' || text[1] == 'X')) {
        base = 16;
        text += 2;
    } else if (base == 0) {
        base = text[0] == '0' ? 8 : 10;
    }
    if (base < 2 || base > 36) {
        errno = EINVAL;
        if (end != NULL) *end = (char*)start;
        return 0;
    }

    const char* digits = text;
    unsigned long long value = 0;
    int overflow = 0;
    for (;;) {
        const int digit = digit_value(*text);
        if (digit < 0 || digit >= base) break;
        if (value > (ULLONG_MAX - (unsigned)digit) / (unsigned)base) {
            overflow = 1;
            value = ULLONG_MAX;
        } else if (!overflow) {
            value = value * (unsigned)base + (unsigned)digit;
        }
        text++;
    }
    if (text == digits) {
        if (end != NULL) *end = (char*)start;
        return 0;
    }
    if (end != NULL) *end = (char*)text;
    if (overflow) errno = ERANGE;
    return negative ? 0ULL - value : value;
}

unsigned long long strtoull(const char* text, char** end, int base) {
    return parse_unsigned(text, end, base);
}

unsigned long strtoul(const char* text, char** end, int base) {
    const unsigned long long value = parse_unsigned(text, end, base);
    if (value > ULONG_MAX) {
        errno = ERANGE;
        return ULONG_MAX;
    }
    return (unsigned long)value;
}

unsigned long long __isoc23_strtoull(const char* text, char** end, int base) {
    return strtoull(text, end, base);
}

unsigned long __isoc23_strtoul(const char* text, char** end, int base) {
    return strtoul(text, end, base);
}

double strtod(const char* text, char** end) {
    const char* start = text;
    while (*text == ' ' || (*text >= '\t' && *text <= '\r')) text++;
    int negative = 0;
    if (*text == '+' || *text == '-') {
        negative = *text == '-';
        text++;
    }
    double value = 0.0;
    int digits = 0;
    while (*text >= '0' && *text <= '9') {
        value = value * 10.0 + (*text++ - '0');
        digits++;
    }
    if (*text == '.') {
        text++;
        double scale = 0.1;
        while (*text >= '0' && *text <= '9') {
            value += (*text++ - '0') * scale;
            scale *= 0.1;
            digits++;
        }
    }
    if (digits == 0) {
        if (end != NULL) *end = (char*)start;
        return 0.0;
    }
    int exponent = 0;
    int exponent_negative = 0;
    if (*text == 'e' || *text == 'E') {
        const char* exponent_start = text++;
        if (*text == '+' || *text == '-') {
            exponent_negative = *text == '-';
            text++;
        }
        if (*text < '0' || *text > '9') {
            text = exponent_start;
        } else {
            while (*text >= '0' && *text <= '9' && exponent < 400) {
                exponent = exponent * 10 + (*text++ - '0');
            }
        }
    }
    while (exponent-- > 0) value = exponent_negative ? value / 10.0 : value * 10.0;
    if (end != NULL) *end = (char*)text;
    return negative ? -value : value;
}

float strtof(const char* text, char** end) {
    return (float)strtod(text, end);
}

static unsigned short ctype_table[384];
static const unsigned short* ctype_table_pointer = &ctype_table[128];
static int ctype_initialized;

const unsigned short int** __ctype_b_loc(void) {
    if (!ctype_initialized) {
        for (int value = 0; value < 256; value++) {
            unsigned short flags = 0;
            if (value < 32 || value == 127) flags |= 0x0002;
            if (value == ' ' || (value >= '\t' && value <= '\r')) flags |= 0x2001;
            if (value >= '0' && value <= '9') flags |= 0x0808;
            if ((value >= 'a' && value <= 'z') ||
                (value >= 'A' && value <= 'Z')) flags |= 0x0408;
            if (value >= 'a' && value <= 'z') flags |= 0x0200;
            if (value >= 'A' && value <= 'Z') flags |= 0x0100;
            if ((value >= '0' && value <= '9') ||
                (value >= 'a' && value <= 'f') ||
                (value >= 'A' && value <= 'F')) flags |= 0x1000;
            if (value >= 32 && value < 127) flags |= 0x4000;
            if (value > 32 && value < 127) flags |= 0x8000;
            if (value > 32 && value < 127 &&
                !((value >= '0' && value <= '9') ||
                  (value >= 'a' && value <= 'z') ||
                  (value >= 'A' && value <= 'Z'))) flags |= 0x0004;
            ctype_table[value + 128] = flags;
        }
        ctype_initialized = 1;
    }
    return &ctype_table_pointer;
}

void* mremap(void* old_address, size_t old_length, size_t new_length,
             int flags, ...) {
    if (old_address == NULL || old_length == 0 || new_length == 0 ||
        (flags & ~MREMAP_MAYMOVE) != 0) {
        errno = EINVAL;
        return MAP_FAILED;
    }
    if (old_length == new_length) return old_address;
    void* replacement = mmap(NULL, new_length, PROT_READ | PROT_WRITE,
                             MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (replacement == MAP_FAILED) return MAP_FAILED;
    const size_t copy_length = old_length < new_length ? old_length : new_length;
    memcpy(replacement, old_address, copy_length);
    if (munmap(old_address, old_length) != 0) {
        munmap(replacement, new_length);
        return MAP_FAILED;
    }
    return replacement;
}

double strtod_l(const char* text, char** end, void* locale) {
    (void)locale;
    return strtod(text, end);
}

float strtof_l(const char* text, char** end, void* locale) {
    (void)locale;
    return strtof(text, end);
}

long double strtold_l(const char* text, char** end, void* locale) {
    (void)locale;
    return (long double)strtod(text, end);
}

size_t __mbrlen(const char* text, size_t length, void* state) {
    (void)state;
    if (text == NULL) return 0;
    if (length == 0) return (size_t)-2;
    const unsigned char lead = (unsigned char)text[0];
    if (lead < 0x80) return 1;
    const size_t expected = (lead & 0xe0) == 0xc0 ? 2 :
                            ((lead & 0xf0) == 0xe0 ? 3 :
                             ((lead & 0xf8) == 0xf0 ? 4 : 0));
    if (expected == 0) return (size_t)-1;
    if (length < expected) return (size_t)-2;
    for (size_t index = 1; index < expected; index++) {
        if (((unsigned char)text[index] & 0xc0) != 0x80) return (size_t)-1;
    }
    return expected;
}

void abort(void) {
    __builtin_trap();
}

void __stack_chk_fail(void) {
    __builtin_trap();
}

uintptr_t __libc_stack_end = 0;

typedef int _Unwind_Reason_Code;
typedef uintptr_t _Unwind_Word;
typedef struct _Unwind_Context _Unwind_Context;
typedef _Unwind_Reason_Code (*_Unwind_Trace_Fn)(_Unwind_Context*, void*);

_Unwind_Reason_Code _Unwind_Backtrace(_Unwind_Trace_Fn trace, void* argument) {
    (void)trace;
    (void)argument;
    return 0;
}

_Unwind_Word _Unwind_GetIP(_Unwind_Context* context) {
    (void)context;
    return 0;
}

void _Unwind_SetGR(_Unwind_Context* context, int register_number,
                   _Unwind_Word value) {
    (void)context;
    (void)register_number;
    (void)value;
}

int gettimeofday(void* value, void* timezone) {
    (void)timezone;
    if (value == NULL) {
        errno = EFAULT;
        return -1;
    }
    struct kz_timeval {
        long seconds;
        long microseconds;
    };
    const int64_t milliseconds = kz_host_wall_time_ms();
    struct kz_timeval* result = (struct kz_timeval*)value;
    result->seconds = (long)(milliseconds / 1000);
    result->microseconds = (long)(milliseconds % 1000) * 1000;
    return 0;
}

time_t time(time_t* result) {
    const time_t seconds = (time_t)(kz_host_wall_time_ms() / 1000);
    if (result != NULL) *result = seconds;
    return seconds;
}

char* strerror(int error) {
    switch (error) {
        case 0: return "success";
        case EINVAL: return "invalid argument";
        case ENOMEM: return "out of memory";
        case EAGAIN: return "try again";
        case EBUSY: return "resource busy";
        case EIO: return "I/O error";
        case EBADF: return "bad file descriptor";
        case ENOSYS: return "function not implemented";
        default: return "unknown error";
    }
}

long __sysconf(int name) { return sysconf(name); }

long __isoc23_strtol(const char* text, char** end, int base) {
    return (long)parse_unsigned(text, end, base);
}

long long __isoc23_strtoll(const char* text, char** end, int base) {
    return (long long)parse_unsigned(text, end, base);
}

int setpriority(int which, int who, int priority) {
    (void)which;
    (void)who;
    (void)priority;
    return 0;
}

int open64(const char* path, int flags, ...) {
    (void)path;
    (void)flags;
    errno = ENOENT;
    return -1;
}

int __open64_2(const char* path, int flags) {
    return open64(path, flags);
}

int unlink(const char* path) {
    (void)path;
    errno = ENOENT;
    return -1;
}

FILE* tmpfile64(void) {
    errno = ENOSYS;
    return NULL;
}

FILE* fdopen(int file_descriptor, const char* mode) {
    (void)mode;
    if (file_descriptor == 0) return stdin;
    if (file_descriptor == 1) return stdout;
    if (file_descriptor == 2) return stderr;
    errno = EBADF;
    return NULL;
}

ssize_t read(int file_descriptor, void* buffer, size_t length) {
    if (buffer == NULL && length != 0) {
        errno = EFAULT;
        return -1;
    }
    if (file_descriptor == 0) return 0;
    errno = EBADF;
    return -1;
}

int close(int file_descriptor) {
    if (file_descriptor >= 0 && file_descriptor <= 2) return 0;
    errno = EBADF;
    return -1;
}

int sigemptyset(void* set) {
    if (set == NULL) {
        errno = EFAULT;
        return -1;
    }
    memset(set, 0, SIGSET_SIZE);
    return 0;
}

int sigaction(int signal_number, const void* action, void* old_action) {
    (void)action;
    (void)old_action;
    (void)signal_number;
    errno = ENOSYS;
    return -1;
}

int sigaltstack(const void* stack, void* old_stack) {
    (void)stack;
    (void)old_stack;
    errno = ENOSYS;
    return -1;
}

int feclearexcept(int exceptions) {
    (void)exceptions;
    return 0;
}

int feraiseexcept(int exceptions) {
    (void)exceptions;
    return 0;
}

struct kz_utsname {
    char sysname[65];
    char nodename[65];
    char release[65];
    char version[65];
    char machine[65];
    char domainname[65];
};

int uname(void* output) {
    if (output == NULL) {
        errno = EFAULT;
        return -1;
    }
    struct kz_utsname* name = (struct kz_utsname*)output;
    memset(name, 0, sizeof(*name));
    strncpy(name->sysname, "kz", sizeof(name->sysname) - 1);
    strncpy(name->nodename, "localhost", sizeof(name->nodename) - 1);
    strncpy(name->release, "0.1", sizeof(name->release) - 1);
    strncpy(name->version, "freestanding", sizeof(name->version) - 1);
    strncpy(name->machine, "x86_64", sizeof(name->machine) - 1);
    strncpy(name->domainname, "local", sizeof(name->domainname) - 1);
    return 0;
}