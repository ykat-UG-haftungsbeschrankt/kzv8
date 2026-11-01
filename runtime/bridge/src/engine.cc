#include "v8_engine.h"

#include <memory>

#include "os_platform.h"
#include "v8.h"

extern "C" int kz_host_console_write(const char* data, size_t length);

namespace {

void report(const char* message, size_t length) {
	kz_host_console_write(message, length);
}

}  // namespace

extern "C" int kz_v8_run_smoke() {
	v8::V8::SetFlagsFromString("--single-threaded");
	v8::Platform* platform = kz_os_platform_create_for_kernel();
	if (platform == nullptr) return -1;

	v8::V8::InitializePlatform(platform);
	if (!v8::V8::Initialize()) {
		v8::V8::DisposePlatform();
		kz_os_platform_destroy(platform);
		return -2;
	}

	v8::ArrayBuffer::Allocator* allocator =
			v8::ArrayBuffer::Allocator::NewDefaultAllocator();
	if (allocator == nullptr) {
		v8::V8::Dispose();
		v8::V8::DisposePlatform();
		kz_os_platform_destroy(platform);
		return -3;
	}

	v8::Isolate::CreateParams params;
	params.array_buffer_allocator = allocator;
	v8::Isolate* isolate = v8::Isolate::New(params);
	if (isolate == nullptr) {
		delete allocator;
		v8::V8::Dispose();
		v8::V8::DisposePlatform();
		kz_os_platform_destroy(platform);
		return -4;
	}

	bool succeeded = false;
	{
		v8::Isolate::Scope isolate_scope(isolate);
		v8::HandleScope handle_scope(isolate);
		v8::Local<v8::Context> context = v8::Context::New(isolate);
		if (!context.IsEmpty()) {
			v8::Context::Scope context_scope(context);
			v8::Local<v8::String> source =
					v8::String::NewFromUtf8Literal(isolate, "6 * 7");
			v8::Local<v8::Script> script;
			if (v8::Script::Compile(context, source).ToLocal(&script)) {
				v8::Local<v8::Value> result;
				if (script->Run(context).ToLocal(&result)) {
					succeeded = result->Int32Value(context).FromMaybe(-1) == 42;
				}
			}
		}
	}

	for (size_t count = 0; count < 128 && kz_os_platform_pump_one(platform); count++) {
	}
	isolate->Dispose();
	delete allocator;
	v8::V8::Dispose();
	v8::V8::DisposePlatform();
	kz_os_platform_destroy(platform);

	if (!succeeded) return -5;
	static constexpr char message[] = "kz: V8 evaluated 6 * 7 = 42\n";
	report(message, sizeof(message) - 1);
	return 0;
}
