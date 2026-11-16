#!/usr/bin/env bash
# Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
#
# Permission for non-commercial use is hereby granted,
# free of charge, without warranty of any kind.

set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

VIRTIOFS_ROOT="${VIRTIOFS_ROOT:-$ROOT_DIR/build/virtiofs-root}"
VIRTIOFS_SOCKET="${VIRTIOFS_SOCKET:-$ROOT_DIR/build/virtiofs.sock}"

if [[ -n "${VIRTIOFSD:-}" ]]; then
    VIRTIOFSD_BIN=$VIRTIOFSD
elif [[ -x /usr/libexec/virtiofsd ]]; then
    VIRTIOFSD_BIN=/usr/libexec/virtiofsd
else
    VIRTIOFSD_BIN=$(command -v virtiofsd || true)
fi
if [[ -z "$VIRTIOFSD_BIN" ]] || ! command -v "$VIRTIOFSD_BIN" >/dev/null 2>&1; then
    printf 'virtiofsd was not found; set VIRTIOFSD to its executable path.\n' >&2
    exit 1
fi

if [[ -S "$VIRTIOFS_SOCKET" ]]; then
    printf 'VirtIO-FS socket already exists: %s\n' "$VIRTIOFS_SOCKET" >&2
    printf 'Check for an existing QEMU/virtiofsd session before retrying.\n' >&2
    exit 1
fi
if pgrep -af '[v]irtiofsd' | grep -Fq -- "$VIRTIOFS_SOCKET"; then
    printf 'A virtiofsd process is already using %s\n' "$VIRTIOFS_SOCKET" >&2
    exit 1
fi
if pgrep -af '[q]emu-system-x86_64' | grep -Fq -- "$VIRTIOFS_SOCKET"; then
    printf 'A QEMU guest is already using %s\n' "$VIRTIOFS_SOCKET" >&2
    exit 1
fi

mkdir -p "$VIRTIOFS_ROOT" "$(dirname -- "$VIRTIOFS_SOCKET")"

export VIRTIOFS_ROOT VIRTIOFS_SOCKET
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER:-clang}"
export CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER:-$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld/ld.lld}"
export CLANG="${CLANG:-clang}"
export OBJCOPY="${OBJCOPY:-objcopy}"

"$VIRTIOFSD_BIN" --log-level info --socket-path="$VIRTIOFS_SOCKET" \
    --shared-dir="$VIRTIOFS_ROOT" --sandbox=none &
VIRTIOFSD_PID=$!

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    kill "$VIRTIOFSD_PID" 2>/dev/null || true
    wait "$VIRTIOFSD_PID" 2>/dev/null || true
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for ((attempt = 0; attempt < 100; attempt++)); do
    [[ -S "$VIRTIOFS_SOCKET" ]] && break
    if ! kill -0 "$VIRTIOFSD_PID" 2>/dev/null; then
        wait "$VIRTIOFSD_PID" || true
        VIRTIOFSD_PID=
        printf 'virtiofsd exited before creating %s\n' "$VIRTIOFS_SOCKET" >&2
        exit 1
    fi
    sleep 0.1
done
if [[ ! -S "$VIRTIOFS_SOCKET" ]]; then
    printf 'Timed out waiting for VirtIO-FS socket: %s\n' "$VIRTIOFS_SOCKET" >&2
    exit 1
fi

make RUNTIME_V8=0 run-x86_64-virtiofs-gtk
