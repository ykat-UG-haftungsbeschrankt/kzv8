#
# Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
#
# Permission for non-commercial use is hereby granted,
# free of charge, without warranty of any kind.
#

	.text
	.globl NtWriteFile
NtWriteFile:
	movq %rcx, %r10
	movl $0x0008, %eax
	syscall
	retq

	.globl NtTerminateProcess
NtTerminateProcess:
	movq %rcx, %r10
	movl $0x002c, %eax
	syscall
	retq

	.globl NtQueryPerformanceCounter
NtQueryPerformanceCounter:
	movq %rcx, %r10
	movl $0x0031, %eax
	syscall
	retq