/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

typedef void *HANDLE;
typedef unsigned long DWORD;
typedef long NTSTATUS;
typedef struct {
    NTSTATUS status;
    unsigned long padding;
    unsigned long long information;
} IO_STATUS_BLOCK;

__declspec(dllimport) NTSTATUS NtQueryPerformanceCounter(
    long long *counter,
    long long *frequency);
__declspec(dllimport) NTSTATUS NtWriteFile(
    HANDLE file,
    HANDLE event,
    void *apc_routine,
    void *apc_context,
    IO_STATUS_BLOCK *io_status,
    const void *buffer,
    DWORD length,
    const long long *byte_offset,
    DWORD *key);

__declspec(dllexport) NTSTATUS hello_from_library(void) {
    static const char message[] = "Hello, world from the Windows DLL!\r\n";
    long long counter;
    long long frequency;
    IO_STATUS_BLOCK io_status;

    if (NtQueryPerformanceCounter(&counter, &frequency) < 0) {
        return -1;
    }
    return NtWriteFile(
        (HANDLE)1,
        0,
        0,
        0,
        &io_status,
        message,
        sizeof(message) - 1,
        0,
        0);
}