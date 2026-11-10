# Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
#
# Permission for non-commercial use is hereby granted,
# free of charge, without warranty of any kind.

CARGO ?= cargo
CLANG ?= clang
OBJCOPY ?= llvm-objcopy
CARGO_TARGET_DIR ?= build/cargo
FPU_MODE ?= sse
AARCH64_FPU_STATE ?= on
V8_OUT ?= runtime/v8/src/out.gn/x64.release
V8_PLATFORM_OUT ?= runtime/v8/src/out.gn/x64.platform
V8_AARCH64_PLATFORM_OUT ?= runtime/v8/src/out.gn/arm64.platform
V8_CXX ?= runtime/v8/src/third_party/llvm-build/Release+Asserts/bin/clang++
QEMU_GDB_PORT ?= 1234
KERNEL_X86_RUSTFLAGS ?= -C relocation-model=static -C link-arg=-no-pie -C link-arg=--error-limit=0
V8_RUST_LIBS ?= $(shell find "$(V8_PLATFORM_OUT)/obj/third_party/rust" -type f -name '*.rlib' -print) $(wildcard $(V8_PLATFORM_OUT)/local_rustc_sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib/*.rlib)
V8_RUST_ALLOCATOR_LIBS ?= $(wildcard $(V8_PLATFORM_OUT)/obj/build/rust/allocator/*.rlib)
RUNTIME_V8 ?= 1

ifeq ($(RUNTIME_V8),1)
RUNTIME_V8_BUILD_DEPENDENCY := app-build
RUNTIME_V8_FEATURES := --features runtime-v8
else ifeq ($(RUNTIME_V8),0)
RUNTIME_V8_BUILD_DEPENDENCY :=
RUNTIME_V8_FEATURES :=
else
$(error RUNTIME_V8 must be 0 or 1)
endif

ifeq ($(FPU_MODE),x87)
KERNEL_FPU_FEATURES := --no-default-features --features fpu-x87
else ifeq ($(FPU_MODE),sse)
KERNEL_FPU_FEATURES := --no-default-features --features fpu-sse
else ifeq ($(FPU_MODE),avx)
KERNEL_FPU_FEATURES := --no-default-features --features fpu-avx
else ifeq ($(FPU_MODE),off)
KERNEL_FPU_FEATURES := --no-default-features
KERNEL_X86_FPU_RUSTFLAGS := -C target-feature=-x87,-mmx,-sse,-sse2
else
$(error FPU_MODE must be x87, sse, avx, or off)
endif

ifeq ($(AARCH64_FPU_STATE),on)
AARCH64_FPU_FEATURES :=
AARCH64_RUST_TARGET := aarch64-unknown-none
else ifeq ($(AARCH64_FPU_STATE),off)
AARCH64_FPU_FEATURES := --features aarch64-fpu-off
AARCH64_RUST_TARGET := aarch64-unknown-none-softfloat
else
$(error AARCH64_FPU_STATE must be on or off)
endif

.PHONY: all test app-build doc run-x86_64 run-aarch64 debug-x86_64 debug-aarch64 v8-host-test v8-platform-test clean force
all: $(RUNTIME_V8_BUILD_DEPENDENCY) build/x86_64/kernel.elf build/aarch64/kernel.bin

force:

build/x86_64/kernel64.elf: $(RUNTIME_V8_BUILD_DEPENDENCY) force
	mkdir -p $(@D)

ifeq ($(RUNTIME_V8),1)
	cd runtime/v8/src && buildtools/linux64/gn gen out.gn/x64.platform --args="$$(cat ../args-platform.gn)"
	ninja -C "$(V8_PLATFORM_OUT)" v8_monolith
endif
	RUSTFLAGS="$(KERNEL_X86_RUSTFLAGS) $${RUSTFLAGS} $(KERNEL_X86_FPU_RUSTFLAGS)" CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" OBJCOPY="$(OBJCOPY)" V8_CXX="$(V8_CXX)" V8_PLATFORM_OUT="$(V8_PLATFORM_OUT)" $(CARGO) build --manifest-path Cargo.toml --release --target x86_64-unknown-none $(KERNEL_FPU_FEATURES) $(RUNTIME_V8_FEATURES)
	cp "$(CARGO_TARGET_DIR)/x86_64-unknown-none/release/kernel" $@

build/x86_64/kernel.elf: build/x86_64/kernel64.elf
	$(OBJCOPY) -O elf32-i386 $< $@

build/aarch64/kernel.elf: $(RUNTIME_V8_BUILD_DEPENDENCY) force
	mkdir -p $(@D)

ifeq ($(RUNTIME_V8),1)
	cd runtime/v8/src && buildtools/linux64/gn gen out.gn/arm64.platform --args="$$(cat ../args-platform-aarch64.gn)"
	ninja -C "$(V8_AARCH64_PLATFORM_OUT)" v8_monolith libc++
endif
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" OBJCOPY="$(OBJCOPY)" V8_CXX="$(V8_CXX)" V8_AARCH64_PLATFORM_OUT="$(V8_AARCH64_PLATFORM_OUT)" $(CARGO) build --manifest-path Cargo.toml --release --target $(AARCH64_RUST_TARGET) $(AARCH64_FPU_FEATURES) $(RUNTIME_V8_FEATURES)
	cp "$(CARGO_TARGET_DIR)/$(AARCH64_RUST_TARGET)/release/kernel" $@

build/aarch64/kernel.bin: build/aarch64/kernel.elf
	$(OBJCOPY) -O binary $< $@

app-build:
	npm --prefix app ci --no-audit --no-fund
	npm --prefix app run build

test: all
	node tests/v8/vfat-smoke.js
	python3 tests/qemu/smoke.py

doc: $(RUNTIME_V8_BUILD_DEPENDENCY)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" OBJCOPY="$(OBJCOPY)" V8_CXX="$(V8_CXX)" V8_PLATFORM_OUT="$(V8_PLATFORM_OUT)" $(CARGO) doc --manifest-path Cargo.toml $(RUNTIME_V8_FEATURES)

v8-host-test:
	ninja -C "$(V8_OUT)" v8_hello_world d8
	"$(V8_OUT)/v8_hello_world"
	"$(V8_OUT)/d8" --allow-natives-syntax tests/v8/jit-smoke.js
	$(MAKE) v8-platform-test

v8-platform-test:
	mkdir -p build/v8
	cd runtime/v8/src && buildtools/linux64/gn gen out.gn/x64.platform --args="$$(cat ../args-platform.gn)"
	ninja -C "$(V8_PLATFORM_OUT)" v8_monolith
	"$(V8_CXX)" -std=c++20 -nostdlib++ -nostdinc++ -fPIC -pthread -fuse-ld=lld -Wall -Wextra -Wno-unused-parameter \
		-I runtime/bridge/include \
		-I runtime/v8/src/include \
		-I "$(V8_PLATFORM_OUT)/gen/include" \
		-I "$(V8_PLATFORM_OUT)/gen" \
		-I runtime/v8/src/buildtools/third_party/libc++ \
		-isystem runtime/v8/src/third_party/libc++/src/include \
		-isystem runtime/v8/src/third_party/libc++abi/src/include \
		-I runtime/v8/src/third_party/abseil-cpp \
		-DV8_HAVE_TARGET_OS -DV8_TARGET_OS_LINUX -DV8_TARGET_ARCH_X64 \
		-DV8_ENABLE_SANDBOX -DV8_COMPRESS_POINTERS \
		-DV8_COMPRESS_POINTERS_IN_SHARED_CAGE \
		-D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE \
		-D_LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS \
		-D_LIBCXXABI_DISABLE_VISIBILITY_ANNOTATIONS \
		-D_LIBCPP_INSTRUMENTED_WITH_ASAN=0 \
		runtime/bridge/src/platform.cc tests/v8/os-platform-smoke.cc \
		-Wl,--start-group \
		"$(V8_PLATFORM_OUT)/obj/libv8_monolith.a" \
		"$(V8_PLATFORM_OUT)/obj/buildtools/third_party/libc++/libc++.a" \
		"$(V8_PLATFORM_OUT)/obj/buildtools/third_party/libc++abi/libc++abi.a" \
		$(V8_RUST_ALLOCATOR_LIBS) \
		$(V8_RUST_LIBS) \
		-Wl,--end-group -ldl -pthread -lrt -lm -o build/v8/os-platform-smoke
	build/v8/os-platform-smoke

run-x86_64: build/x86_64/kernel.elf
	qemu-system-x86_64 -machine microvm,acpi=off -accel tcg -m 5G -smp 2 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

run-aarch64: build/aarch64/kernel.bin
	qemu-system-aarch64 -machine virt,gic-version=2 -cpu cortex-a72 -accel tcg -m 128M -smp 2 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

debug-x86_64: build/x86_64/kernel.elf
	qemu-system-x86_64 -machine microvm,acpi=off -accel tcg -m 5G -smp 2 -display none -serial none -monitor none -no-reboot -S -gdb tcp:127.0.0.1:$(QEMU_GDB_PORT) -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

debug-aarch64: build/aarch64/kernel.bin
	qemu-system-aarch64 -machine virt,gic-version=2 -cpu cortex-a72 -accel tcg -m 128M -smp 2 -display none -serial none -monitor none -no-reboot -S -gdb tcp:127.0.0.1:$(QEMU_GDB_PORT) -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

clean:
	#rm -rf build