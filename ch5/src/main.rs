#![no_std]
#![no_main]
#![feature(naked_functions, asm_const)]
//#![feature(default_alloc_error_handler)]
// #![deny(warnings)]

mod process;
mod processor;

#[macro_use]
extern crate rcore_console;

extern crate alloc;

use alloc::collections::BTreeMap;
use core::ffi::CStr;
use impls::{Console, SyscallContext};
use kernel_vm::{frame_alloc_page_with_clear, frame_dealloc, init_frame_allocator};
use polyhal::{
    common::{get_mem_areas, PageAlloc},
    instruction::Instruction,
    kcontext::{context_switch_pt, KContext, KContextArgs},
    trap::{run_user_task, EscapeReason, TrapType},
    trapframe::{TrapFrame, TrapFrameArgs},
    PhysPage,
};
use process::Process;
use processor::{ProcManager, PROCESSOR};
use rcore_console::log::{self, info};
use rcore_task_manage::ProcId;
use spin::Lazy;
use syscall::Caller;
use xmas_elf::ElfFile;
static mut esr: EscapeReason = EscapeReason::NoReason;

// 应用程序内联进来。
core::arch::global_asm!(include_str!(env!("APP_ASM")));
// 物理内存容量 = 48 MiB。
/// 加载用户进程。
static APPS: Lazy<BTreeMap<&'static str, &'static [u8]>> = Lazy::new(|| {
    extern "C" {
        static app_names: u8;
    }
    unsafe {
        linker::AppMeta::locate()
            .iter(0)
            .scan(&app_names as *const _ as usize, |addr, data| {
                let name = CStr::from_ptr(*addr as _).to_str().unwrap();
                *addr += name.as_bytes().len() + 1;
                Some((name, data))
            })
    }
    .collect()
});
pub struct PageAllocImpl;

impl PageAlloc for PageAllocImpl {
    #[inline]
    fn alloc(&self) -> PhysPage {
        frame_alloc_page_with_clear().expect("failed to alloc page")
    }

    #[inline]
    fn dealloc(&self, ppn: PhysPage) {
        frame_dealloc(ppn)
    }
}

unsafe fn str_len(ptr: *const u8) -> usize {
    let mut i = 0;
    loop {
        if *ptr.add(i) == 0 {
            break i;
        }
        i += 1;
    }
}

#[polyhal::arch_interrupt]
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    // match trap_type{
    //     SysCall=>{
    // log::info!("syscall_type : {:?} ", ctx[TrapFrameArgs::SYSCALL]);
    //     },
    //     _=>{}
    //      }
}
//The entry point
#[polyhal::arch_entry]
extern "C" fn rust_main() -> ! {
    // 初始化 `console`
    rcore_console::init_console(&Console);
    rcore_console::set_log_level(option_env!("LOG"));
    rcore_console::test_log();
    // 初始化内核堆
    kernel_alloc::init_heap();
    // 加载应用程序
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
    syscall::init_scheduling(&SyscallContext);
    syscall::init_clock(&SyscallContext);
    // 加载初始进程
    let initproc_data = APPS.get("initproc").unwrap();
    // println!("{:?}",initproc_data);
    if let Some(process) = Process::from_elf(ElfFile::new(initproc_data).unwrap()) {
        unsafe {
            PROCESSOR.set_manager(ProcManager::new());
            PROCESSOR.add(process.pid, process, ProcId::from_usize(usize::MAX));
        }
    }
    // if let Some(data)= APPS.get("user_shell")
    // {
    //     println!("success!");
    // }
    // else{
    //     println!("fail");
    // }
    schedule();
}

pub fn schedule() -> ! {
    loop {
        if let Some(task) = unsafe { PROCESSOR.find_next() } {
            let mut _unused = KContext::blank();
            let new_pagetable = unsafe { task.memory_set.token() };
            // log::info!("change pagetable: {:?}", new_pagetable);
            unsafe {
                task.task_cx[KContextArgs::KPC] = task_entry as usize;
                // let mut scheduler = &mut *SCHEDULER;
                context_switch_pt(
                    &mut _unused as *mut KContext,
                    &mut task.task_cx,
                    new_pagetable,
                );
            }
        } else {
            println!("no task");
            break;
        }
    }
    Instruction::shutdown();
}

pub fn task_entry() {
    loop {
        unsafe {
            esr = run_user_task(&mut PROCESSOR.get_current().unwrap().trap_cx);
        }
        unsafe {
            match esr {
                EscapeReason::SysCall => {
                    use syscall::{SyscallId as Id, SyscallResult as Ret};
                    let ctx = &mut PROCESSOR.get_current().unwrap().trap_cx;
                    ctx[TrapFrameArgs::SEPC] += 4;
                    // ctx.move_next();
                    let id: Id = ctx[TrapFrameArgs::SYSCALL].into();
                    let args = ctx.args();
                    match syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                        Ret::Done(ret) => match id {
                            Id::EXIT => unsafe { PROCESSOR.make_current_exited(ret) },
                            _ => {
                                let ctx = &mut PROCESSOR.get_current().unwrap().trap_cx;
                                ctx[TrapFrameArgs::ARG0] = ret as _;
                                unsafe { PROCESSOR.make_current_suspend() };
                            }
                        },
                        Ret::Unsupported(_) => {
                            log::info!("id = {id:?}");
                            unsafe { PROCESSOR.make_current_exited(-2) };
                        }
                    }
                }
                EscapeReason::Timer => {
                    unsafe { PROCESSOR.make_current_suspend() };
                }
                e => {
                    log::error!("unsupported trap: {e:?}");
                    unsafe { PROCESSOR.make_current_exited(-3) };
                }
            }
        }
        schedule();
    }
}

/// Rust 异常处理函数，以异常方式关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    Instruction::shutdown();
}

/// 各种接口库的实现。
mod impls {
    use crate::{str_len, APPS, PROCESSOR};
    use alloc::alloc::alloc_zeroed;
    use core::{alloc::Layout, ptr::NonNull, slice, str::from_utf8_unchecked};
    use polyhal::{debug_console::DebugConsole, trapframe::TrapFrameArgs, Time};
    use rcore_console::log;
    use rcore_task_manage::ProcId;
    use syscall::*;
    use xmas_elf::ElfFile;

    #[repr(transparent)]

    pub struct Console;

    impl rcore_console::Console for Console {
        #[inline]
        fn put_char(&self, c: u8) {
            #[allow(deprecated)]
            DebugConsole::putchar(c as _);
        }
    }

    pub struct SyscallContext;

    impl IO for SyscallContext {
        fn write(&self, caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            match fd {
                STDOUT | STDDEBUG => {
                    print!("{}", unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                            buf as *mut u8,
                            count,
                        ))
                    });
                    count as _
                }
                _ => {
                    log::error!("unsupported fd: {fd}");
                    -1
                }
            }
        }

        #[inline]
        fn read(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            if fd == STDIN {
                for _ in 0..count {
                    loop {
                        if let Some(c) = DebugConsole::getchar() {
                            let c = c as u8;
                            let mut ptr = buf as *mut u8;
                            unsafe {
                                *ptr = c;
                                ptr = ptr.add(1);
                            }
                            break;
                        }
                    }
                }
                count as _
            } else {
                log::error!("unsupported fd: {fd}");
                -1
            }
        }
    }

    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, exit_code: usize) -> isize {
            exit_code as isize
        }

        fn fork(&self, _caller: Caller) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            let mut child_proc = current.fork().unwrap();
            let pid = child_proc.pid;
            let context = &mut child_proc.trap_cx;
            context[TrapFrameArgs::ARG0] = 0 as _;
            unsafe {
                PROCESSOR.add(pid, child_proc, current.pid);
            }
            pid.get_usize() as isize
        }

        fn exec(&self, _caller: Caller, path: usize, count: usize) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            let ptr = path as *const u8;
            let len = unsafe { str_len(ptr) };
            unsafe {
                Some(from_utf8_unchecked(slice::from_raw_parts(ptr, len)))
                    .and_then(|name| APPS.get(name))
                    .and_then(|input| ElfFile::new(input).ok())
                    .map_or_else(
                        || {
                            log::error!("unknown app, select one in the list: ");
                            APPS.keys().for_each(|app| println!("{app}"));
                            println!();
                            -1
                        },
                        |data| {
                            println!("exec success");
                            current.exec(data);
                            0
                        },
                    )
            }
        }

        fn wait(&self, _caller: Caller, pid: isize, exit_code_ptr: usize) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            if let Some((dead_pid, exit_code)) =
                unsafe { PROCESSOR.wait(ProcId::from_usize(pid as usize)) }
            {
                let ptr = exit_code_ptr as *mut isize;
                unsafe { *ptr = exit_code };
                return dead_pid.get_usize() as _;
            } else {
                // 等待的子进程不存在
                return -1;
            }
        }

        fn getpid(&self, _caller: Caller) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            current.pid.get_usize() as _
        }
    }

    impl Scheduling for SyscallContext {
        #[inline]
        fn sched_yield(&self, _caller: Caller) -> isize {
            0
        }
    }

    impl Clock for SyscallContext {
        #[inline]
        fn clock_gettime(&self, _caller: Caller, clock_id: ClockId, tp: usize) -> isize {
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    let time = Time::now().to_usec();
                    *unsafe { &mut *(tp as *mut TimeSpec) } = TimeSpec {
                        tv_sec: time / 1_000_000,
                        tv_nsec: time % 1_000_000,
                    };
                    0
                }
                _ => -1,
            }
        }
    }
}
