fn main() {
    use std::{env, fs, path::PathBuf};

    let ld = &PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("can't find target");
    fs::write(
        ld,
        linker::SCRIPT.replace("%BASE_ADDRESS%", linker::get_arch_base(&arch).1),
    )
    .unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=APP_ASM");
    println!("cargo:rustc-link-arg=-T{}", ld.display());
}
