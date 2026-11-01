RUSTC ?= $(HOME)/.cargo/bin/rustc
CLANG ?= clang
LINKER ?= ld.lld
OBJCOPY ?= llvm-objcopy
RUSTFLAGS = --edition=2021 -C panic=abort -C opt-level=2 -C relocation-model=static --emit=obj

.PHONY: all test run-x86_64 run-aarch64 clean
all: build/x86_64/kernel.elf build/aarch64/kernel.bin

build/x86_64/kernel.o: kernel/src/main.rs
	mkdir -p $(@D)
	$(RUSTC) $(RUSTFLAGS) --target x86_64-unknown-none $< -o $@

build/aarch64/kernel.o: kernel/src/main.rs
	mkdir -p $(@D)
	$(RUSTC) $(RUSTFLAGS) --target aarch64-unknown-none $< -o $@

build/x86_64/boot.o: kernel/arch/x86_64/boot.S
	mkdir -p $(@D)
	$(CLANG) --target=x86_64-none-elf -c $< -o $@

build/aarch64/boot.o: kernel/arch/aarch64/boot.S
	mkdir -p $(@D)
	$(CLANG) --target=aarch64-none-elf -c $< -o $@

build/x86_64/kernel64.elf: build/x86_64/boot.o build/x86_64/kernel.o linker/x86_64.ld
	$(LINKER) -T linker/x86_64.ld -o $@ build/x86_64/boot.o build/x86_64/kernel.o

build/x86_64/kernel.elf: build/x86_64/kernel64.elf
	$(OBJCOPY) -O elf32-i386 $< $@

build/aarch64/kernel.elf: build/aarch64/boot.o build/aarch64/kernel.o linker/aarch64.ld
	$(LINKER) -T linker/aarch64.ld -o $@ build/aarch64/boot.o build/aarch64/kernel.o

build/aarch64/kernel.bin: build/aarch64/kernel.elf
	$(OBJCOPY) -O binary $< $@

test: all
	python3 tests/smoke.py

run-x86_64: build/x86_64/kernel.elf
	qemu-system-x86_64 -machine microvm -accel tcg -m 128M -smp 1 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

run-aarch64: build/aarch64/kernel.bin
	qemu-system-aarch64 -machine virt -cpu cortex-a72 -accel tcg -m 128M -smp 1 -display none -serial none -monitor none -no-reboot -global virtio-mmio.force-legacy=false -kernel $< -device virtio-serial-device -chardev stdio,id=console,signal=off -device virtconsole,chardev=console

clean:
	rm -rf build