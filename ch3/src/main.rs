#![no_std]
#![no_main]
#![feature(naked_functions)]
#![feature(alloc_error_handler)]

mod task;

#[macro_use]
extern crate rcore_console;
extern crate alloc;

// use buddy_system_allocator::LockedHeap;
use frame_allocater::{frame_alloc_persist, frame_dealloc, init_frame_allocator};
//use heap_allocator::init_heap;
use impls::{Console, SyscallContext};
use polyhal::irq::IRQ;
use polyhal::time::Time;
use polyhal::{
    common::{get_mem_areas, PageAlloc},
    consts::VIRT_ADDR_START,
    instruction::Instruction,
    pagetable::PAGE_SIZE,
    trap::{EscapeReason, TrapType},
    trapframe::{TrapFrame, TrapFrameArgs},
    MappingFlags, MappingSize, PageTableWrapper, PhysPage, VirtPage,
};
use rcore_console::log::{self, info};
use task::TaskControlBlock;
pub mod frame_allocater;
//pub mod heap_allocator;
pub mod config;
mod sync;
use alloc::vec::Vec;
use TrapType::*;

// #[global_allocator]
// static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();
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

// 应用程序内联进来。
core::arch::global_asm!(include_str!(env!("APP_ASM")));
// 应用程序数量。
const APP_CAPACITY: usize = 32;
#[polyhal::arch_interrupt]
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {}
//The entry point
#[polyhal::arch_entry]
extern "C" fn rust_main() -> ! {
    // 初始化 `console`
    kernel_alloc::init_heap();
    rcore_console::init_console(&Console);
    rcore_console::set_log_level(option_env!("LOG"));
    rcore_console::test_log();
    // 初始化 syscall
    syscall::init_io(&SyscallContext);
    syscall::init_process(&SyscallContext);
    syscall::init_scheduling(&SyscallContext);
    syscall::init_clock(&SyscallContext);
    polyhal::common::init(&PageAllocImpl);
    get_mem_areas().into_iter().for_each(|(start, size)| {
        info!(
            "frame alloocator add frame {:#x} - {:#x}",
            start,
            start + size
        );
        init_frame_allocator(start, start + size);
    });

    // 任务控制块
    let mut tcbs = Vec::<TaskControlBlock>::new();
    for _ in 0..APP_CAPACITY {
        tcbs.push(TaskControlBlock::zero());
    }
    let mut index_mod = 0;
    // 初始化
    let new_page_table = PageTableWrapper::alloc();
    new_page_table.change();
    for (i, app) in linker::AppMeta::locate().iter(VIRT_ADDR_START).enumerate() {
        println!("{:x}", app.as_ptr() as usize - VIRT_ADDR_START);
        let entry = app.as_ptr() as usize - VIRT_ADDR_START;
        for i in 0..0x20 {
            new_page_table.map_page(
                VirtPage::from_addr(entry + PAGE_SIZE * i),
                PhysPage::from_addr(entry + PAGE_SIZE * i),
                MappingFlags::URWX,
                MappingSize::Page4KB,
            );
        }
        tcbs[i].stack.iter().enumerate().for_each(|(p, ft)| {
            new_page_table.map_page(
                VirtPage::from_addr(0x1_8000_0000 + i * 0x5000 + p  * 0x1000),
                ft.ppn,
                MappingFlags::URWX,
                MappingSize::Page4KB,
            );
        });
        tcbs[i].init(entry, 0x1_8000_0000 + (i + 1) * 0x5000);
        index_mod += 1;
    }
    println!();
    // 多道执行
    let mut remain = index_mod;
    let mut i = 0usize;
    while remain > 0 {
        let tcb = &mut tcbs[i];
        if !tcb.finish {
            let esr = tcb.execute();
            let finish = match esr {
                EscapeReason::Timer => {
                    log::trace!("app{i} timeout");
                    false
                }
                EscapeReason::SysCall => {
                    use task::SchedulingEvent as Event;
                    match tcb.handle_syscall() {
                        Event::None => continue,
                        Event::Exit(code) => {
                            log::info!("app{i} exit with code {code}");
                            true
                        }
                        Event::Yield => {
                            log::debug!("app{i} yield");
                            false
                        }
                        Event::UnsupportedSyscall(id) => {
                            log::error!("app{i} call an unsupported syscall {}", id.0);
                            true
                        }
                    }
                }
                EscapeReason::Exception(e) => {
                    log::error!("app{i} was killed by {e:?}");
                    true
                }
                EscapeReason::NoReason => false,
                _ => {
                    log::error!("app{i} was killed by an unexpected interrupt");
                    true
                }
            };
            if finish {
                tcb.finish = true;
                remain -= 1;
            }
        }
        i = (i + 1) % index_mod;
    }
    Instruction::shutdown();
}

/// Rust 异常处理函数，以异常方式关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    Instruction::shutdown();
}

/// 各种接口库的实现
mod impls {
    use polyhal::{debug_console::DebugConsole, Time};
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
        #[inline]
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

    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: syscall::Caller, _status: usize) -> isize {
            0
        }
    }

    impl Scheduling for SyscallContext {
        #[inline]
        fn sched_yield(&self, _caller: syscall::Caller) -> isize {
            0
        }
    }

    impl Clock for SyscallContext {
        #[inline]
        fn clock_gettime(&self, _caller: syscall::Caller, clock_id: ClockId, tp: usize) -> isize {
            match clock_id {
                ClockId::CLOCK_MONOTONIC => {
                    let time = Time::now().to_usec();
                    println!("time is {}", time);
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
