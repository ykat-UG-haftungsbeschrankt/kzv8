#ifndef KZ_RUNTIME_BRIDGE_INCLUDE_OS_PLATFORM_H_
#define KZ_RUNTIME_BRIDGE_INCLUDE_OS_PLATFORM_H_

#include <stdint.h>

#include "v8-platform.h"

extern "C" {

v8::Platform* kz_os_platform_create(v8::PageAllocator* page_allocator);
v8::Platform* kz_os_platform_create_for_kernel();
void kz_os_platform_destroy(v8::Platform* platform);
int kz_os_platform_pump_one(v8::Platform* platform);
uint64_t kz_os_platform_next_deadline_ns(v8::Platform* platform);

}

#endif