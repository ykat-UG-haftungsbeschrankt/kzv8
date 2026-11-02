#include "v8_engine.h"

#include <cstdint>
#include <memory>

#include "os_platform.h"
#include "v8.h"

extern "C" int kz_host_console_write(const char* data, size_t length);

namespace {

void report(const char* message, size_t length) {
#if defined(KZ_FREESTANDING) && defined(__x86_64__)
	for (size_t index = 0; index < length; ++index) {
		__asm__ volatile("outb %0, $0xe9"
									 :
									 : "a"(static_cast<unsigned char>(message[index])));
	}
#endif
	kz_host_console_write(message, length);
}

void report_result(std::int32_t value) {
	char message[32];
	static constexpr char prefix[] = "kz: JS result: ";
	size_t length = sizeof(prefix) - 1;
	for (size_t index = 0; index < length; ++index) {
		message[index] = prefix[index];
	}
	uint32_t magnitude;
	if (value < 0) {
		message[length++] = '-';
		magnitude = static_cast<uint32_t>(-static_cast<int64_t>(value));
	} else {
		magnitude = static_cast<uint32_t>(value);
	}
	char digits[10];
	size_t digit_count = 0;
	do {
		digits[digit_count++] = static_cast<char>('0' + magnitude % 10);
		magnitude /= 10;
	} while (magnitude != 0);
	while (digit_count != 0) {
		message[length++] = digits[--digit_count];
	}
	message[length++] = '\n';
	report(message, length);
}

}  // namespace

extern "C" int kz_v8_run_smoke() {
	report("kz: V8 smoke entered\n", sizeof("kz: V8 smoke entered\n") - 1);
#if !defined(KZ_FREESTANDING)
	v8::V8::SetFlagsFromString("--single-threaded");
#endif
	report("kz: V8 flags set\n", sizeof("kz: V8 flags set\n") - 1);
	v8::Platform* platform = kz_os_platform_create_for_kernel();
	if (platform == nullptr) return -1;
	report("kz: V8 platform created\n", sizeof("kz: V8 platform created\n") - 1);

	v8::V8::InitializePlatform(platform);
	report("kz: V8 platform initialized\n", sizeof("kz: V8 platform initialized\n") - 1);
	if (!v8::V8::Initialize()) {
		v8::V8::DisposePlatform();
		kz_os_platform_destroy(platform);
		return -2;
	}
	report("kz: V8 initialized\n", sizeof("kz: V8 initialized\n") - 1);

	v8::ArrayBuffer::Allocator* allocator =
			v8::ArrayBuffer::Allocator::NewDefaultAllocator();
	if (allocator == nullptr) {
		v8::V8::Dispose();
		v8::V8::DisposePlatform();
		kz_os_platform_destroy(platform);
		return -3;
	}
	report("kz: V8 allocator ready\n", sizeof("kz: V8 allocator ready\n") - 1);

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
	report("kz: V8 isolate created\n", sizeof("kz: V8 isolate created\n") - 1);

	bool succeeded = false;
	std::int32_t script_result = -1;
	{
		v8::Isolate::Scope isolate_scope(isolate);
		v8::HandleScope handle_scope(isolate);
		v8::Local<v8::Context> context = v8::Context::New(isolate);
		if (!context.IsEmpty()) {
			report("kz: V8 context created\n", sizeof("kz: V8 context created\n") - 1);
			v8::Context::Scope context_scope(context);
			v8::Local<v8::String> source =
					v8::String::NewFromUtf8Literal(isolate, "6 * 7");
			v8::Local<v8::Script> script;
			if (v8::Script::Compile(context, source).ToLocal(&script)) {
				report("kz: V8 script compiled\n", sizeof("kz: V8 script compiled\n") - 1);
				v8::Local<v8::Value> result;
				if (script->Run(context).ToLocal(&result)) {
					report("kz: V8 script ran\n", sizeof("kz: V8 script ran\n") - 1);
					script_result = result->Int32Value(context).FromMaybe(-1);
					succeeded = script_result == 42;
					report_result(script_result);
				}
			}
		}
	}

	for (size_t count = 0; count < 128 && kz_os_platform_pump_one(platform); count++) {
	}
	report("kz: V8 disposing isolate\n", sizeof("kz: V8 disposing isolate\n") - 1);
	isolate->Dispose();
	report("kz: V8 isolate disposed\n", sizeof("kz: V8 isolate disposed\n") - 1);
	delete allocator;
	report("kz: V8 disposing runtime\n", sizeof("kz: V8 disposing runtime\n") - 1);
	v8::V8::Dispose();
	report("kz: V8 runtime disposed\n", sizeof("kz: V8 runtime disposed\n") - 1);
	v8::V8::DisposePlatform();
	report("kz: V8 platform disposed\n", sizeof("kz: V8 platform disposed\n") - 1);
	kz_os_platform_destroy(platform);

	if (!succeeded) return -5;
	static constexpr char message[] = "kz: V8 evaluated 6 * 7 = 42\n";
	report(message, sizeof(message) - 1);
	return 0;
}
