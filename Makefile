CARGO ?= cargo
CLANG ?= clang
OBJCOPY ?= llvm-objcopy
CARGO_TARGET_DIR ?= build/cargo
V8_OUT ?= runtime/v8/src/out.gn/x64.release

.PHONY: all test run-x86_64 run-aarch64 v8-host-test clean force
all: build/x86_64/kernel.elf build/aarch64/kernel.bin

force:

build/x86_64/kernel64.elf: force
	mkdir -p $(@D)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" $(CARGO) build --manifest-path kernel/Cargo.toml --release --target x86_64-unknown-none
	cp "$(CARGO_TARGET_DIR)/x86_64-unknown-none/release/kernel" $@

build/x86_64/kernel.elf: build/x86_64/kernel64.elf
	$(OBJCOPY) -O elf32-i386 $< $@

build/aarch64/kernel.elf: force
	mkdir -p $(@D)
	CARGO_TARGET_DIR="$(CARGO_TARGET_DIR)" CLANG="$(CLANG)" $(CARGO) build --manifest-path kernel/Cargo.toml --release --target aarch64-unknown-none
	cp "$(CARGO_TARGET_DIR)/aarch64-unknown-none/release/kernel" $@

build/aarch64/kernel.bin: build/aarch64/kernel.elf
	$(OBJCOPY) -O binary $< $@

test: all
	python3 tests/qemu/smoke.py

v8-host-test:
	ninja -C "$(V8_OUT)" v8_hello_world d8
	"$(V8_OUT)/v8_hello_world"
	"$(V8_OUT)/d8" --allow-natives-syntax tests/v8/jit-smoke.js

run-x86_64: build/x86_64/kernel.elf
	qemu-system-x86_64 -machine microvm -accel tcg -m 128M -smp 1 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

run-aarch64: build/aarch64/kernel.bin
	qemu-system-aarch64 -machine virt -cpu cortex-a72 -accel tcg -m 128M -smp 1 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

clean:
	rm -rf build