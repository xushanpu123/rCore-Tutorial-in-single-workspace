#![no_std]
#![no_main]
#![feature(naked_functions, asm_const)]
//#![feature(default_alloc_error_handler)]
// #![deny(warnings)]

pub mod process;

#[macro_use]
extern crate rcore_console;

extern crate alloc;

use crate::{impls::SyscallContext, process::Process};
use alloc::vec::Vec;
use impls::Console;
use kernel_vm::{frame_alloc_page_with_clear, frame_dealloc, init_frame_allocator, MemorySet};
use polyhal::{
    common::{get_mem_areas, PageAlloc}, consts::VIRT_ADDR_START, instruction::Instruction, kcontext::*, trap::{run_user_task, EscapeReason, TrapType}, trapframe::{TrapFrame, TrapFrameArgs}, PhysPage
};
use rcore_console::log::{self, info};
use syscall::{Caller, Scheduling};
use xmas_elf::ElfFile;

// 应用程序内联进来。
core::arch::global_asm!(include_str!(env!("APP_ASM")));
// 物理内存容量 = 24 MiB。
const MEMORY: usize = 24 << 20;

// 进程列表。
static mut PROCESSES: Vec<Process> = Vec::new();
static stack: [usize; 256] = [0; 256];
static mut esr: EscapeReason = EscapeReason::NoReason;
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

fn blank_kcontext(ksp: usize, kpc: usize) -> KContext {
    let mut kcx = KContext::blank();
    kcx[KContextArgs::KPC] = kpc;
    kcx[KContextArgs::KSP] = ksp;
    kcx[KContextArgs::KTP] = read_current_tp();
    kcx
}

#[polyhal::arch_interrupt]
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    // match trap_type{
    //     SysCall=>{
    //      log::info!("syscall_type : {} ", ctx[TrapFrameArgs::SYSCALL]);
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
    for (i, elf) in linker::AppMeta::locate().iter(0).enumerate() {
        let base = elf.as_ptr() as usize;
        log::info!("detect app[{i}]: {base:#x}..{:#x}", base + elf.len());
        if let Some(process) = Process::new(ElfFile::new(elf).unwrap()) {
            unsafe { PROCESSES.push(process) };
        }
    }
    // 初始化 syscall
    syscall::init_io(&SyscallContext);
    syscall::init_process(&SyscallContext);
    syscall::init_scheduling(&SyscallContext);
    syscall::init_clock(&SyscallContext);
    // 建立调度线程，目的是划分异常域。调度线程上发生内核异常时会回到这个控制流处理
    // let mut scheduling = KContext::blank();
    // scheduling[KContextArgs::KPC] = schedule as usize;
    // scheduling[KContextArgs::KSP] = stack.as_ptr() as usize + VIRT_ADDR_START;
    // println!("{:x},{:x}",scheduling[KContextArgs::KPC],scheduling[KContextArgs::KSP] );
    // let mut unused = KContext::blank();
    // println!("123");
    // unsafe{context_switch(&mut unused as *mut KContext, &mut scheduling as *mut KContext);}
    schedule();
    // panic!("trap from scheduling thread");
}

pub fn schedule() -> ! {
    while !unsafe { PROCESSES.is_empty() } {
        let mut _unused = KContext::blank();
        let new_pagetable = unsafe { PROCESSES[0].memory_set.token() };
        // log::info!("change pagetable: {:?}", new_pagetable);
        unsafe {
            PROCESSES[0].task_cx[KContextArgs::KPC] = task_entry as usize;
            // let mut scheduler = &mut *SCHEDULER;
            context_switch_pt(&mut _unused as *mut KContext,
                &mut PROCESSES[0].task_cx,
                new_pagetable,
            );
        }
        
    }
    Instruction::shutdown();
}

pub fn task_entry() {
    unsafe {
        esr = run_user_task(&mut PROCESSES[0].trap_cx);
    }
    unsafe {
        match esr {
            EscapeReason::SysCall => {
                use syscall::{SyscallId as Id, SyscallResult as Ret};

                let ctx = unsafe { &mut PROCESSES[0].trap_cx };
                let id: Id = ctx[TrapFrameArgs::SYSCALL].into();
                let args = ctx.args();
                match syscall::handle(Caller { entity: 0, flow: 0 }, id, args) {
                    Ret::Done(ret) => match id {
                        Id::EXIT => unsafe {
                            PROCESSES.remove(0);
                        },
                        Id::SCHED_YIELD => {
                            ctx[TrapFrameArgs::ARG0] = ret as _;
                            ctx[TrapFrameArgs::SEPC] += 4;
                            PROCESSES.rotate_left(1);
                        }
                        _ => {
                            ctx[TrapFrameArgs::ARG0] = ret as _;
                            ctx[TrapFrameArgs::SEPC] += 4;
                        }
                    },
                    Ret::Unsupported(_) => {
                        log::info!("id = {id:?}");
                        unsafe { PROCESSES.remove(0) };
                    }
                }
            }
            EscapeReason::Timer => {
                PROCESSES.rotate_left(1);
            }
            e => {
                unsafe { PROCESSES.remove(0) };
            }
        }
       schedule();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log::error!("{info}");
    Instruction::shutdown();
}

pub fn hexdump(data: &[u8], mut start_addr: usize) {
    const PRELAND_WIDTH: usize = 70;
    println!("{:-^1$}", " hexdump ", PRELAND_WIDTH);
    for offset in (0..data.len()).step_by(16) {
        print!("{:08x} ", start_addr);
        start_addr += 0x10;
        for i in 0..16 {
            if offset + i < data.len() {
                print!("{:02x} ", data[offset + i]);
            } else {
                print!("{:02} ", "");
            }
        }

        print!("{:>6}", ' ');

        for i in 0..16 {
            if offset + i < data.len() {
                let c = data[offset + i];
                if c >= 0x20 && c <= 0x7e {
                    print!("{}", c as char);
                } else {
                    print!(".");
                }
            } else {
                print!("{:02} ", "");
            }
        }

        println!("");
    }
    println!("{:-^1$}", " hexdump end ", PRELAND_WIDTH);
}

/// 各种接口库的实现。
mod impls {
    use crate::PROCESSES;
    use polyhal::{debug_console::DebugConsole, Time};
    use rcore_console::log;
    use syscall::*;
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
    }

    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, _status: usize) -> isize {
            0
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
        fn clock_gettime(&self, caller: Caller, clock_id: ClockId, tp: usize) -> isize {
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
