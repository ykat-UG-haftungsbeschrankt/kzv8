#ifndef KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_STDIO_H_
#define KZ_RUNTIME_BRIDGE_LINUX_INCLUDE_STDIO_H_

#ifdef __cplusplus
extern "C" {
#endif

#define EOF (-1)

int putchar(int character);
int puts(const char* text);
int printf(const char* format, ...);

#ifdef __cplusplus
}
#endif

#endif