import subprocess
import sys


def check(architecture):
    common = [
        "-accel", "tcg", "-smp", "2",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-no-reboot", "-global", "virtio-mmio.force-legacy=false",
        "-device", "virtio-serial-device",
        "-chardev", "stdio,id=console,signal=off",
        "-device", "virtconsole,chardev=console",
    ]
    if architecture == "x86_64":
        command = ["qemu-system-x86_64", "-machine", "microvm", "-m", "5G",
                   "-kernel", "build/x86_64/kernel.elf"]
    else:
        command = ["qemu-system-aarch64", "-machine", "virt,gic-version=2",
                   "-cpu", "cortex-a72", "-m", "2G",
                   "-kernel", "build/aarch64/kernel.bin"]
    try:
        result = subprocess.run(command + common, capture_output=True, timeout=5)
        output = result.stdout
        errors = result.stderr.decode(errors="replace")
        if result.returncode != 0:
            raise RuntimeError(errors)
    except subprocess.TimeoutExpired as timeout:
        output = timeout.stdout or b""
        errors = (timeout.stderr or b"").decode(errors="replace")
    expected = f"kz: {architecture} Rust kernel booted\n"
    expected += "kz: virtio-console transmit OK\n"
    expected += "kz: SMP online\n"
    expected += "kz: Rust futures smoke OK\n"
    expected += "kz: C stdio printf 42 hello 0x2a\n"
    expected += "kz: C stdio puts works\n"
    expected += "!\n"
    expected += "kz: RAM above 128 MiB detected\n"
    expected += "kz: mmap smoke OK\n"
    expected += "kz: malloc/free smoke OK\n"
    if architecture == "x86_64":
        expected += "kz: RAM above 4 GiB mapped\n"
    expected += "kz: pthreads smoke OK\n"
    if architecture == "x86_64":
        expected += "kz: V8 page allocator hooks OK\n"
    expected += "kz: V8 not integrated\n"
    expected = expected.encode()
    if output != expected:
        raise RuntimeError(f"{architecture}: unexpected output {output!r}\n{errors}")
    print(output.decode(), end="")
    print(f"PASS: {architecture}")


if __name__ == "__main__":
    try:
        check("x86_64")
        check("aarch64")
    except (RuntimeError, FileNotFoundError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)