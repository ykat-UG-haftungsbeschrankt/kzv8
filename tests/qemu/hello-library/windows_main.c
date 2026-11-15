/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

typedef long NTSTATUS;

__declspec(dllimport) NTSTATUS hello_from_library(void);
__declspec(dllimport) NTSTATUS NtTerminateProcess(void *process, NTSTATUS status);

void mainCRTStartup(void) {
    NtTerminateProcess((void *)-1, hello_from_library());
}