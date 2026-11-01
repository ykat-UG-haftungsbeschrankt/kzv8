#include "v8_linux.h"

#include <cstring>
#include <limits>
#include <memory>

#include "libplatform/libplatform.h"
#include "v8.h"

namespace {

struct LinuxV8Runtime {
  std::unique_ptr<v8::Platform> platform;
  bool platform_initialized = false;
  bool initialized = false;

  LinuxV8Runtime() : platform(v8::platform::NewDefaultPlatform()) {
    if (!platform) return;
    v8::V8::InitializePlatform(platform.get());
    platform_initialized = true;
    initialized = v8::V8::Initialize();
  }

  ~LinuxV8Runtime() {
    if (initialized) v8::V8::Dispose();
    if (platform_initialized) v8::V8::DisposePlatform();
  }
};

LinuxV8Runtime& runtime() {
  static LinuxV8Runtime instance;
  return instance;
}

int evaluate(v8::Isolate* isolate, const char* source, int source_length,
             char* result, size_t result_capacity, size_t* result_length) {
  int status = KZ_LINUX_V8_SCRIPT_FAILED;
  {
    v8::Isolate::Scope isolate_scope(isolate);
    v8::HandleScope handle_scope(isolate);
    v8::TryCatch try_catch(isolate);
    v8::Local<v8::Context> context = v8::Context::New(isolate);
    if (context.IsEmpty()) return status;
    v8::Context::Scope context_scope(context);

    v8::Local<v8::String> source_string;
    if (!v8::String::NewFromUtf8(isolate, source, v8::NewStringType::kNormal,
                                 source_length)
             .ToLocal(&source_string)) {
      return status;
    }

    v8::Local<v8::Script> script;
    if (!v8::Script::Compile(context, source_string).ToLocal(&script)) {
      return status;
    }

    v8::Local<v8::Value> value;
    if (!script->Run(context).ToLocal(&value)) return status;

    v8::String::Utf8Value utf8(isolate, value);
    const int length = utf8.length();
    const char* bytes = *utf8;
    if (length < 0 || bytes == nullptr) return status;
    if (static_cast<size_t>(length) >= result_capacity) {
      return KZ_LINUX_V8_RESULT_TOO_LARGE;
    }

    std::memcpy(result, bytes, static_cast<size_t>(length));
    result[length] = '\0';
    *result_length = static_cast<size_t>(length);
    status = KZ_LINUX_V8_OK;
  }
  return status;
}

}  // namespace

extern "C" int kz_linux_v8_eval(const char* source, size_t source_length,
                                char* result, size_t result_capacity,
                                size_t* result_length) {
  if (result_length != nullptr) *result_length = 0;
  if (result != nullptr && result_capacity != 0) result[0] = '\0';
  if (source == nullptr || result == nullptr || result_length == nullptr ||
      result_capacity == 0 ||
      source_length > static_cast<size_t>(std::numeric_limits<int>::max())) {
    return KZ_LINUX_V8_INVALID_ARGUMENT;
  }

  LinuxV8Runtime& v8_runtime = runtime();
  if (!v8_runtime.initialized) return KZ_LINUX_V8_INITIALIZATION_FAILED;

  std::unique_ptr<v8::ArrayBuffer::Allocator> allocator(
      v8::ArrayBuffer::Allocator::NewDefaultAllocator());
  if (!allocator) return KZ_LINUX_V8_ISOLATE_FAILED;

  v8::Isolate::CreateParams params;
  params.array_buffer_allocator = allocator.get();
  v8::Isolate* isolate = v8::Isolate::New(params);
  if (isolate == nullptr) return KZ_LINUX_V8_ISOLATE_FAILED;

  const int status = evaluate(isolate, source, static_cast<int>(source_length),
                              result, result_capacity, result_length);
  isolate->Dispose();
  return status;
}