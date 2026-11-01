use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let package_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository_dir = package_dir.parent().unwrap();
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let clang = env::var_os("CLANG").unwrap_or_else(|| OsString::from("clang"));

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
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rustc-link-arg={}", object.display());
    }
}
