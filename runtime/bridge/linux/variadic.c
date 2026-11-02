#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

#include <fcntl.h>
#include <stdio.h>
#include <sys/types.h>

#if defined(__x86_64__)
#define KZ_VA_LIST_PTR(arguments) ((void*)(arguments))
#else
#define KZ_VA_LIST_PTR(arguments) ((void*)&(arguments))
#endif

enum {
    KZ_VA_INT = 1,
    KZ_VA_UINT = 2,
    KZ_VA_LONG = 3,
    KZ_VA_ULONG = 4,
    KZ_VA_LLONG = 5,
    KZ_VA_ULLONG = 6,
    KZ_VA_POINTER = 7,
    KZ_VA_SIZE = 8,
    KZ_VA_PTRDIFF = 9,
};

extern int kz_rust_printf(const char* format, void* arguments);
extern int kz_rust_vsnprintf(char* buffer, size_t size, const char* format,
                             void* arguments);
extern int kz_rust_vsnprintf_chk(char* buffer, size_t size, int flag,
                                 size_t object_size, const char* format,
                                 void* arguments);
extern int kz_rust_vfprintf(FILE* stream, const char* format, void* arguments);
extern int kz_rust_vprintf(const char* format, void* arguments);
extern int kz_rust_vasprintf(char** output, const char* format,
                             void* arguments);
extern int kz_rust_prctl(int option, void* arguments);
extern long kz_rust_syscall(long number, void* arguments);
extern int kz_rust_open(const char* path, int flags, unsigned int mode);
extern void* kz_rust_mremap(void* old_address, size_t old_length,
                            size_t new_length, int flags);
extern int kz_rust_sscanf(const char* input, const char* format,
                          void* arguments);
extern double kz_rust_strtold_l(const char* input, char** end, void* locale);

uintptr_t kz_va_arg(void* raw_arguments, int kind) {
    va_list* arguments = (va_list*)raw_arguments;
    switch (kind) {
        case KZ_VA_INT: return (uintptr_t)va_arg(*arguments, int);
        case KZ_VA_UINT: return (uintptr_t)va_arg(*arguments, unsigned int);
        case KZ_VA_LONG: return (uintptr_t)va_arg(*arguments, long);
        case KZ_VA_ULONG: return (uintptr_t)va_arg(*arguments, unsigned long);
        case KZ_VA_LLONG: return (uintptr_t)va_arg(*arguments, long long);
        case KZ_VA_ULLONG: return (uintptr_t)va_arg(*arguments, unsigned long long);
        case KZ_VA_POINTER: return (uintptr_t)va_arg(*arguments, void*);
        case KZ_VA_SIZE: return (uintptr_t)va_arg(*arguments, size_t);
        case KZ_VA_PTRDIFF: return (uintptr_t)va_arg(*arguments, ptrdiff_t);
        default: return 0;
    }
}

void kz_va_copy(void* destination, void* source) {
    va_copy(*(va_list*)destination, *(va_list*)source);
}

void kz_va_end(void* arguments) {
    va_end(*(va_list*)arguments);
}

int printf(const char* format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_printf(format, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

int snprintf(char* buffer, size_t size, const char* format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_vsnprintf(buffer, size, format,
                                         KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

int vsnprintf(char* buffer, size_t size, const char* format,
              va_list arguments) {
    return kz_rust_vsnprintf(buffer, size, format, KZ_VA_LIST_PTR(arguments));
}

int vfprintf(FILE* stream, const char* format, va_list arguments) {
    return kz_rust_vfprintf(stream, format, KZ_VA_LIST_PTR(arguments));
}

int vprintf(const char* format, va_list arguments) {
    return kz_rust_vprintf(format, KZ_VA_LIST_PTR(arguments));
}

int __vfprintf_chk(FILE* stream, int flag, const char* format,
                   va_list arguments) {
    (void)flag;
    return kz_rust_vfprintf(stream, format, KZ_VA_LIST_PTR(arguments));
}

int __vprintf_chk(int flag, const char* format, va_list arguments) {
    (void)flag;
    return kz_rust_vprintf(format, KZ_VA_LIST_PTR(arguments));
}

int __vsnprintf_chk(char* buffer, size_t size, int flag, size_t object_size,
                    const char* format, va_list arguments) {
    return kz_rust_vsnprintf_chk(buffer, size, flag, object_size, format,
                                 KZ_VA_LIST_PTR(arguments));
}

int vasprintf(char** output, const char* format, va_list arguments) {
    return kz_rust_vasprintf(output, format, KZ_VA_LIST_PTR(arguments));
}

int open(const char* path, int flags, ...) {
    unsigned int mode = 0;
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode = (unsigned int)va_arg(arguments, int);
        va_end(arguments);
    }
    return kz_rust_open(path, flags, mode);
}

int open64(const char* path, int flags, ...) {
    unsigned int mode = 0;
    if ((flags & O_CREAT) != 0) {
        va_list arguments;
        va_start(arguments, flags);
        mode = (unsigned int)va_arg(arguments, int);
        va_end(arguments);
    }
    return kz_rust_open(path, flags, mode);
}

int prctl(int option, ...) {
    va_list arguments;
    va_start(arguments, option);
    const int result = kz_rust_prctl(option, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

long syscall(long number, ...) {
    va_list arguments;
    va_start(arguments, number);
    const long result = kz_rust_syscall(number, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

void* mremap(void* old_address, size_t old_length, size_t new_length,
             int flags, ...) {
    return kz_rust_mremap(old_address, old_length, new_length, flags);
}

int __isoc23_sscanf(const char* input, const char* format, ...) {
    va_list arguments;
    va_start(arguments, format);
    const int result = kz_rust_sscanf(input, format, KZ_VA_LIST_PTR(arguments));
    va_end(arguments);
    return result;
}

long double strtold_l(const char* input, char** end, void* locale) {
    return (long double)kz_rust_strtold_l(input, end, locale);
}