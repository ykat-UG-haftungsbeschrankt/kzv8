import subprocess
import sys


def check(architecture):
    common = [
        "-accel", "tcg", "-m", "128M", "-smp", "1",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-no-reboot", "-global", "virtio-mmio.force-legacy=false",
        "-device", "virtio-serial-device",
        "-chardev", "stdio,id=console,signal=off",
        "-device", "virtconsole,chardev=console",
    ]
    if architecture == "x86_64":
        command = ["qemu-system-x86_64", "-machine", "microvm",
                   "-kernel", "build/x86_64/kernel.elf"]
    else:
        command = ["qemu-system-aarch64", "-machine", "virt",
                   "-cpu", "cortex-a72", "-kernel", "build/aarch64/kernel.bin"]
    try:
        result = subprocess.run(command + common, capture_output=True, timeout=5)
        output = result.stdout
        errors = result.stderr.decode(errors="replace")
        if result.returncode != 0:
            raise RuntimeError(errors)
    except subprocess.TimeoutExpired as timeout:
        output = timeout.stdout or b""
        errors = (timeout.stderr or b"").decode(errors="replace")
    expected = (
        f"kz: {architecture} Rust kernel booted\n"
        "kz: virtio-console transmit OK\n"
        "kz: V8 not integrated\n"
    ).encode()
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