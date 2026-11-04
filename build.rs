/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use std::fs;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect_rlibs(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rlibs(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rlib") {
            output.push(path);
        }
    }
}

fn emit_header_dependencies(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            emit_header_dependencies(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    let package_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_dir = package_dir.clone();
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let v8_target = if target_arch == "aarch64" {
        "aarch64-unknown-linux-gnu"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    let fpu_avx = env::var_os("CARGO_FEATURE_FPU_AVX").is_some();
    let clang = env::var_os("CLANG").unwrap_or_else(|| OsString::from("clang"));
    let objcopy = env::var_os("OBJCOPY").unwrap_or_else(|| OsString::from("llvm-objcopy"));
    let v8_cxx = env::var_os("V8_CXX")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repository_dir.join(path)
            }
        })
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| OsString::from("clang++"));
    let (v8_platform_output_env, default_v8_platform_output) = if target_arch == "aarch64" {
        (
            "V8_AARCH64_PLATFORM_OUT",
            repository_dir.join("runtime/v8/src/out.gn/arm64.platform"),
        )
    } else {
        (
            "V8_PLATFORM_OUT",
            repository_dir.join("runtime/v8/src/out.gn/x64.platform"),
        )
    };
    let v8_platform_output = env::var_os(v8_platform_output_env)
        .map(PathBuf::from)
        .unwrap_or(default_v8_platform_output);
    let v8_platform_output = if v8_platform_output.is_absolute() {
        v8_platform_output
    } else {
        repository_dir.join(v8_platform_output)
    };

    let (clang_target, sources, linker_script) = match target_arch.as_str() {
        "x86_64" => (
            "x86_64-none-elf",
            vec![
                ("kernel/arch/x86_64/boot.S", "boot.o"),
                ("kernel/arch/x86_64/smp_boot.S", "smp_boot.o"),
            ],
            "linker/x86_64.ld",
        ),
        "aarch64" => (
            "aarch64-none-elf",
            vec![
                ("kernel/arch/aarch64/boot.S", "boot.o"),
                ("kernel/arch/aarch64/smp_boot.S", "smp_boot.o"),
            ],
            "linker/aarch64.ld",
        ),
        _ => panic!("unsupported target architecture: {target_arch}"),
    };

    println!("cargo:rerun-if-env-changed=CLANG");
    println!("cargo:rerun-if-env-changed=OBJCOPY");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FPU_AVX");
    println!("cargo:rerun-if-env-changed=V8_CXX");
    println!("cargo:rerun-if-env-changed=V8_PLATFORM_OUT");
    println!("cargo:rerun-if-env-changed=V8_AARCH64_PLATFORM_OUT");
    let linker_script = repository_dir.join(linker_script);
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rustc-link-arg=-T{}", linker_script.display());

    for (source_name, object_name) in sources {
        let source = package_dir.join(source_name);
        let object = output_dir.join(object_name);
        let mut command = Command::new(&clang);
        command.arg(format!("--target={clang_target}"));
        if target_arch == "x86_64" && fpu_avx {
            command.arg("-DKZ_FPU_AVX");
        }
        let status = command
            .arg("-c")
            .arg(&source)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("failed to run Clang");
        assert!(
            status.success(),
            "Clang failed to assemble {}",
            source.display()
        );
        if target_arch == "x86_64" && object_name == "boot.o" {
            let status = Command::new(&objcopy)
                .args([
                    "--globalize-symbol=pml4",
                    "--globalize-symbol=stack_bottom",
                    "--globalize-symbol=stack_top",
                ])
                .arg(&object)
                .status()
                .expect("failed to export the x86 PML4 symbol");
            assert!(status.success(), "failed to export the x86 PML4 symbol");
        }
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rustc-link-arg={}", object.display());
    }

    let include_dir = repository_dir.join("runtime/bridge/linux/include");
    let posix_include_dir = repository_dir.join("runtime/bridge/posix/include");
    let c_sources = [
        ("runtime/bridge/linux/variadic.c", "variadic.o"),
        ("runtime/bridge/posix/monetary.c", "posix_monetary.o"),
        ("runtime/bridge/posix/math.c", "posix_math.o"),
        ("runtime/bridge/posix/mqueue.c", "posix_mqueue.o"),
        ("runtime/bridge/posix/semaphore.c", "posix_semaphore.o"),
        ("runtime/bridge/posix/stdio.c", "posix_stdio.o"),
        ("runtime/bridge/posix/stdlib.c", "posix_stdlib.o"),
        ("runtime/bridge/posix/syslog.c", "posix_syslog.o"),
        ("runtime/bridge/posix/sys/sem.c", "posix_sys_sem.o"),
        ("runtime/bridge/posix/unistd.c", "posix_unistd.o"),
        ("runtime/bridge/posix/wchar.c", "posix_wchar.o"),
    ];

    for header in [
        "stdio.h",
        "string.h",
        "stdlib.h",
        "pthread.h",
        "errno.h",
        "fcntl.h",
        "sys/types.h",
        "sys/stat.h",
        "sys/mman.h",
        "time.h",
        "unistd.h",
        "sys/uio.h",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            include_dir.join(header).display()
        );
    }
    emit_header_dependencies(&posix_include_dir);
    for (source, object) in c_sources {
        let source = repository_dir.join(source);
        let object = output_dir.join(object);
        let mut command = Command::new(&clang);
        command
            .arg(format!("--target={clang_target}"))
            .args([
                "-std=c11",
                "-ffreestanding",
                "-fPIC",
                "-fno-builtin",
                "-fno-stack-protector",
                "-fno-unwind-tables",
                "-fno-asynchronous-unwind-tables",
                "-Wall",
                "-Wextra",
                "-Werror",
            ]);
        if target_arch == "x86_64" {
            command.arg("-mno-red-zone");
        }
        let status = command
            .arg("-I")
            .arg(&posix_include_dir)
            .arg("-I")
            .arg(&include_dir)
            .arg("-c")
            .arg(&source)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("failed to run Clang for freestanding C source");
        assert!(
            status.success(),
            "Clang failed to compile {}",
            source.display()
        );
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rustc-link-arg={}", object.display());
    }

    if target_arch == "x86_64" || target_arch == "aarch64" {
        let v8_source = repository_dir.join("runtime/v8/src");
        let v8_archive = v8_platform_output.join("obj/libv8_monolith.a");
        let target_obj = v8_platform_output.join("obj");
        let libcxx_archive = target_obj.join("buildtools/third_party/libc++/libc++.a");
        let libcxxabi_archive = target_obj.join("buildtools/third_party/libc++abi/libc++abi.a");
        for archive in [&v8_archive, &libcxx_archive, &libcxxabi_archive] {
            assert!(archive.is_file(), "missing V8 archive: {}", archive.display());
            println!("cargo:rerun-if-changed={}", archive.display());
        }
        let compiler_rt_archive = if target_arch == "aarch64" {
            let output = Command::new(&v8_cxx)
                .arg("--print-resource-dir")
                .output()
                .expect("failed to locate the V8 Clang resource directory");
            assert!(output.status.success(), "failed to query the V8 Clang resource directory");
            let resource_dir = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            let archive = resource_dir
                .join("lib/aarch64-unknown-linux-gnu/libclang_rt.builtins.a");
            assert!(archive.is_file(), "missing compiler runtime archive: {}", archive.display());
            println!("cargo:rerun-if-changed={}", archive.display());
            Some(archive)
        } else {
            None
        };

        for (source, object) in [
            ("runtime/bridge/src/platform.cc", "v8_platform.o"),
            ("runtime/bridge/src/engine.cc", "v8_engine.o"),
        ] {
            let source = repository_dir.join(source);
            let object = output_dir.join(object);
            let mut command = Command::new(&v8_cxx);
            command.arg(format!("--target={v8_target}"));
            if target_arch == "x86_64" {
                command.arg("-mno-red-zone");
            }
            let status = command
                .args([
                    "-std=c++20",
                    "-nostdinc++",
                    "-ffreestanding",
                    "-fPIC",
                    "-fno-exceptions",
                    "-fno-rtti",
                    "-fno-stack-protector",
                    "-fno-unwind-tables",
                    "-fno-asynchronous-unwind-tables",
                    "-Wall",
                    "-Wextra",
                    "-Wno-unused-parameter",
                    "-D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_EXTENSIVE",
                    "-D_LIBCPP_DISABLE_VISIBILITY_ANNOTATIONS",
                    "-D_LIBCXXABI_DISABLE_VISIBILITY_ANNOTATIONS",
                    "-D_LIBCPP_INSTRUMENTED_WITH_ASAN=0",
                    "-DKZ_FREESTANDING",
                    "-DV8_HAVE_TARGET_OS",
                    "-DV8_TARGET_OS_LINUX",
                    "-DV8_ENABLE_SANDBOX",
                    "-DV8_COMPRESS_POINTERS",
                    "-DV8_COMPRESS_POINTERS_IN_SHARED_CAGE",
                ])
                .arg("-I")
                .arg(v8_source.join("buildtools/third_party/libc++"))
                .arg("-isystem")
                .arg(v8_source.join("third_party/libc++/src/include"))
                .arg("-isystem")
                .arg(v8_source.join("third_party/libc++abi/src/include"))
                .arg("-I")
                .arg(v8_source.join("include"))
                .arg("-I")
                .arg(v8_platform_output.join("gen/include"))
                .arg("-I")
                .arg(v8_platform_output.join("gen"))
                .arg("-I")
                .arg(v8_source.join("third_party/abseil-cpp"))
                .arg("-I")
                .arg(repository_dir.join("runtime/bridge/include"))
                .arg(if target_arch == "aarch64" {
                    "-DV8_TARGET_ARCH_ARM64"
                } else {
                    "-DV8_TARGET_ARCH_X64"
                })
                .arg("-c")
                .arg(&source)
                .arg("-o")
                .arg(&object)
                .status()
                .expect("failed to compile the V8 kernel bridge");
            assert!(status.success(), "failed to compile {}", source.display());
            println!("cargo:rerun-if-changed={}", source.display());
            println!("cargo:rustc-link-arg={}", object.display());
        }

        let mut rust_archives = Vec::new();
        collect_rlibs(
            &v8_platform_output.join("obj/third_party/rust"),
            &mut rust_archives,
        );
        collect_rlibs(
            &v8_platform_output.join("obj/build/rust/allocator"),
            &mut rust_archives,
        );
        collect_rlibs(
            &v8_platform_output
                .join("local_rustc_sysroot/lib/rustlib")
                .join(v8_target)
                .join("lib"),
            &mut rust_archives,
        );
        rust_archives.sort();

        println!("cargo:rustc-link-arg=--start-group");
        for archive in [v8_archive, libcxx_archive, libcxxabi_archive] {
            println!("cargo:rustc-link-arg={}", archive.display());
        }
        if let Some(archive) = compiler_rt_archive {
            println!("cargo:rustc-link-arg={}", archive.display());
        }
        for archive in rust_archives {
            println!("cargo:rustc-link-arg={}", archive.display());
        }
        println!("cargo:rustc-link-arg=--end-group");
    }
}
