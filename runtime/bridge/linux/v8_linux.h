#ifndef KZ_RUNTIME_BRIDGE_LINUX_V8_LINUX_H_
#define KZ_RUNTIME_BRIDGE_LINUX_V8_LINUX_H_

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

enum kz_linux_v8_status {
  KZ_LINUX_V8_OK = 0,
  KZ_LINUX_V8_INVALID_ARGUMENT = -1,
  KZ_LINUX_V8_INITIALIZATION_FAILED = -2,
  KZ_LINUX_V8_ISOLATE_FAILED = -3,
  KZ_LINUX_V8_SCRIPT_FAILED = -4,
  KZ_LINUX_V8_RESULT_TOO_LARGE = -5,
};

int kz_linux_v8_eval(const char* source, size_t source_length, char* result,
                     size_t result_capacity, size_t* result_length);

#ifdef __cplusplus
}
#endif

#endif