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

fn main() {
    let package_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_dir = package_dir.parent().unwrap();
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
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
    let v8_platform_output = env::var_os("V8_PLATFORM_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_dir.join("runtime/v8/src/out.gn/x64.platform"));
    let v8_platform_output = if v8_platform_output.is_absolute() {
        v8_platform_output
    } else {
        repository_dir.join(v8_platform_output)
    };

    let (clang_target, sources, linker_script) = match target_arch.as_str() {
        "x86_64" => (
            "x86_64-none-elf",
            vec![("arch/x86_64/boot.S", "boot.o")],
            "linker/x86_64.ld",
        ),
        "aarch64" => (
            "aarch64-none-elf",
            vec![("arch/aarch64/boot.S", "boot.o")],
            "linker/aarch64.ld",
        ),
        _ => panic!("unsupported target architecture: {target_arch}"),
    };

    println!("cargo:rerun-if-env-changed=CLANG");
    println!("cargo:rerun-if-env-changed=OBJCOPY");
    println!("cargo:rerun-if-env-changed=V8_CXX");
    println!("cargo:rerun-if-env-changed=V8_PLATFORM_OUT");
    let linker_script = repository_dir.join(linker_script);
    println!("cargo:rerun-if-changed={}", linker_script.display());
    println!("cargo:rustc-link-arg=-T{}", linker_script.display());

    for (source, object) in sources {
        let source = package_dir.join(source);
        let object = output_dir.join(object);
        let status = Command::new(&clang)
            .arg(format!("--target={clang_target}"))
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
        if target_arch == "x86_64" {
            let status = Command::new(&objcopy)
                .arg("--globalize-symbol=pml4")
                .arg(&object)
                .status()
                .expect("failed to export the x86 PML4 symbol");
            assert!(status.success(), "failed to export the x86 PML4 symbol");
        }
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rustc-link-arg={}", object.display());
    }

    let include_dir = repository_dir.join("runtime/bridge/linux/include");
    let c_sources = [
        ("runtime/bridge/linux/stdio.c", "stdio.o"),
        ("runtime/bridge/linux/string.c", "string.o"),
        ("runtime/bridge/linux/stdio_smoke.c", "stdio_smoke.o"),
        ("runtime/bridge/linux/malloc_smoke.c", "malloc_smoke.o"),
        ("runtime/bridge/linux/mmap_smoke.c", "mmap_smoke.o"),
        ("runtime/bridge/linux/pthread_smoke.c", "pthread_smoke.o"),
    ];

    for header in [
        "stdio.h",
        "string.h",
        "stdlib.h",
        "pthread.h",
        "errno.h",
        "sys/mman.h",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            include_dir.join(header).display()
        );
    }
    for (source, object) in c_sources {
        let source = repository_dir.join(source);
        let object = output_dir.join(object);
        let mut command = Command::new(&clang);
        command
            .arg(format!("--target={clang_target}"))
            .args([
                "-std=c11",
                "-ffreestanding",
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

    if target_arch == "x86_64" {
        let v8_source = repository_dir.join("runtime/v8/src");
        let v8_archive = v8_platform_output.join("obj/libv8_monolith.a");
        let libcxx_archive = v8_platform_output
            .join("obj/buildtools/third_party/libc++/libc++.a");
        let libcxxabi_archive = v8_platform_output
            .join("obj/buildtools/third_party/libc++abi/libc++abi.a");
        for archive in [&v8_archive, &libcxx_archive, &libcxxabi_archive] {
            assert!(archive.is_file(), "missing V8 archive: {}", archive.display());
            println!("cargo:rerun-if-changed={}", archive.display());
        }

        for (source, object) in [
            ("runtime/bridge/src/platform.cc", "v8_platform.o"),
            ("runtime/bridge/src/engine.cc", "v8_engine.o"),
        ] {
            let source = repository_dir.join(source);
            let object = output_dir.join(object);
            let status = Command::new(&v8_cxx)
                .arg("--target=x86_64-unknown-linux-gnu")
                .args([
                    "-std=c++20",
                    "-nostdinc++",
                    "-ffreestanding",
                    "-fno-exceptions",
                    "-fno-rtti",
                    "-fno-stack-protector",
                    "-fno-unwind-tables",
                    "-fno-asynchronous-unwind-tables",
                    "-mno-red-zone",
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
                    "-DV8_TARGET_ARCH_X64",
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
            &v8_platform_output.join(
                "local_rustc_sysroot/lib/rustlib/x86_64-unknown-linux-gnu/lib",
            ),
            &mut rust_archives,
        );
        rust_archives.sort();

        println!("cargo:rustc-link-arg=--start-group");
        for archive in [v8_archive, libcxx_archive, libcxxabi_archive] {
            println!("cargo:rustc-link-arg={}", archive.display());
        }
        for archive in rust_archives {
            println!("cargo:rustc-link-arg={}", archive.display());
        }
        println!("cargo:rustc-link-arg=--end-group");
    }
}
