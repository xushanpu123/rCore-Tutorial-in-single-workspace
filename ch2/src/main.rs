#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(alloc_error_handler)]
// #![deny(warnings)]

#[macro_use]
extern crate rcore_console;
extern crate alloc;
use core::fmt::Debug;

use buddy_system_allocator::LockedHeap;
use frame_allocater::{frame_alloc_persist, frame_dealloc, init_frame_allocator};
use heap_allocator::init_heap;
use impls::{Console, SyscallContext};
use log::info;
use polyhal::boot::boot_page_table;
use polyhal::common::{get_mem_areas, PageAlloc};
use polyhal::consts::VIRT_ADDR_START;
use polyhal::debug_console::DebugConsole;
use polyhal::instruction::Instruction;
use polyhal::pagetable::PageTableWrapper;
use polyhal::pagetable::PAGE_SIZE;
use polyhal::trap::{
    run_user_task, EscapeReason,
    TrapType::{self, *},
};
use polyhal::trapframe::{TrapFrame, TrapFrameArgs};
use polyhal::{MappingFlags, MappingSize, PageTable, PhysPage, VirtPage};
use rcore_console::{init_console, log, set_log_level};
use syscall::{Caller, SyscallId};
mod config;
pub mod frame_allocater;
pub mod heap_allocator;
mod sync;
#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();
pub struct PageAllocImpl;

impl PageAlloc for PageAllocImpl {
    #[inline]
    fn alloc(&self) -> PhysPage {
        frame_alloc_persist().expect("can't find memory page")
    }

    #[inline]
    fn dealloc(&self, ppn: PhysPage) {
        frame_dealloc(ppn)
    }
}

// 用户程序内联进来。
core::arch::global_asm!(include_str!(env!("APP_ASM")));

static mut Exit: bool = false;
#[polyhal::arch_interrupt]
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    // log::info!("trap_type @ {:x?} {:#x?}", trap_type, ctx);

    match trap_type {
        Timer | SysCall => {}
        StorePageFault(_paddr) | LoadPageFault(_paddr) | InstructionPageFault(_paddr) => {
            println!(
                "[kernel] PageFault in application, kernel killed it. paddr={:x}",
                _paddr
            );
            unsafe {
                Exit = true;
            }
        }
        IllegalInstruction(_) => unsafe {
            Exit = true;
        },
        _ => {
            panic!("{:?}", trap_type);
        }
    }
}
//The entry point
#[polyhal::arch_entry]
extern "C" fn rust_main() -> ! {
    log::info!("hello!");
    init_heap();
    init_console(&impls::Console);
    set_log_level(Some("Debug"));
    polyhal::common::init(&PageAllocImpl);
    get_mem_areas().into_iter().for_each(|(start, size)| {
        info!(
            "frame alloocator add frame {:#x} - {:#x}",
            start,
            start + size
        );
        init_frame_allocator(start, start + size);
    });

    // 初始化 syscall
    syscall::init_io(&SyscallContext);
    syscall::init_process(&SyscallContext);
    
    for (i, app) in linker::AppMeta::locate().iter(VIRT_ADDR_START).enumerate() {
        let new_page_table = PageTableWrapper::alloc();
        new_page_table.change();
        let app_base = app.as_ptr() as usize & (!VIRT_ADDR_START);

        for i in 0..0x20 {
            new_page_table.map_page(
                VirtPage::from_addr(app_base + PAGE_SIZE * i),
                PhysPage::from_addr(
                    app_base + PAGE_SIZE * i),
                MappingFlags::URWX,
                MappingSize::Page4KB,
            );
        }
        new_page_table.map_page(
            VirtPage::from_addr(0x1_8000_0000),
            frame_alloc_persist().expect("can't allocate frame"),
            MappingFlags::URWX,
            MappingSize::Page4KB,
        );

        log::info!("[kernel] Loading app_{}", i);
        log::info!("load app{i} to {app_base:#x}");
        // 初始化上下文
        let mut ctx = TrapFrame::new();
        ctx[TrapFrameArgs::SEPC] = app_base;
        ctx[TrapFrameArgs::SP] = 0x1_8000_0000 + PAGE_SIZE;
        new_page_table.change();

        let mut ctx_mut = unsafe { (&mut ctx as *mut TrapFrame).as_mut().unwrap() };
        // 执行应用程序
        loop {
            let esr = run_user_task(ctx_mut);

            match esr {
                EscapeReason::SysCall => {
                    ctx_mut.syscall_ok();
                    use SyscallResult::*;
                    match handle_syscall(&mut ctx_mut) {
                        Done => continue,
                        Exit(code) => {
                            log::info!("app{i} exit with code {code}");
                            break;
                        }
                        Error(id) => {
                            log::error!("app{i} call an unsupported syscall {}", id.0);
                            break;
                        }
                    }
                }
                _ => {}
            }
            unsafe {
                if (Exit == true) {
                    Exit = false;
                    break;
                }
            }
        }
        boot_page_table().change();
    }
    Instruction::shutdown();
}

/// Rust 异常处理函数，以异常方式关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    Instruction::shutdown();
}

enum SyscallResult {
    Done,
    Exit(usize),
    Error(SyscallId),
}

/// 处理系统调用，返回是否应该终止程序。
fn handle_syscall(ctx: &mut TrapFrame) -> SyscallResult {
    use syscall::{SyscallId as Id, SyscallResult as Ret};
    let args = ctx.args();
    let id = ctx[TrapFrameArgs::SYSCALL].into();
    match syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
        Ret::Done(ret) => match id {
            Id::EXIT => SyscallResult::Exit(args[0]),
            _ => SyscallResult::Done,
        },
        Ret::Unsupported(id) => SyscallResult::Error(id),
    }
}

/// 各种接口库的实现print
mod impls {
    use polyhal::debug_console::DebugConsole;
    use syscall::{STDDEBUG, STDOUT};

    pub struct Console;

    impl rcore_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            #[allow(deprecated)]
            DebugConsole::putchar(c as _);
        }
    }

    pub struct SyscallContext;

    impl syscall::IO for SyscallContext {
        fn write(&self, _caller: syscall::Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    print!("{}", unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            buf as *const u8,
                            count,
                        ))
                    });
                    count as _
                }
                _ => {
                    rcore_console::log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }
    }

    impl syscall::Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: syscall::Caller, _status: usize) -> isize {
            0
        }
    }
}
