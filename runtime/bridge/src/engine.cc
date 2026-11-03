#include "v8_engine.h"

#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>

#include "os_platform.h"
#include "v8.h"

extern "C" int kz_host_console_write(const char* data, size_t length);
extern "C" uint64_t kz_host_time_ns();
extern "C" void kz_host_wait_for_interrupt();
extern "C" uint64_t kz_host_block_capacity(void* device);
extern "C" int kz_host_block_read(void* device, uint64_t sector,
									  uint8_t* output, size_t length);
extern "C" int kz_host_block_write(void* device, uint64_t sector,
									   const uint8_t* input, size_t length);
extern "C" int kz_host_tcp_available(void* context);
extern "C" int kz_host_tcp_connect(void* context, const uint8_t* address,
										uint32_t remote_port);
extern "C" int kz_host_tcp_send(void* context, const uint8_t* input,
									 size_t length);
extern "C" int kz_host_tcp_receive(void* context, uint8_t* output,
										 size_t length);
extern "C" int kz_host_tcp_close(void* context);

struct KzRawEvent {
	uint32_t kind;
	uint32_t flags;
	uint64_t id;
	int32_t status;
	uint32_t reserved;
	uint64_t data;
};

struct TimerSlot {
	uint64_t id = 0;
	bool active = false;
	v8::Global<v8::Function> callback;
};

struct TimerBindings {
	void* event_loop = nullptr;
	std::array<TimerSlot, 16> slots{};
};

struct DeviceBindings {
	void* block_device = nullptr;
	void* tcp_context = nullptr;
	bool console_called = false;
};

struct ModuleCacheEntry {
	bool used = false;
	std::array<char, 256> url{};
	v8::Global<v8::Module> module;
};

struct PendingImport {
	bool active = false;
	v8::Global<v8::Promise::Resolver> resolver;
	v8::Global<v8::Module> module;
};

struct ModuleLoader {
	v8::Global<v8::Function> read_module;
	v8::Global<v8::Function> read_module_sync;
	std::array<ModuleCacheEntry, 32> modules{};
	std::array<PendingImport, 16> pending{};
};

constexpr uint32_t kModuleLoaderDataSlot = 0;

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

void throw_error(v8::Isolate* isolate, const char* message) {
	isolate->ThrowException(v8::Exception::Error(
			v8::String::NewFromUtf8(isolate, message).ToLocalChecked()));
}

void* function_data(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto data = args.DataV2();
	if (data.IsEmpty()) return nullptr;
	return data.As<v8::External>()->Value(v8::kExternalPointerTypeTagDefault);
}

void* event_loop_context(const v8::FunctionCallbackInfo<v8::Value>& args) {
	return function_data(args);
}

bool value_to_u64(v8::Local<v8::Value> value, uint64_t* output);
void report(const char* message, size_t length);

void console_log(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* devices = static_cast<DeviceBindings*>(function_data(args));
	if (devices != nullptr) devices->console_called = true;
	auto context = isolate->GetCurrentContext();
	char output[512];
	size_t length = 0;
	for (int index = 0; index < args.Length() && length < sizeof(output); ++index) {
		if (index != 0) output[length++] = ' ';
		v8::Local<v8::String> text;
		if (!args[index]->ToString(context).ToLocal(&text)) return;
		v8::String::Utf8Value utf8(isolate, text);
		if (*utf8 == nullptr) return;
		size_t copy_length = static_cast<size_t>(utf8.length());
		if (copy_length > sizeof(output) - length) {
			copy_length = sizeof(output) - length;
		}
		std::memcpy(output + length, *utf8, copy_length);
		length += copy_length;
	}
	if (length < sizeof(output)) output[length++] = '\n';
	report(output, length);
}

void timer_set_timeout(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* timers = static_cast<TimerBindings*>(function_data(args));
	if (timers == nullptr || args.Length() < 2 || !args[0]->IsFunction() ||
			!args[1]->IsNumber()) {
		throw_type_error(isolate, "setTimeout expects a callback and delay in milliseconds");
		return;
	}
	const double delay_ms = args[1].As<v8::Number>()->Value();
	constexpr double kMaxSafeInteger = 9007199254740991.0;
	if (!std::isfinite(delay_ms) || delay_ms < 0 ||
			delay_ms * 1'000'000.0 > kMaxSafeInteger) {
		throw_range_error(isolate, "timer delay is out of range");
		return;
	}
	TimerSlot* slot = nullptr;
	for (TimerSlot& candidate : timers->slots) {
		if (!candidate.active) {
			slot = &candidate;
			break;
		}
	}
	if (slot == nullptr) {
		throw_range_error(isolate, "timer callback capacity exhausted");
		return;
	}
	uint64_t timer_id = 0;
	const auto delay_ns = static_cast<uint64_t>(delay_ms * 1'000'000.0);
	if (!kz_host_event_loop_schedule_timer(timers->event_loop, delay_ns,
													 &timer_id)) {
		throw_range_error(isolate, "event loop timer capacity exhausted");
		return;
	}
	slot->id = timer_id;
	slot->active = true;
	slot->callback.Reset(isolate, args[0].As<v8::Function>());
	args.GetReturnValue().Set(v8::BigInt::NewFromUnsigned(isolate, timer_id));
}

void timer_clear_timeout(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* timers = static_cast<TimerBindings*>(function_data(args));
	uint64_t timer_id = 0;
	if (timers == nullptr || args.Length() < 1 ||
			!value_to_u64(args[0], &timer_id)) {
		throw_type_error(isolate, "clearTimeout expects a timer id");
		return;
	}
	const bool cancelled =
			kz_host_event_loop_cancel_timer(timers->event_loop, timer_id) != 0;
	if (cancelled) {
		for (TimerSlot& slot : timers->slots) {
			if (slot.active && slot.id == timer_id) {
				slot.active = false;
				slot.callback.Reset();
				break;
			}
		}
	}
	args.GetReturnValue().Set(cancelled);
}

DeviceBindings* device_bindings(const v8::FunctionCallbackInfo<v8::Value>& args) {
	return static_cast<DeviceBindings*>(function_data(args));
}

void block_capacity(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* bindings = device_bindings(args);
	const uint64_t capacity = bindings == nullptr || bindings->block_device == nullptr
												   ? 0
												   : kz_host_block_capacity(bindings->block_device);
	args.GetReturnValue().Set(v8::BigInt::NewFromUnsigned(isolate, capacity));
}

void block_read(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* bindings = device_bindings(args);
	uint64_t sector = 0;
	if (bindings == nullptr || bindings->block_device == nullptr || args.Length() < 2 ||
			!value_to_u64(args[0], &sector) || !args[1]->IsUint32()) {
		throw_type_error(isolate, "block.read expects a sector and byte length");
		return;
	}
	const uint32_t length =
			args[1]->Uint32Value(isolate->GetCurrentContext()).FromMaybe(0);
	if (length == 0 || length > 64 * 1024 || length % 512 != 0) {
		throw_range_error(isolate, "block.read length must be sector aligned and at most 64 KiB");
		return;
	}
	auto backing_store = v8::ArrayBuffer::NewBackingStore(isolate, length);
	if (!backing_store) {
		throw_error(isolate, "unable to allocate block read buffer");
		return;
	}
	auto* output = static_cast<uint8_t*>(backing_store->Data());
	const int status =
			kz_host_block_read(bindings->block_device, sector, output, length);
	if (status != static_cast<int>(length)) {
		const char* message = status == -1 ? "block read rejected"
										: status == -2 ? "block read submit failed"
										: status == -3 ? "block read transfer failed"
														 : status == -4 ? "block read timed out"
																: "block read failed";
		throw_error(isolate, message);
		return;
	}
	auto buffer = v8::ArrayBuffer::New(isolate, std::move(backing_store));
	args.GetReturnValue().Set(v8::Uint8Array::New(buffer, 0, length));
}

void block_write(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* bindings = device_bindings(args);
	uint64_t sector = 0;
	if (bindings == nullptr || bindings->block_device == nullptr || args.Length() < 2 ||
			!value_to_u64(args[0], &sector) || !args[1]->IsUint8Array()) {
		throw_type_error(isolate, "block.write expects a sector and Uint8Array");
		return;
	}
	auto input = args[1].As<v8::Uint8Array>();
	const size_t length = input->ByteLength();
	if (length == 0 || length > 64 * 1024 || length % 512 != 0) {
		throw_range_error(isolate, "block.write length must be sector aligned and at most 64 KiB");
		return;
	}
	auto backing_store = input->Buffer()->GetBackingStore();
	auto* bytes = static_cast<const uint8_t*>(backing_store->Data()) +
			input->ByteOffset();
	const int written =
			kz_host_block_write(bindings->block_device, sector, bytes, length);
	if (written < 0) {
		throw_error(isolate, "block write failed");
		return;
	}
	args.GetReturnValue().Set(v8::Integer::New(isolate, written));
}

void tcp_available(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* bindings = device_bindings(args);
	args.GetReturnValue().Set(bindings != nullptr &&
												kz_host_tcp_available(bindings->tcp_context) != 0);
}

void tcp_connect(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* bindings = device_bindings(args);
	if (bindings == nullptr || args.Length() < 2 || !args[0]->IsUint8Array() ||
			args[1]->IsUint32() == false) {
		throw_type_error(isolate, "tcp.connect expects an IPv4 Uint8Array and port");
		return;
	}
	auto address = args[0].As<v8::Uint8Array>();
	if (address->Length() != 4) {
		throw_range_error(isolate, "tcp.connect expects four IPv4 bytes");
		return;
	}
	const uint32_t port =
			args[1]->Uint32Value(isolate->GetCurrentContext()).FromMaybe(0);
	auto backing_store = address->Buffer()->GetBackingStore();
	auto* bytes = static_cast<const uint8_t*>(backing_store->Data()) +
			address->ByteOffset();
	const int status = kz_host_tcp_connect(bindings->tcp_context, bytes, port);
	if (status != 1) {
		const char* message = status == -1 ? "TCP connection arguments rejected"
										: status == -2 ? "TCP device unavailable"
										: status == -3 ? "TCP socket already open"
														 : status == -4 ? "TCP socket creation failed"
															: status == -5 ? "TCP connection closed"
																: status == -7 ? "TCP network poll failed"
																	 : "TCP connection timed out";
		throw_error(isolate, message);
		return;
	}
	args.GetReturnValue().Set(true);
}

void tcp_send(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	auto* bindings = device_bindings(args);
	if (bindings == nullptr || args.Length() < 1 || !args[0]->IsUint8Array()) {
		throw_type_error(isolate, "tcp.send expects a Uint8Array");
		return;
	}
	auto input = args[0].As<v8::Uint8Array>();
	const size_t length = input->ByteLength();
	if (length == 0 || length > 4096) {
		throw_range_error(isolate, "tcp.send length must be between 1 and 4096 bytes");
		return;
	}
	auto backing_store = input->Buffer()->GetBackingStore();
	auto* bytes = static_cast<const uint8_t*>(backing_store->Data()) +
			input->ByteOffset();
	const int written = kz_host_tcp_send(bindings->tcp_context, bytes, length);
	if (written < 0) {
		throw_error(isolate, "TCP send failed");
		return;
	}
	args.GetReturnValue().Set(v8::Integer::New(isolate, written));
}

void tcp_receive(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* isolate = args.GetIsolate();
	if (args.Length() < 1 || !args[0]->IsUint32()) {
		throw_type_error(isolate, "tcp.receive expects a byte length");
		return;
	}
	const uint32_t length =
			args[0]->Uint32Value(isolate->GetCurrentContext()).FromMaybe(0);
	if (length == 0 || length > 4096) {
		throw_range_error(isolate, "tcp.receive length must be between 1 and 4096 bytes");
		return;
	}
	auto backing_store = v8::ArrayBuffer::NewBackingStore(isolate, length);
	if (!backing_store) {
		throw_error(isolate, "unable to allocate TCP receive buffer");
		return;
	}
	auto* output = static_cast<uint8_t*>(backing_store->Data());
	auto* bindings = device_bindings(args);
	const int received = bindings == nullptr
									 ? -1
									 : kz_host_tcp_receive(bindings->tcp_context, output, length);
	if (received <= 0 || static_cast<uint32_t>(received) > length) {
		throw_error(isolate, "TCP receive failed");
		return;
	}
	auto buffer = v8::ArrayBuffer::New(isolate, std::move(backing_store));
	args.GetReturnValue().Set(
			v8::Uint8Array::New(buffer, 0, static_cast<size_t>(received)));
}

void tcp_close(const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* bindings = device_bindings(args);
	args.GetReturnValue().Set(bindings != nullptr &&
											kz_host_tcp_close(bindings->tcp_context) != 0);
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

v8::Local<v8::ObjectTemplate> create_global_template(
		v8::Isolate* isolate, void* event_loop, DeviceBindings* devices,
		TimerBindings* timers) {
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
	auto timer_data = v8::External::New(isolate, timers,
												v8::kExternalPointerTypeTagDefault);
	auto timer = v8::ObjectTemplate::New(isolate);
	set_event_loop_function(isolate, timer, timer_data, "setTimeout",
							timer_set_timeout);
	set_event_loop_function(isolate, timer, timer_data, "clearTimeout",
							timer_clear_timeout);
	kz->Set(v8::String::NewFromUtf8Literal(isolate, "timer"), timer);
	auto device_data = v8::External::New(isolate, devices,
												v8::kExternalPointerTypeTagDefault);
	auto block = v8::ObjectTemplate::New(isolate);
	block->Set(v8::String::NewFromUtf8Literal(isolate, "available"),
				v8::Boolean::New(isolate, devices->block_device != nullptr));
	set_event_loop_function(isolate, block, device_data, "capacitySectors",
							block_capacity);
	set_event_loop_function(isolate, block, device_data, "read", block_read);
	set_event_loop_function(isolate, block, device_data, "write", block_write);
	kz->Set(v8::String::NewFromUtf8Literal(isolate, "block"), block);
	auto tcp = v8::ObjectTemplate::New(isolate);
	tcp->Set(v8::String::NewFromUtf8Literal(isolate, "available"),
			 v8::Boolean::New(isolate,
							 kz_host_tcp_available(devices->tcp_context) != 0));
	set_event_loop_function(isolate, tcp, device_data, "connect", tcp_connect);
	set_event_loop_function(isolate, tcp, device_data, "send", tcp_send);
	set_event_loop_function(isolate, tcp, device_data, "receive", tcp_receive);
	set_event_loop_function(isolate, tcp, device_data, "close", tcp_close);
	kz->Set(v8::String::NewFromUtf8Literal(isolate, "tcp"), tcp);
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

bool global_boolean(v8::Isolate* isolate, v8::Local<v8::Context> context,
					const char* name) {
	auto key = v8::String::NewFromUtf8(isolate, name).ToLocalChecked();
	v8::Local<v8::Value> value;
	if (!context->Global()->Get(context, key).ToLocal(&value) ||
			!value->IsBoolean()) {
		return false;
	}
	return value.As<v8::Boolean>()->Value();
}

ModuleLoader* module_loader(v8::Isolate* isolate) {
	return static_cast<ModuleLoader*>(isolate->GetData(kModuleLoaderDataSlot));
}

ModuleCacheEntry* find_module(ModuleLoader* loader, const char* url) {
	for (ModuleCacheEntry& entry : loader->modules) {
		if (entry.used && std::strcmp(entry.url.data(), url) == 0) return &entry;
	}
	return nullptr;
}

ModuleCacheEntry* allocate_module(ModuleLoader* loader) {
	for (ModuleCacheEntry& entry : loader->modules) {
		if (!entry.used) return &entry;
	}
	return nullptr;
}

PendingImport* allocate_pending_import(
		ModuleLoader* loader, v8::Isolate* isolate,
		v8::Local<v8::Promise::Resolver> resolver) {
	for (PendingImport& pending : loader->pending) {
		if (pending.active) continue;
		pending.active = true;
		pending.resolver.Reset(isolate, resolver);
		return &pending;
	}
	return nullptr;
}

void release_pending_import(PendingImport* pending) {
	if (pending == nullptr) return;
	pending->resolver.Reset();
	pending->module.Reset();
	pending->active = false;
}

void reject_pending_import(PendingImport* pending,
							   v8::Local<v8::Context> context,
							   v8::Local<v8::Value> reason) {
	if (pending == nullptr || !pending->active) return;
	auto resolver = pending->resolver.Get(v8::Isolate::GetCurrent());
	if (!resolver.IsEmpty()) resolver->Reject(context, reason).FromMaybe(false);
	release_pending_import(pending);
}

bool compile_module_response(v8::Local<v8::Context> context,
							 v8::Local<v8::Value> response,
							 v8::Local<v8::Module>* output) {
	auto* isolate = v8::Isolate::GetCurrent();
	if (!response->IsObject()) {
		throw_type_error(isolate, "VFAT loader returned a non-object");
		return false;
	}
	v8::Local<v8::Object> object;
	if (!response->ToObject(context).ToLocal(&object)) return false;
	v8::Local<v8::Value> url_value;
	v8::Local<v8::Value> bytes_value;
	if (!object
			->Get(context, v8::String::NewFromUtf8Literal(isolate, "url"))
			.ToLocal(&url_value) ||
		!object
			->Get(context, v8::String::NewFromUtf8Literal(isolate, "bytes"))
			.ToLocal(&bytes_value) ||
		!url_value->IsString() || !bytes_value->IsUint8Array()) {
		throw_type_error(isolate, "VFAT loader must return a URL and Uint8Array");
		return false;
	}

	v8::String::Utf8Value url_utf8(isolate, url_value);
	if (*url_utf8 == nullptr || url_utf8.length() == 0 ||
			static_cast<size_t>(url_utf8.length()) >= ModuleCacheEntry{}.url.size()) {
		throw_range_error(isolate, "VFAT module URL is too long");
		return false;
	}
	std::array<char, 256> canonical_url{};
	std::memcpy(canonical_url.data(), *url_utf8,
				static_cast<size_t>(url_utf8.length()));
	if (ModuleCacheEntry* cached =
			find_module(module_loader(isolate), canonical_url.data())) {
		*output = cached->module.Get(isolate);
		return !output->IsEmpty();
	}

	auto source_bytes = bytes_value.As<v8::Uint8Array>();
	const size_t source_length = source_bytes->ByteLength();
	if (source_length > 4 * 1024 * 1024 || source_length > 0x7fffffffU) {
		throw_range_error(isolate, "VFAT module source exceeds the size limit");
		return false;
	}
	auto backing_store = source_bytes->Buffer()->GetBackingStore();
	const char* source_data =
			static_cast<const char*>(backing_store->Data()) +
			source_bytes->ByteOffset();
	v8::Local<v8::String> source_string;
	if (!v8::String::NewFromUtf8(isolate, source_data,
									 v8::NewStringType::kNormal,
									 static_cast<int>(source_length))
				.ToLocal(&source_string)) {
		return false;
	}
	v8::Local<v8::String> url_string = url_value.As<v8::String>();
	v8::ScriptOrigin origin(url_string);
	v8::ScriptCompiler::Source module_source(source_string, origin);
	v8::Local<v8::Module> module;
	if (!v8::ScriptCompiler::CompileModule(isolate, &module_source)
			.ToLocal(&module)) {
		return false;
	}
	ModuleCacheEntry* entry = allocate_module(module_loader(isolate));
	if (entry == nullptr) {
		throw_range_error(isolate, "VFAT module cache capacity exhausted");
		return false;
	}
	entry->used = true;
	std::memcpy(entry->url.data(), canonical_url.data(), canonical_url.size());
	entry->module.Reset(isolate, module);
	*output = module;
	return true;
}

v8::MaybeLocal<v8::Module> resolve_module(
		v8::Local<v8::Context> context, v8::Local<v8::String> specifier,
		v8::Local<v8::FixedArray> import_attributes,
		v8::Local<v8::Module> referrer) {
	auto* isolate = v8::Isolate::GetCurrent();
	if (!import_attributes.IsEmpty() && import_attributes->Length() != 0) {
		throw_type_error(isolate, "import attributes are not supported by VFAT");
		return {};
	}
	ModuleLoader* loader = module_loader(isolate);
	if (loader == nullptr || loader->read_module_sync.IsEmpty()) {
		throw_error(isolate, "VFAT module resolver is not initialized");
		return {};
	}
	v8::Local<v8::Value> arguments[] = {specifier, referrer->GetResourceName()};
	v8::Local<v8::Value> response;
	if (!loader->read_module_sync.Get(isolate)
			->Call(context, context->Global(), 2, arguments)
			.ToLocal(&response)) {
		return {};
	}
	v8::Local<v8::Module> module;
	if (!compile_module_response(context, response, &module)) return {};
	return module;
}

void module_evaluation_fulfilled(
		const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* pending = static_cast<PendingImport*>(function_data(args));
	if (pending == nullptr || !pending->active) return;
	auto* isolate = args.GetIsolate();
	auto context = isolate->GetCurrentContext();
	auto resolver = pending->resolver.Get(isolate);
	auto module = pending->module.Get(isolate);
	if (!resolver.IsEmpty() && !module.IsEmpty()) {
		resolver->Resolve(context, module->GetModuleNamespace()).FromMaybe(false);
	}
	release_pending_import(pending);
}

void module_evaluation_rejected(
		const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* pending = static_cast<PendingImport*>(function_data(args));
	if (pending == nullptr || !pending->active) return;
	auto context = args.GetIsolate()->GetCurrentContext();
	auto reason = args.Length() > 0
					  ? args[0]
					  : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										args.GetIsolate(), "module evaluation failed"));
	reject_pending_import(pending, context, reason);
}

bool attach_module_handlers(v8::Local<v8::Context> context,
							v8::Local<v8::Promise> promise,
							PendingImport* pending,
							v8::FunctionCallback fulfilled_callback,
							v8::FunctionCallback rejected_callback) {
				auto* isolate = v8::Isolate::GetCurrent();
	auto data = v8::External::New(
			isolate, pending, v8::kExternalPointerTypeTagDefault);
	v8::Local<v8::Function> fulfilled;
	v8::Local<v8::Function> rejected;
	if (!v8::Function::New(context, fulfilled_callback, data).ToLocal(&fulfilled) ||
			!v8::Function::New(context, rejected_callback, data).ToLocal(&rejected)) {
		return false;
	}
	v8::Local<v8::Promise> chained;
	return promise->Then(context, fulfilled, rejected).ToLocal(&chained);
}

void module_source_fulfilled(
		const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* pending = static_cast<PendingImport*>(function_data(args));
	if (pending == nullptr || !pending->active) return;
	auto* isolate = args.GetIsolate();
	auto context = isolate->GetCurrentContext();
	ModuleLoader* loader = module_loader(isolate);
	if (loader == nullptr || args.Length() == 0) {
		reject_pending_import(
				pending, context,
				v8::Exception::Error(v8::String::NewFromUtf8Literal(
						isolate, "VFAT module load returned no source")));
		return;
	}
	v8::TryCatch try_catch(isolate);
	v8::Local<v8::Module> module;
	if (!compile_module_response(context, args[0], &module)) {
		auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "unable to compile VFAT module"));
		reject_pending_import(pending, context, reason);
		return;
	}
	pending->module.Reset(isolate, module);
	if (module->GetStatus() == v8::Module::kErrored) {
		reject_pending_import(pending, context, module->GetException());
		return;
	}
	if (module->GetStatus() == v8::Module::kUninstantiated) {
		const auto instantiated = module->InstantiateModule(context, resolve_module);
		if (instantiated.IsNothing() || !instantiated.FromMaybe(false)) {
			auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "unable to instantiate VFAT module"));
			reject_pending_import(pending, context, reason);
			return;
		}
	}
	if (module->GetStatus() == v8::Module::kEvaluated) {
		auto resolver = pending->resolver.Get(isolate);
		if (!resolver.IsEmpty()) {
			resolver->Resolve(context, module->GetModuleNamespace()).FromMaybe(false);
		}
		release_pending_import(pending);
		return;
	}

	v8::Local<v8::Promise> evaluation;
	if (!module->Evaluate(context).ToLocal(&evaluation)) {
		auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "unable to evaluate VFAT module"));
		reject_pending_import(pending, context, reason);
		return;
	}
	if (!attach_module_handlers(context, evaluation, pending,
								module_evaluation_fulfilled,
								module_evaluation_rejected)) {
		auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "unable to observe module evaluation"));
		reject_pending_import(pending, context, reason);
	}
}

void module_source_rejected(
		const v8::FunctionCallbackInfo<v8::Value>& args) {
	auto* pending = static_cast<PendingImport*>(function_data(args));
	if (pending == nullptr || !pending->active) return;
	auto context = args.GetIsolate()->GetCurrentContext();
	auto reason = args.Length() > 0
					  ? args[0]
					  : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										args.GetIsolate(), "VFAT module load rejected"));
	reject_pending_import(pending, context, reason);
}

v8::MaybeLocal<v8::Promise> host_import_module_dynamically(
		v8::Local<v8::Context> context, v8::Local<v8::Data> host_defined_options,
		v8::Local<v8::Value> resource_name, v8::Local<v8::String> specifier,
		v8::Local<v8::FixedArray> import_attributes) {
	auto* isolate = v8::Isolate::GetCurrent();
	v8::Local<v8::Promise::Resolver> resolver;
	if (!v8::Promise::Resolver::New(context).ToLocal(&resolver)) return {};
	auto promise = resolver->GetPromise();
	if (!import_attributes.IsEmpty() && import_attributes->Length() != 0) {
		resolver->Reject(context, v8::Exception::TypeError(
				v8::String::NewFromUtf8Literal(isolate,
						"import attributes are not supported by VFAT")))
				.FromMaybe(false);
		return promise;
	}
	ModuleLoader* loader = module_loader(isolate);
	if (loader == nullptr || loader->read_module.IsEmpty()) {
		resolver->Reject(context, v8::Exception::Error(
				v8::String::NewFromUtf8Literal(isolate,
						"VFAT module loader is not initialized")))
				.FromMaybe(false);
		return promise;
	}
	PendingImport* pending = allocate_pending_import(loader, isolate, resolver);
	if (pending == nullptr) {
		resolver->Reject(context, v8::Exception::RangeError(
				v8::String::NewFromUtf8Literal(isolate,
						"too many pending module imports")))
				.FromMaybe(false);
		return promise;
	}
	v8::Local<v8::Value> arguments[] = {specifier, resource_name};
	v8::Local<v8::Value> load_result;
	v8::TryCatch try_catch(isolate);
	if (!loader->read_module.Get(isolate)
			->Call(context, context->Global(), 2, arguments)
			.ToLocal(&load_result) || !load_result->IsPromise()) {
		auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "VFAT loader did not return a promise"));
		reject_pending_import(pending, context, reason);
		return promise;
	}
	if (!attach_module_handlers(context, load_result.As<v8::Promise>(), pending,
								module_source_fulfilled,
								module_source_rejected)) {
		auto reason = try_catch.HasCaught()
						 ? try_catch.Exception()
						 : v8::Exception::Error(v8::String::NewFromUtf8Literal(
										 isolate, "unable to observe VFAT load promise"));
		reject_pending_import(pending, context, reason);
	}
	return promise;
}

bool run_vfat_esm_smoke(v8::Local<v8::Context> context,
						v8::Isolate* isolate, ModuleLoader* loader,
						const uint8_t* bootstrap_source,
						size_t bootstrap_length) {
	if (bootstrap_source == nullptr || bootstrap_length == 0 ||
			bootstrap_length > 0x7fffffffU) {
		return false;
	}
	v8::TryCatch try_catch(isolate);
	v8::Local<v8::String> bootstrap_text;
	if (!v8::String::NewFromUtf8(isolate,
								reinterpret_cast<const char*>(bootstrap_source),
								v8::NewStringType::kNormal,
								static_cast<int>(bootstrap_length))
				.ToLocal(&bootstrap_text)) {
		return false;
	}
	v8::Local<v8::Script> bootstrap;
	if (!v8::Script::Compile(context, bootstrap_text).ToLocal(&bootstrap)) return false;
	v8::Local<v8::Value> bootstrap_result;
	if (!bootstrap->Run(context).ToLocal(&bootstrap_result)) return false;

	auto global = context->Global();
	v8::Local<v8::Value> read_module;
	v8::Local<v8::Value> read_module_sync;
	if (!global
			->Get(context, v8::String::NewFromUtf8Literal(
						isolate, "__kzVfatReadModule"))
			.ToLocal(&read_module) ||
		!global
			->Get(context, v8::String::NewFromUtf8Literal(
						isolate, "__kzVfatReadModuleSync"))
			.ToLocal(&read_module_sync) ||
		!read_module->IsFunction() || !read_module_sync->IsFunction()) {
		return false;
	}
	loader->read_module.Reset(isolate, read_module.As<v8::Function>());
	loader->read_module_sync.Reset(isolate,
									read_module_sync.As<v8::Function>());

	static constexpr char trampoline_source[] =
			"await import('virtiovfat:///app/main.js');";
	auto source = v8::String::NewFromUtf8Literal(isolate, trampoline_source);
	auto resource_name = v8::String::NewFromUtf8Literal(isolate,
														"kz:///boot.mjs");
	v8::ScriptOrigin origin(resource_name);
	v8::ScriptCompiler::Source module_source(source, origin);
	v8::Local<v8::Module> trampoline;
	if (!v8::ScriptCompiler::CompileModule(isolate, &module_source)
			.ToLocal(&trampoline)) {
		return false;
	}
	const auto instantiated = trampoline->InstantiateModule(context, resolve_module);
	if (instantiated.IsNothing() || !instantiated.FromMaybe(false)) return false;
	v8::Local<v8::Promise> evaluation;
	if (!trampoline->Evaluate(context).ToLocal(&evaluation)) return false;
	isolate->PerformMicrotaskCheckpoint();
	return evaluation->State() == v8::Promise::kFulfilled &&
			global_boolean(isolate, context, "__kzAppLoaded");
}

void reset_module_loader(ModuleLoader* loader) {
	loader->read_module.Reset();
	loader->read_module_sync.Reset();
	for (ModuleCacheEntry& module : loader->modules) {
		module.module.Reset();
		module.used = false;
	}
	for (PendingImport& pending : loader->pending) {
		release_pending_import(&pending);
	}
}

bool timers_pending(const TimerBindings& timers) {
	for (const TimerSlot& slot : timers.slots) {
		if (slot.active) return true;
	}
	return false;
}

bool dispatch_timer_event(v8::Isolate* isolate,
							  v8::Local<v8::Context> context,
							  TimerBindings* timers,
							  const KzRawEvent& event) {
	if (event.kind != 1) return true;
	for (TimerSlot& slot : timers->slots) {
		if (!slot.active || slot.id != event.id) continue;
		auto callback = slot.callback.Get(isolate);
		slot.active = false;
		slot.callback.Reset();
		v8::TryCatch try_catch(isolate);
		v8::Local<v8::Value> result;
		const bool called = callback
								->Call(context, context->Global(), 0, nullptr)
								.ToLocal(&result);
		isolate->PerformMicrotaskCheckpoint();
		return called && !try_catch.HasCaught();
	}
	return true;
}

bool pump_timer_callbacks(v8::Isolate* isolate,
						  v8::Local<v8::Context> context,
						  TimerBindings* timers,
						  void* event_loop) {
	const uint64_t started = kz_host_time_ns();
	for (size_t iteration = 0;
		 iteration < 1'000'000 && timers_pending(*timers); ++iteration) {
		KzRawEvent event{};
		const int has_event = kz_host_event_loop_poll(event_loop, &event);
		if (has_event) {
			if (!dispatch_timer_event(isolate, context, timers, event)) return false;
		} else {
			uint64_t deadline_ns = 0;
			if (!kz_host_event_loop_next_deadline(event_loop, &deadline_ns)) {
				return false;
			}
			if (kz_host_time_ns() < deadline_ns) {
				kz_host_wait_for_interrupt();
			}
		}
		if (kz_host_time_ns() - started >= 1'000'000'000) break;
	}
	return !timers_pending(*timers);
}

}  // namespace

extern "C" int kz_v8_run_smoke(void* event_loop, void* block_device,
								  void* tcp_context) {
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
	isolate->SetMicrotasksPolicy(v8::MicrotasksPolicy::kExplicit);
	DeviceBindings devices{block_device, tcp_context};
	TimerBindings timers{};
	timers.event_loop = event_loop;

	bool succeeded = false;
	std::int32_t script_result = -1;
	{
		v8::Isolate::Scope isolate_scope(isolate);
		v8::HandleScope handle_scope(isolate);
		v8::Local<v8::Context> context =
				v8::Context::New(isolate, nullptr,
								 create_global_template(isolate, event_loop, &devices,
																 &timers));
		if (!context.IsEmpty()) {
			report("kz: V8 context created\n", sizeof("kz: V8 context created\n") - 1);
			v8::Context::Scope context_scope(context);
			auto console = v8::Object::New(isolate);
			auto console_data = v8::External::New(
					isolate, &devices, v8::kExternalPointerTypeTagDefault);
			auto console_function = v8::Function::New(
					context, console_log, console_data).ToLocalChecked();
			console
					->Set(context, v8::String::NewFromUtf8Literal(isolate, "log"),
						  console_function)
					.Check();
			context->Global()
					->Set(context, v8::String::NewFromUtf8Literal(isolate, "console"),
						  console)
					.Check();
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
					"globalThis.__kzBlockDone=false;globalThis.__kzTcpDone=false;"
					"console.log('kz: JavaScript console binding OK');"
					"if(kz.block.available){if(kz.block.capacitySectors()<10n)return -5;"
					"const seed=kz.block.read(8n,512);"
					"if(seed[0]!==90||seed[1]!==165)return -6;"
					"const pattern=new Uint8Array(512);"
					"for(let i=0;i<512;i++)pattern[i]=(i*17+3)&255;"
					"if(kz.block.write(9n,pattern)!==512)return -7;"
					"const actual=kz.block.read(9n,512);"
					"for(let i=0;i<512;i++)if(actual[i]!==pattern[i])return -8;"
					"console.log('kz: raw block binding OK');"
					"globalThis.__kzBlockDone=true;}"
					"if(kz.tcp.available){"
					"if(!kz.tcp.connect(new Uint8Array([10,0,2,2]),17777))return -9;"
					"const payload=new Uint8Array([75,90,33,10]);"
					"if(kz.tcp.send(payload)!==payload.length)return -10;"
					"const reply=kz.tcp.receive(payload.length);"
					"let tcpOk=reply.length===payload.length;"
					"for(let i=0;i<payload.length;i++)if(reply[i]!==payload[i])tcpOk=false;"
					"kz.tcp.close();if(!tcpOk)return -11;"
					"console.log('kz: TCP binding OK');"
					"globalThis.__kzTcpDone=true;}"
					"globalThis.__kzPromiseDone=false;"
					"globalThis.__kzTimerDone=false;"
					"globalThis.__kzTimerMicrotaskDone=false;"
					"Promise.resolve().then(()=>{globalThis.__kzPromiseDone=true;});"
					"const promiseDeferred=!globalThis.__kzPromiseDone;"
					"kz.timer.setTimeout(()=>{globalThis.__kzTimerDone=true;"
					"Promise.resolve().then(()=>{globalThis.__kzTimerMicrotaskDone=true;});},0);"
					"return ready&&timed&&completed&&cancelOk&&noDeadline&&removed&&"
					"promiseDeferred?6*7:-12;})()");
			v8::Local<v8::Script> script;
			if (v8::Script::Compile(context, source).ToLocal(&script)) {
				report("kz: V8 script compiled\n", sizeof("kz: V8 script compiled\n") - 1);
				v8::Local<v8::Value> result;
				if (script->Run(context).ToLocal(&result)) {
					report("kz: V8 script ran\n", sizeof("kz: V8 script ran\n") - 1);
					script_result = result->Int32Value(context).FromMaybe(-1);
					isolate->PerformMicrotaskCheckpoint();
					const bool promise_done =
							global_boolean(isolate, context, "__kzPromiseDone");
					const bool timer_done = pump_timer_callbacks(
							isolate, context, &timers, event_loop);
					const bool block_done =
							global_boolean(isolate, context, "__kzBlockDone");
					const bool tcp_done =
							global_boolean(isolate, context, "__kzTcpDone");
					const bool block_required = devices.block_device != nullptr;
					const bool tcp_required =
							kz_host_tcp_available(devices.tcp_context) != 0;
					succeeded = script_result == 42 && devices.console_called &&
							(!block_required || block_done) &&
							(!tcp_required || tcp_done) && promise_done && timer_done &&
							global_boolean(isolate, context, "__kzTimerDone") &&
							global_boolean(isolate, context,
												"__kzTimerMicrotaskDone");
					if (succeeded) {
						report("kz: V8 event loop smoke OK\n",
								 sizeof("kz: V8 event loop smoke OK\n") - 1);
						report("kz: V8 explicit microtasks OK\n",
								 sizeof("kz: V8 explicit microtasks OK\n") - 1);
					}
					report_result(script_result);
				}
			}
		}
	}

	for (TimerSlot& slot : timers.slots) {
		if (slot.active) {
			static_cast<void>(
					kz_host_event_loop_cancel_timer(event_loop, slot.id));
			slot.active = false;
			slot.callback.Reset();
		}
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
