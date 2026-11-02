#include "v8_engine.h"

#include <cstdint>
#include <memory>

#include "os_platform.h"
#include "v8.h"

extern "C" int kz_host_console_write(const char* data, size_t length);

struct KzRawEvent {
	uint32_t kind;
	uint32_t flags;
	uint64_t id;
	int32_t status;
	uint32_t reserved;
	uint64_t data;
};

extern "C" {
int kz_host_event_loop_poll(void* context, KzRawEvent* output);
int kz_host_event_loop_schedule_timer(void* context, uint64_t delay_ns,
											  uint64_t* timer_id);
int kz_host_event_loop_cancel_timer(void* context, uint64_t timer_id);
int kz_host_event_loop_register(void* context, uint64_t source_id,
									uint32_t interest, uint64_t data, uint64_t* token);
int kz_host_event_loop_modify(void* context, uint64_t token,
								  uint32_t interest, uint64_t data);
int kz_host_event_loop_set_readiness(void* context, uint64_t token,
									 uint32_t readiness);
int kz_host_event_loop_deregister(void* context, uint64_t token);
int kz_host_event_loop_complete(void* context, uint64_t id, int32_t status);
int kz_host_event_loop_next_deadline(void* context, uint64_t* deadline_ns);
}

namespace {

void throw_type_error(v8::Isolate* isolate, const char* message) {
	isolate->ThrowException(v8::Exception::TypeError(
			v8::String::NewFromUtf8(isolate, message).ToLocalChecked()));
}

void throw_range_error(v8::Isolate* isolate, const char* message) {
	isolate->ThrowException(v8::Exception::RangeError(
			v8::String::NewFromUtf8(isolate, message).ToLocalChecked()));
}

void* event_loop_context(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto data = args.DataV2();
	if (data.IsEmpty()) return nullptr;
	return data.As<v8::External>()->Value(v8::kExternalPointerTypeTagDefault);
}

bool value_to_u64(v8::Local<v8::Value> value, uint64_t* output) {
	if (value->IsBigInt()) {
		bool lossless = false;
		*output = value.As<v8::BigInt>()->Uint64Value(&lossless);
		return lossless;
	}
	if (!value->IsNumber()) return false;
	const double number = value.As<v8::Number>()->Value();
	constexpr double kMaxSafeInteger = 9007199254740991.0;
	if (number < 0 || number > kMaxSafeInteger) return false;
	*output = static_cast<uint64_t>(number);
	return true;
}

bool value_to_flags(v8::Local<v8::Value> value, uint32_t* output) {
	if (!value->IsUint32()) return false;
	*output = value->Uint32Value(v8::Isolate::GetCurrent()->GetCurrentContext())
								  .FromMaybe(0);
	return true;
}

void event_loop_poll(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	if (context == nullptr) {
		throw_type_error(isolate, "event loop is unavailable");
		return;
	}
	KzRawEvent event{};
	if (!kz_host_event_loop_poll(context, &event)) {
		args.GetReturnValue().Set(v8::Null(isolate));
		return;
	}
	const char* kind = nullptr;
	switch (event.kind) {
		case 1: kind = "timer"; break;
		case 2: kind = "completion"; break;
		case 3: kind = "readiness"; break;
		case 4: kind = "deviceInterrupt"; break;
		default:
			throw_type_error(isolate, "unknown event kind");
			return;
	}
	auto context_scope = isolate->GetCurrentContext();
	auto object = v8::Object::New(isolate);
	object->Set(context_scope, v8::String::NewFromUtf8Literal(isolate, "kind"),
				v8::String::NewFromUtf8(isolate, kind).ToLocalChecked())
				.Check();
	object->Set(context_scope, v8::String::NewFromUtf8Literal(isolate, "flags"),
				v8::Integer::NewFromUnsigned(isolate, event.flags))
				.Check();
	object->Set(context_scope, v8::String::NewFromUtf8Literal(isolate, "id"),
				v8::BigInt::NewFromUnsigned(isolate, event.id))
				.Check();
	object->Set(context_scope, v8::String::NewFromUtf8Literal(isolate, "status"),
				v8::Integer::New(isolate, event.status))
				.Check();
	object->Set(context_scope, v8::String::NewFromUtf8Literal(isolate, "data"),
				v8::BigInt::NewFromUnsigned(isolate, event.data))
				.Check();
	args.GetReturnValue().Set(object);
}

void event_loop_schedule_timer(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	if (context == nullptr || args.Length() < 1 || !args[0]->IsNumber()) {
		throw_type_error(isolate, "scheduleTimer expects a delay in nanoseconds");
		return;
	}
	const double delay = args[0].As<v8::Number>()->Value();
	constexpr double kMaxSafeInteger = 9007199254740991.0;
	if (delay < 0 || delay > kMaxSafeInteger) {
		throw_range_error(isolate, "timer delay is out of range");
		return;
	}
	uint64_t timer_id = 0;
	if (!kz_host_event_loop_schedule_timer(context, static_cast<uint64_t>(delay),
											   &timer_id)) {
		throw_range_error(isolate, "event loop timer capacity exhausted");
		return;
	}
	args.GetReturnValue().Set(v8::BigInt::NewFromUnsigned(isolate, timer_id));
}

void event_loop_cancel_timer(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t timer_id = 0;
	if (context == nullptr || args.Length() < 1 ||
			!value_to_u64(args[0], &timer_id)) {
		throw_type_error(isolate, "cancelTimer expects a timer id");
		return;
	}
	args.GetReturnValue().Set(kz_host_event_loop_cancel_timer(context, timer_id) != 0);
}

void event_loop_register(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t source_id = 0;
	uint64_t data = 0;
	uint32_t interest = 0;
	if (context == nullptr || args.Length() < 3 ||
			!value_to_u64(args[0], &source_id) ||
			!value_to_flags(args[1], &interest) ||
			!value_to_u64(args[2], &data)) {
		throw_type_error(isolate, "register expects source id, flags, and data");
		return;
	}
	uint64_t token = 0;
	if (!kz_host_event_loop_register(context, source_id, interest, data, &token)) {
		throw_range_error(isolate, "event source registration failed");
		return;
	}
	args.GetReturnValue().Set(v8::BigInt::NewFromUnsigned(isolate, token));
}

void event_loop_modify(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t token = 0;
	uint64_t data = 0;
	uint32_t interest = 0;
	if (context == nullptr || args.Length() < 3 ||
			!value_to_u64(args[0], &token) ||
			!value_to_flags(args[1], &interest) ||
			!value_to_u64(args[2], &data)) {
		throw_type_error(isolate, "modify expects token, flags, and data");
		return;
	}
	args.GetReturnValue().Set(
			kz_host_event_loop_modify(context, token, interest, data) != 0);
}

void event_loop_set_readiness(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t token = 0;
	uint32_t readiness = 0;
	if (context == nullptr || args.Length() < 2 ||
			!value_to_u64(args[0], &token) ||
			!value_to_flags(args[1], &readiness)) {
		throw_type_error(isolate, "setReadiness expects token and flags");
		return;
	}
	args.GetReturnValue().Set(
			kz_host_event_loop_set_readiness(context, token, readiness) != 0);
}

void event_loop_deregister(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t token = 0;
	if (context == nullptr || args.Length() < 1 || !value_to_u64(args[0], &token)) {
		throw_type_error(isolate, "deregister expects a source token");
		return;
	}
	args.GetReturnValue().Set(kz_host_event_loop_deregister(context, token) != 0);
}

void event_loop_complete(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	uint64_t id = 0;
	if (context == nullptr || args.Length() < 2 || !value_to_u64(args[0], &id) ||
			!args[1]->IsInt32()) {
		throw_type_error(isolate, "complete expects id and status");
		return;
	}
	const int32_t status = args[1]
							  ->Int32Value(isolate->GetCurrentContext()).FromMaybe(0);
	args.GetReturnValue().Set(kz_host_event_loop_complete(context, id, status) != 0);
}

void event_loop_next_deadline(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	void* context = event_loop_context(args);
	if (context == nullptr) {
		throw_type_error(isolate, "event loop is unavailable");
		return;
	}
	uint64_t deadline_ns = 0;
	if (!kz_host_event_loop_next_deadline(context, &deadline_ns)) {
		args.GetReturnValue().Set(v8::Null(isolate));
		return;
	}
	args.GetReturnValue().Set(v8::BigInt::NewFromUnsigned(isolate, deadline_ns));
}

void set_event_loop_function(v8::Isolate* isolate,
							 v8::Local<v8::ObjectTemplate> object,
							 v8::Local<v8::External> data,
							 const char* name,
							 v8::FunctionCallback callback) {
	auto key = v8::String::NewFromUtf8(isolate, name).ToLocalChecked();
	object->Set(key, v8::FunctionTemplate::New(isolate, callback, data));
}

v8::Local<v8::ObjectTemplate> create_global_template(v8::Isolate* isolate,
													 void* event_loop) {
	auto data = v8::External::New(isolate, event_loop,
								  v8::kExternalPointerTypeTagDefault);
	auto events = v8::ObjectTemplate::New(isolate);
	set_event_loop_function(isolate, events, data, "poll", event_loop_poll);
	set_event_loop_function(isolate, events, data, "scheduleTimer",
							event_loop_schedule_timer);
	set_event_loop_function(isolate, events, data, "cancelTimer",
							event_loop_cancel_timer);
	set_event_loop_function(isolate, events, data, "register", event_loop_register);
	set_event_loop_function(isolate, events, data, "modify", event_loop_modify);
	set_event_loop_function(isolate, events, data, "setReadiness",
							event_loop_set_readiness);
	set_event_loop_function(isolate, events, data, "deregister",
							event_loop_deregister);
	set_event_loop_function(isolate, events, data, "complete", event_loop_complete);
	set_event_loop_function(isolate, events, data, "nextDeadline",
							event_loop_next_deadline);
	auto kz = v8::ObjectTemplate::New(isolate);
	kz->Set(v8::String::NewFromUtf8Literal(isolate, "events"), events);
	auto global = v8::ObjectTemplate::New(isolate);
	global->Set(v8::String::NewFromUtf8Literal(isolate, "kz"), kz);
	return global;
}

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

extern "C" int kz_v8_run_smoke(void* event_loop) {
	if (event_loop == nullptr) return -6;
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
		v8::Local<v8::Context> context =
				v8::Context::New(isolate, nullptr,
								 create_global_template(isolate, event_loop));
		if (!context.IsEmpty()) {
			report("kz: V8 context created\n", sizeof("kz: V8 context created\n") - 1);
			v8::Context::Scope context_scope(context);
			v8::Local<v8::String> source = v8::String::NewFromUtf8Literal(
					isolate,
					"(function(){const token=kz.events.register(23n,1,7n);"
					"if(!kz.events.modify(token,1,8n))return -1;"
					"if(!kz.events.setReadiness(token,1))return -2;"
					"const timer=kz.events.scheduleTimer(0);"
					"if(!kz.events.complete(9n,0))return -3;"
					"let ready=false,timed=false,completed=false;"
					"for(let i=0;i<12&&!(ready&&timed&&completed);i++){"
					"const event=kz.events.poll();if(!event)continue;"
					"if(event.kind==='readiness'&&event.id===23n&&event.flags===1&&event.data===8n)ready=true;"
					"if(event.kind==='timer'&&event.id===timer)timed=true;"
					"if(event.kind==='completion'&&event.id===9n&&event.status===0)completed=true;}"
					"const canceled=kz.events.scheduleTimer(1000000000);"
					"const cancelOk=kz.events.cancelTimer(canceled);"
					"const noDeadline=kz.events.nextDeadline()===null;"
					"const removed=kz.events.deregister(token);"
					"return ready&&timed&&completed&&cancelOk&&noDeadline&&removed?6*7:-4;})()");
			v8::Local<v8::Script> script;
			if (v8::Script::Compile(context, source).ToLocal(&script)) {
				report("kz: V8 script compiled\n", sizeof("kz: V8 script compiled\n") - 1);
				v8::Local<v8::Value> result;
				if (script->Run(context).ToLocal(&result)) {
					report("kz: V8 script ran\n", sizeof("kz: V8 script ran\n") - 1);
					script_result = result->Int32Value(context).FromMaybe(-1);
					succeeded = script_result == 42;
					if (succeeded) {
						report("kz: V8 event loop smoke OK\n",
								 sizeof("kz: V8 event loop smoke OK\n") - 1);
					}
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
