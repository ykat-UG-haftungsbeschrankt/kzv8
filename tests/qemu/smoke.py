import socket
import struct
import subprocess
import sys
import tempfile
import threading
from pathlib import Path


TCP_TEST_PORT = 17777
SECTOR_SIZE = 512
DISK_SECTORS = 16 * 1024 * 1024 // SECTOR_SIZE
FAT_PARTITION_START = 2048


def put_u16(buffer, offset, value):
    struct.pack_into("<H", buffer, offset, value)


def put_u32(buffer, offset, value):
    struct.pack_into("<I", buffer, offset, value)


def short_entry(alias, attributes, cluster, size):
    name = alias.encode("ascii")
    if len(name) != 11:
        raise ValueError(f"invalid 8.3 alias: {alias!r}")
    entry = bytearray(32)
    entry[:11] = name
    entry[11] = attributes
    put_u16(entry, 20, cluster >> 16)
    put_u16(entry, 26, cluster & 0xFFFF)
    put_u32(entry, 28, size)
    return entry


def lfn_entries(name, alias):
    short_name = alias.encode("ascii")
    checksum = 0
    for value in short_name:
        checksum = (((checksum & 1) << 7) + (checksum >> 1) + value) & 0xFF
    encoded = name.encode("utf-16le")
    units = list(struct.unpack(f"<{len(encoded) // 2}H", encoded))
    units.append(0)
    while len(units) % 13:
        units.append(0xFFFF)
    positions = (1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30)
    count = len(units) // 13
    entries = []
    for ordinal in range(count, 0, -1):
        entry = bytearray([0xFF] * 32)
        entry[0] = ordinal | (0x40 if ordinal == count else 0)
        entry[11] = 0x0F
        entry[12] = 0
        entry[13] = checksum
        put_u16(entry, 26, 0)
        for index, position in enumerate(positions):
            put_u16(entry, position, units[(ordinal - 1) * 13 + index])
        entries.append(entry)
    return entries


def create_fat16_image(path, app_files):
    disk = bytearray(DISK_SECTORS * SECTOR_SIZE)
    volume_sectors = DISK_SECTORS - FAT_PARTITION_START
    reserved_sectors = 1
    fat_count = 2
    root_entry_count = 512
    root_directory_sectors = root_entry_count * 32 // SECTOR_SIZE
    fat_sectors = 1
    while True:
        clusters = (volume_sectors - reserved_sectors - fat_count * fat_sectors
                    - root_directory_sectors)
        required = ((clusters + 2) * 2 + SECTOR_SIZE - 1) // SECTOR_SIZE
        if required == fat_sectors:
            break
        fat_sectors = required
    cluster_count = (volume_sectors - reserved_sectors - fat_count * fat_sectors
                     - root_directory_sectors)
    if not 4085 <= cluster_count < 65525:
        raise ValueError("test image is not a valid FAT16 volume")

    mbr = memoryview(disk)[:SECTOR_SIZE]
    mbr[446 + 4] = 0x06
    put_u32(mbr, 446 + 8, FAT_PARTITION_START)
    put_u32(mbr, 446 + 12, volume_sectors)
    mbr[510:512] = b"\x55\xaa"

    boot_offset = FAT_PARTITION_START * SECTOR_SIZE
    boot = memoryview(disk)[boot_offset:boot_offset + SECTOR_SIZE]
    boot[:3] = b"\xeb\x3c\x90"
    boot[3:11] = b"MSWIN4.1"
    put_u16(boot, 11, SECTOR_SIZE)
    boot[13] = 1
    put_u16(boot, 14, reserved_sectors)
    boot[16] = fat_count
    put_u16(boot, 17, root_entry_count)
    put_u16(boot, 19, volume_sectors)
    boot[21] = 0xF8
    put_u16(boot, 22, fat_sectors)
    put_u16(boot, 24, 63)
    put_u16(boot, 26, 255)
    put_u32(boot, 28, FAT_PARTITION_START)
    boot[38] = 0x29
    put_u32(boot, 39, 0x4B5A2026)
    boot[43:54] = b"KZAPP      "
    boot[54:62] = b"FAT16   "
    boot[510:512] = b"\x55\xaa"

    fat_start = FAT_PARTITION_START + reserved_sectors

    def set_fat_entry(cluster, value):
        byte_offset = cluster * 2
        for copy in range(fat_count):
            offset = ((fat_start + copy * fat_sectors) * SECTOR_SIZE
                      + byte_offset)
            put_u16(disk, offset, value)

    set_fat_entry(0, 0xFFF8)
    set_fat_entry(1, 0xFFFF)
    set_fat_entry(2, 0xFFFF)

    root_sector = fat_start + fat_count * fat_sectors
    root_offset = root_sector * SECTOR_SIZE
    disk[root_offset:root_offset + 32] = short_entry("APP        ", 0x10, 2, 0)
    data_sector = root_sector + root_directory_sectors
    app_directory = bytearray(SECTOR_SIZE)
    directory_offset = 0
    next_cluster = 3

    for name, alias, contents in app_files:
        contents = bytes(contents)
        cluster_count_for_file = max(1, (len(contents) + SECTOR_SIZE - 1) // SECTOR_SIZE)
        clusters = list(range(next_cluster, next_cluster + cluster_count_for_file))
        next_cluster += cluster_count_for_file
        if next_cluster >= cluster_count + 2:
            raise ValueError("test app exceeds FAT16 image capacity")
        for index, cluster in enumerate(clusters):
            set_fat_entry(cluster, clusters[index + 1] if index + 1 < len(clusters) else 0xFFFF)
            chunk = contents[index * SECTOR_SIZE:(index + 1) * SECTOR_SIZE]
            sector = data_sector + cluster - 2
            data_offset = sector * SECTOR_SIZE
            disk[data_offset:data_offset + len(chunk)] = chunk

        for entry in lfn_entries(name, alias):
            app_directory[directory_offset:directory_offset + 32] = entry
            directory_offset += 32
        entry = short_entry(alias, 0x20, clusters[0], len(contents))
        app_directory[directory_offset:directory_offset + 32] = entry
        directory_offset += 32
        if directory_offset + 32 > SECTOR_SIZE:
            raise ValueError("app directory exceeds its test cluster")

    app_offset = data_sector * SECTOR_SIZE
    disk[app_offset:app_offset + SECTOR_SIZE] = app_directory
    disk[8 * SECTOR_SIZE:9 * SECTOR_SIZE] = bytes([0x5A, 0xA5]) + bytes(510)
    path.write_bytes(disk)


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
        "-accel", "tcg", "-smp", "2" if architecture == "x86_64" else "1",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-no-reboot", "-global", "virtio-mmio.force-legacy=false",
        "-device", "virtio-serial-device",
        "-chardev", "stdio,id=console,signal=off",
        "-device", "virtconsole,chardev=console",
        "-device", "virtio-balloon-device",
    ]
    if architecture == "x86_64":
        command = ["qemu-system-x86_64", "-machine", "microvm,acpi=off", "-cpu", "max", "-m", "5G",
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
        app_dist = Path("app/dist")
        create_fat16_image(disk_path, [
            ("main.js", "MAIN    JS ", (app_dist / "main.js").read_bytes()),
            ("feature-module.js", "FEATUR~1JS ",
             (app_dist / "feature-module.js").read_bytes()),
            ("write-check.bin", "WRITECK BIN", b"write-check"),
        ])
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
    if architecture == "x86_64":
        expected += "kz: smp secondary cpus online\n"
    expected += "kz: virtio balloon ready\n"
    expected += "kz: RAM above 128 MiB detected\n"
    expected += "kz: C stdio printf 42 hello 0x2a\n"
    expected += "kz: C stdio puts works\n"
    expected += "!\n"
    expected += "kz: mmap smoke OK\n"
    expected += "kz: malloc/free smoke OK\n"
    if architecture == "x86_64":
        expected += "kz: RAM above 4 GiB mapped\n"
    expected += "kz: pthreads smoke OK\n"
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
    if architecture == "x86_64":
        expected += "kz: raw block binding OK\n"
        expected += "kz: TCP binding OK\n"
    expected += "kz: V8 script ran\n"
    expected += "kz: V8 event loop smoke OK\n"
    expected += "kz: V8 explicit microtasks OK\n"
    expected += "kz: JS result: 42\n"
    if architecture == "x86_64":
        expected += "kz: VFAT ESM app OK\n"
    expected += "kz: V8 disposing isolate\n"
    expected += "kz: V8 isolate disposed\n"
    expected += "kz: V8 disposing runtime\n"
    expected += "kz: V8 runtime disposed\n"
    expected += "kz: V8 platform disposed\n"
    expected += "kz: V8 evaluated 6 * 7 = 42\n"

    expected_devices = [
        "kz: virtio device_id=3 type=console",
        "kz: virtio device_id=5 type=balloon",
    ]
    if architecture == "x86_64":
        expected_devices.extend([
            "kz: virtio device_id=2 type=block",
            "kz: virtio device_id=1 type=network",
        ])
    actual_devices = [
        line.decode().rstrip("\n")
        for line in output.splitlines(keepends=True)
        if line.startswith(b"kz: virtio device_id=")
    ]
    if sorted(actual_devices) != sorted(expected_devices):
        raise RuntimeError(
            f"{architecture}: unexpected VirtIO devices {actual_devices!r}"
            + (f"\nTCP echo server: {echo_state!r}" if echo_listener else "")
        )
    output_without_device_lines = b"".join(
        line
        for line in output.splitlines(keepends=True)
        if not line.startswith(b"kz: virtio device_id=")
    )
    expected = expected.encode()
    if output_without_device_lines != expected:
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