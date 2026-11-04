#include <stdarg.h>
#include <stdio.h>
#include <syslog.h>

static int current_log_mask = 0xff;

void closelog(void) {}

void openlog(const char *identity, int options, int facility) {
    (void)identity;
    (void)options;
    (void)facility;
}

int setlogmask(int mask_priority) {
    const int previous = current_log_mask;
    if (mask_priority != 0) {
        current_log_mask = mask_priority;
    }
    return previous;
}

void syslog(int priority, const char *format, ...) {
    if ((current_log_mask & LOG_MASK(priority & 7)) == 0) {
        return;
    }
    va_list arguments;
    va_start(arguments, format);
    (void)vfprintf(stderr, format, arguments);
    va_end(arguments);
    (void)fputc('\n', stderr);
}