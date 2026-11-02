import socket
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


TCP_TEST_PORT = 17777


def serve_echo_once(listener, state):
    try:
        connection, _ = listener.accept()
        state["accepted"] = True
        with connection:
            connection.settimeout(10)
            message = bytearray()
            while len(message) < 4:
                chunk = connection.recv(4 - len(message))
                if not chunk:
                    return
                message.extend(chunk)
            state["received"] = bytes(message).hex()
            connection.sendall(message)
            connection.shutdown(socket.SHUT_WR)
    except OSError as error:
        state["error"] = str(error)


def check(architecture):
    common = [
        "-accel", "tcg", "-smp", "1",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-no-reboot", "-global", "virtio-mmio.force-legacy=false",
        "-device", "virtio-serial-device",
        "-chardev", "stdio,id=console,signal=off",
        "-device", "virtconsole,chardev=console",
    ]
    if architecture == "x86_64":
        command = ["qemu-system-x86_64", "-machine", "microvm,acpi=off", "-m", "5G",
                   "-kernel", "build/x86_64/kernel.elf"]
    else:
        command = ["qemu-system-aarch64", "-machine", "virt,gic-version=2",
                   "-cpu", "cortex-a72", "-m", "2G",
                   "-kernel", "build/aarch64/kernel.bin"]
    device_args = []
    temporary_directory = None
    echo_listener = None
    echo_thread = None
    echo_state = {"accepted": False, "received": None, "error": None}
    if architecture == "x86_64":
        temporary_directory = tempfile.TemporaryDirectory()
        disk_path = Path(temporary_directory.name) / "smoke.raw"
        with disk_path.open("wb") as disk:
            disk.truncate(16 * 1024 * 1024)
            disk.seek(8 * 512)
            disk.write(bytes([0x5A, 0xA5]) + bytes(510))
        echo_listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        echo_listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        echo_listener.bind(("0.0.0.0", TCP_TEST_PORT))
        echo_listener.listen(1)
        echo_listener.settimeout(12)
        echo_thread = threading.Thread(
            target=serve_echo_once, args=(echo_listener, echo_state), daemon=True
        )
        echo_thread.start()
        device_args = [
            "-drive", f"file={disk_path},format=raw,if=none,id=smoke-disk",
            "-device", "virtio-blk-device,drive=smoke-disk",
            "-netdev", "user,id=smoke-net",
            "-device", "virtio-net-device,netdev=smoke-net",
        ]
    try:
        try:
            result = subprocess.run(
                command + device_args + common, capture_output=True, timeout=30
            )
            output = result.stdout
            errors = result.stderr.decode(errors="replace")
            if result.returncode != 0:
                raise RuntimeError(errors)
        except subprocess.TimeoutExpired as timeout:
            output = timeout.stdout or b""
            errors = (timeout.stderr or b"").decode(errors="replace")
    finally:
        if echo_listener is not None:
            echo_listener.close()
        if echo_thread is not None:
            echo_thread.join(timeout=1)
        if temporary_directory is not None:
            temporary_directory.cleanup()
    expected = ""
    if architecture == "x86_64":
        expected += "kz: first lazy page fault resolved\n"
    expected += f"kz: {architecture} Rust kernel booted\n"
    expected += "kz: virtio-console transmit OK\n"
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
        expected += "kz: V8 smoke entered\n"
        expected += "kz: V8 flags set\n"
        expected += "kz: V8 platform created\n"
        expected += "kz: V8 platform initialized\n"
        expected += "kz: V8 initialized\n"
        expected += "kz: V8 allocator ready\n"
        expected += "kz: V8 isolate created\n"
        expected += "kz: V8 context created\n"
        expected += "kz: V8 script compiled\n"
        expected += "kz: JavaScript console binding OK\n"
        expected += "kz: raw block binding OK\n"
        expected += "kz: TCP binding OK\n"
        expected += "kz: V8 script ran\n"
        expected += "kz: V8 event loop smoke OK\n"
        expected += "kz: V8 explicit microtasks OK\n"
        expected += "kz: JS result: 42\n"
        expected += "kz: V8 disposing isolate\n"
        expected += "kz: V8 isolate disposed\n"
        expected += "kz: V8 disposing runtime\n"
        expected += "kz: V8 runtime disposed\n"
        expected += "kz: V8 platform disposed\n"
        expected += "kz: V8 evaluated 6 * 7 = 42\n"
    else:
        expected += "kz: V8 not integrated\n"
    expected = expected.encode()
    if output != expected:
        raise RuntimeError(
            f"{architecture}: unexpected output {output!r}\n{errors}"
            + (f"\nTCP echo server: {echo_state!r}" if echo_listener else "")
        )
    print(output.decode(), end="")
    print(f"PASS: {architecture}")


if __name__ == "__main__":
    try:
        check("x86_64")
        check("aarch64")
    except (RuntimeError, FileNotFoundError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)