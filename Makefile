CARGO ?= cargo
CLANG ?= clang
OBJCOPY ?= llvm-objcopy
CARGO_TARGET_DIR ?= build/cargo
V8_OUT ?= runtime/v8/src/out.gn/x64.release
V8_PLATFORM_OUT ?= runtime/v8/src/out.gn/x64.platform
V8_CXX ?= runtime/v8/src/third_party/llvm-build/Release+Asserts/bin/clang++
KERNEL_X86_RUSTFLAGS ?= -C relocation-model=static -C link-arg=-no-pie -C link-arg=--error-limit=0
V8_RUST_LIBS ?= $(shell find "$(V8_PLATFORM_OUT)/obj/third_party/rust" -type f -name '*.rlib' -print) $(wildcard $(V8_PLATFORM_OUT)/local_rustc_sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib/*.rlib)
V8_RUST_ALLOCATOR_LIBS ?= $(wildcard $(V8_PLATFORM_OUT)/obj/build/rust/allocator/*.rlib)

.PHONY: all test run-x86_64 run-aarch64 v8-host-test v8-platform-test clean force
all: build/x86_64/kernel.elf build/aarch64/kernel.bin

force:

build/x86_64/kernel64.elf: force
	mkdir -p $(@D)
	cd runtime/v8/src && buildtools/linux64/gn gen out.gn/x64.platform --args="$$(cat ../args-platform.gn)"
	ninja -C "$(V8_PLATFORM_OUT)" v8_monolith
	RUSTFLAGS="$(KERNEL_X86_RUSTFLAGS) $${RUSTFLAGS}" CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" OBJCOPY="$(OBJCOPY)" V8_CXX="$(V8_CXX)" V8_PLATFORM_OUT="$(V8_PLATFORM_OUT)" $(CARGO) build --manifest-path Cargo.toml --release --target x86_64-unknown-none
	cp "$(CARGO_TARGET_DIR)/x86_64-unknown-none/release/kernel" $@

build/x86_64/kernel.elf: build/x86_64/kernel64.elf
	$(OBJCOPY) -O elf32-i386 $< $@

build/aarch64/kernel.elf: force
	mkdir -p $(@D)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" OBJCOPY="$(OBJCOPY)" $(CARGO) build --manifest-path Cargo.toml --release --target aarch64-unknown-none
	cp "$(CARGO_TARGET_DIR)/aarch64-unknown-none/release/kernel" $@

build/aarch64/kernel.bin: build/aarch64/kernel.elf
	$(OBJCOPY) -O binary $< $@

test: all
	python3 tests/qemu/smoke.py

v8-host-test:
	ninja -C "$(V8_OUT)" v8_hello_world d8
	"$(V8_OUT)/v8_hello_world"
	"$(V8_OUT)/d8" --allow-natives-syntax tests/v8/jit-smoke.js
	$(MAKE) v8-platform-test

v8-platform-test:
	mkdir -p build/v8
	cd runtime/v8/src && buildtools/linux64/gn gen out.gn/x64.platform --args="$$(cat ../args-platform.gn)"
	ninja -C "$(V8_PLATFORM_OUT)" v8_monolith
	"$(V8_CXX)" -std=c++20 -nostdlib++ -fPIC -pthread -fuse-ld=lld -Wall -Wextra -Wno-unused-parameter \
		-I runtime/bridge/include \
		-I runtime/v8/src/include \
		-I "$(V8_PLATFORM_OUT)/gen/include" \
		-I "$(V8_PLATFORM_OUT)/gen" \
		-I runtime/v8/src/buildtools/third_party/libc++ \
		-I runtime/v8/src/third_party/abseil-cpp \
		-DV8_HAVE_TARGET_OS -DV8_TARGET_OS_LINUX -DV8_TARGET_ARCH_X64 \
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

clean:
	#rm -rf build