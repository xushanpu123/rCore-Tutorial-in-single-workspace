fn main() {
    use std::{env, fs, path::PathBuf};

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LOG");
    println!("cargo:rerun-if-env-changed=BASE_ADDRESS");

    if let Some(base) = env::var("BASE_ADDRESS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        let text = format!(
            "\
OUTPUT_ARCH(riscv)
ENTRY(_start)
SECTIONS {{
    . = {base};
    . = ALIGN(4K);
    .text : ALIGN(4K) {{
        *(.text.entry)
        *(.text .text.*)
    }}
    . = ALIGN(4K);
    .rodata : ALIGN(4K) {{
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }}
    . = ALIGN(4K);
    .got : ALIGN(4K) {{
        *(.got .got.*)
    }}
    . = ALIGN(4K);
    .data : ALIGN(4K) {{
        *(.data .data.*)
        *(.sdata .sdata.*)
    }}
    . = ALIGN(4K);
    .bss : ALIGN(4K) {{
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }}
}}"
        );
        let ld = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
        fs::write(&ld, text).unwrap();
        println!("cargo:rustc-link-arg=-T{}", ld.display());
    } else {
        let text = format!(
            "\
OUTPUT_ARCH(riscv)
ENTRY(_start)
SECTIONS {{
    . = ALIGN(4K);
    .text : ALIGN(4K) {{
        *(.text.entry)
        *(.text .text.*)
    }}
    . = ALIGN(4K);
    .rodata : ALIGN(4K) {{
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }}
    . = ALIGN(4K);
    .got : ALIGN(4K) {{
        *(.got .got.*)
    }}
    . = ALIGN(4K);
    .data : ALIGN(4K) {{
        *(.data .data.*)
        *(.sdata .sdata.*)
    }}
    . = ALIGN(4K);
    .bss : ALIGN(4K) {{
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }}
}}"
        );
        let ld = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
        fs::write(&ld, text).unwrap();
        println!("cargo:rustc-link-arg=-T{}", ld.display());
    }
}
