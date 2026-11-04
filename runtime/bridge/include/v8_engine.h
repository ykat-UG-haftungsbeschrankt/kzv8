#ifndef KZ_RUNTIME_BRIDGE_INCLUDE_V8_ENGINE_H_
#define KZ_RUNTIME_BRIDGE_INCLUDE_V8_ENGINE_H_

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int kz_v8_run_smoke(void* event_loop, void* block_device, void* tcp_context,
					const uint8_t* bootstrap_source, size_t bootstrap_length);

#ifdef __cplusplus
}
#endif

#endif