#ifndef KZ_RUNTIME_BRIDGE_INCLUDE_V8_ENGINE_H_
#define KZ_RUNTIME_BRIDGE_INCLUDE_V8_ENGINE_H_

#ifdef __cplusplus
extern "C" {
#endif

int kz_v8_run_smoke(void* event_loop, void* block_device, void* tcp_context);

#ifdef __cplusplus
}
#endif

#endif