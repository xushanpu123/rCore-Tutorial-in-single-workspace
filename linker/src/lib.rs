//! 这个板块为内核提供链接脚本的文本，以及依赖于定制链接脚本的功能。
//!
//! build.rs 文件可依赖此板块，并将 [`SCRIPT`] 文本常量写入链接脚本文件：
//!
//! ```rust
//! use std::{env, fs, path::PathBuf};
//!
//! let ld = &PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("linker.ld");
//! fs::write(ld, linker::SCRIPT).unwrap();
//!
//! println!("cargo:rerun-if-changed=build.rs");
//! println!("cargo:rustc-link-arg=-T{}", ld.display());
//! ```
//!
//! 内核使用 [`boot0`] 宏定义内核启动栈和高级语言入口：
//!
//! ```rust
//! linker::boot0!(rust_main; stack = 4 * 4096);
//! ```
//!
//! 内核所在内核区域定义成 4 个部分（[`KernelRegionTitle`]）:
//!
//! 1. 代码段
//! 2. 只读数据段
//! 3. 数据段
//! 4. 启动数据段
//!
//! 启动数据段放在最后，以便启动完成后换栈。届时可放弃启动数据段，将其加入动态内存区。
//!
//! 用 [`KernelLayout`] 结构体定位、保存和访问内核内存布局。

#![no_std]
// #![deny(warnings, missing_docs)]

#[macro_use]
extern crate rcore_console;

mod app;

pub use app::{AppIterator, AppMeta};

/// 链接脚本。
pub const SCRIPT: &str = "\
OUTPUT_ARCH(riscv)
ENTRY(_start)

BASE_ADDRESS = %BASE_ADDRESS%;

SECTIONS
{
    . = BASE_ADDRESS;
    start = .;
    _skernel = .;

    .text ALIGN(4K): {
        stext = .;
        *(.text.entry)
        *(.text .text.*)
        etext = .;
    }

    .rodata ALIGN(4K): {
        srodata = .;
        *(.rodata .rodata.*)
        . = ALIGN(4K);
        erodata = .;
    }

    .data ALIGN(4K): {
        . = ALIGN(4K);
        *(.data.prepage .data.prepage.*)
        . = ALIGN(4K);
        _sdata = .;
        *(.data .data.*)
        *(.sdata .sdata.*)
        _edata = .;
    }

    _load_end = .;

    .bss ALIGN(4K): {
        *(.bss.stack)
        _sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        _ebss = .;
    }

    PROVIDE(end = .);
    /DISCARD/ : {
        *(.comment) *(.gnu*) *(.note*) *(.eh_frame*)
    }
}";

pub fn get_arch_base(arch: &str) -> (&str, &str) {
    if arch == "x86_64" {
        ("i386:x86-64", "0xffffff8000200000")
    } else if arch.contains("riscv64") {
        ("riscv", "0xffffffc080200000") // OUTPUT_ARCH of both riscv32/riscv64 is "riscv"
    } else if arch.contains("aarch64") {
        ("aarch64", "0xffffff8040080000")
    } else if arch.contains("loongarch64") {
        ("loongarch64", "0x9000000090000000")
    } else {
        core::unreachable!()
    }
}
