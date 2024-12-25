#![no_std]
#![no_main]
#![feature(naked_functions)]
//#![feature(default_alloc_error_handler)]
// #![deny(warnings)]

mod fs;
mod process;
mod processor;
mod virtio_block;

#[macro_use]
extern crate rcore_console;

#[macro_use]
extern crate alloc;

use crate::{
    fs::{read_all, FS},
    impls::SyscallContext,
    process::{Process, Thread},
    processor::{ProcManager, ThreadManager},
};
use alloc::alloc::alloc;
use core::{alloc::Layout, mem::MaybeUninit};
use easy_fs::{FSManager, OpenFlags};
use impls::Console;
use kernel_vm::{frame_alloc_page_with_clear, frame_dealloc, init_frame_allocator};
use polyhal::{
    common::{get_mem_areas, PageAlloc},
    instruction::Instruction,
    kcontext::{context_switch, context_switch_pt, KContext, KContextArgs},
    trap::{run_user_task, EscapeReason, TrapType},
    trapframe::{TrapFrame, TrapFrameArgs},
    PhysPage,
};
pub use processor::PROCESSOR;
use rcore_console::log::{self, info};
use rcore_task_manage::{ProcId, Schedule};
use signal::SignalResult;
use syscall::Caller;
use xmas_elf::ElfFile;
static mut esr: EscapeReason = EscapeReason::NoReason;
use spin::Lazy;
pub static SCHEDULER:Lazy<KContext> = Lazy::new(||KContext::blank());

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

#[polyhal::arch_interrupt]
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    // log::info!("trapType is {:?}",trap_type);
    // match trap_type {
    //     TrapType::StorePageFault(_paddr) => {
    //         log::info!("paddr={:x}", _paddr);
    //     }
    //     TrapType::SysCall=> {
    //         if ctx[TrapFrameArgs::SYSCALL] == 64{
    //             println!("syscall");
    //         }
    //     }
    //     _=>{}
    // }
}

//The entry point
#[polyhal::arch_entry]
extern "C" fn rust_main() -> ! {
    // 初始化 `console`
    rcore_console::init_console(&Console);
    rcore_console::set_log_level(option_env!("LOG"));
    rcore_console::test_log();
    kernel_alloc::init_heap();
    polyhal::common::init(&PageAllocImpl);
    get_mem_areas().into_iter().for_each(|(start, size)| {
        info!(
            "frame alloocator add frame {:#x} - {:#x}",
            start,
            start + size
        );
        init_frame_allocator(start, start + size);
    });
    syscall::init_io(&SyscallContext);
    syscall::init_process(&SyscallContext);
    syscall::init_scheduling(&SyscallContext);
    syscall::init_clock(&SyscallContext);
    syscall::init_signal(&SyscallContext);
    syscall::init_thread(&SyscallContext);
    syscall::init_sync_mutex(&SyscallContext);
    let initproc = read_all(FS.open("usertests", OpenFlags::RDONLY).unwrap());
    if let Some((process, thread)) = Process::from_elf(ElfFile::new(initproc.as_slice()).unwrap()) {
        unsafe {
            PROCESSOR.set_proc_manager(ProcManager::new());
            PROCESSOR.set_manager(ThreadManager::new());
            let (pid, tid) = (process.pid, thread.tid);
            PROCESSOR.add_proc(pid, process, ProcId::from_usize(usize::MAX));
            PROCESSOR.add(tid, thread, pid);
        }
    }
    schedule()
}

pub fn schedule() -> ! {
    loop {
        if let Some(task) = unsafe { PROCESSOR.find_next() } {
            let mut temp_ctx = KContext::blank();
            unsafe {
                temp_ctx[KContextArgs::KSP] = task.task_cx[KContextArgs::KSP];
                temp_ctx[KContextArgs::KTP] = task.task_cx[KContextArgs::KTP];
                temp_ctx[KContextArgs::KPC] = task_entry as usize;
                // let mut scheduler = &mut *SCHEDULER;
                let new_pagetable = PROCESSOR.get_proc(task.ppid).unwrap().memory_set.token();
                context_switch_pt(
                    SCHEDULER.as_mut_ptr(),
                    &mut temp_ctx,
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
    let task = unsafe { PROCESSOR.current().unwrap() };
    unsafe {
        esr = run_user_task(&mut task.trap_cx);
    }
    match unsafe { esr } {
        EscapeReason::SysCall => {
            use syscall::{SyscallId as Id, SyscallResult as Ret};
            let ctx = &mut task.trap_cx;
            // ctx[TrapFrameArgs::SEPC] += 4;
            ctx.syscall_ok();
            let id: Id = ctx[TrapFrameArgs::SYSCALL].into();
            let args = ctx.args();
            let syscall_ret = syscall::handle(Caller { entity: 0, flow: 0 }, id, args);
            // 目前信号处理位置放在 syscall 执行之后，这只是临时的实现。
            // 正确处理信号的位置应该是在 “trap 中处理异常和中断和异常之后，返回用户态之前”。
            // 例如发现有访存异常时，应该触发 SIGSEGV 信号然后进行处理。
            // 但目前 syscall 之后直接切换用户程序，没有 “返回用户态” 这一步，甚至 trap 本身也没了。
            //
            // 最简单粗暴的方法是TrapFrameArgsTrapFrameArgs，在 `scause::Trap` 分类的每一条分支之后都加上信号处理，
            // 当然这样可能代码上不够优雅。处理信号的具体时机还需要后续再讨论。
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            match current_proc.signal.handle_signals(ctx) {
                // 进程应该结束执行
                SignalResult::ProcessKilled(exit_code) => unsafe {
                    PROCESSOR.make_current_exited(exit_code as _)
                },
                _ => match syscall_ret {
                    Ret::Done(ret) => match id {
                        Id::EXIT => unsafe { PROCESSOR.make_current_exited(ret) },
                        Id::SEMAPHORE_DOWN | Id::MUTEX_LOCK | Id::CONDVAR_WAIT => {
                            if ret == -1 {
                                unsafe { PROCESSOR.make_current_blocked() };
                            } else {
                                unsafe { PROCESSOR.make_current_suspend() };
                            }
                        }
                        _ => {
                            let ctx = &mut task.trap_cx;
                            ctx[TrapFrameArgs::ARG0] = ret as _;
                            unsafe { PROCESSOR.make_current_suspend() };
                        }
                    },
                    Ret::Unsupported(_) => {
                        log::info!("id = {id:?}");
                        unsafe { PROCESSOR.make_current_exited(-2) };
                    }
                },
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

    let mut _unused = KContext::blank();
    unsafe {context_switch(&mut _unused as *mut KContext, SCHEDULER.as_mut_ptr())};
}

/// Rust 异常处理函数，以异常方式关机。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("{info}");
    Instruction::shutdown();
}

/// 各种接口库的实现。
mod impls {
    unsafe fn str_len(ptr: *const u8) -> usize {
        let mut i = 0;
        loop {
            if *ptr.add(i) == 0 {
                break i;
            }
            i += 1;
        }
    }

    use crate::{
        fs::{read_all, FS},
        Thread, PROCESSOR,
    };
    use alloc::sync::Arc;
    use alloc::{alloc::alloc_zeroed, string::String, vec::Vec};
    use core::{alloc::Layout, ptr::NonNull, slice, str::from_utf8_unchecked};
    use easy_fs::UserBuffer;
    use easy_fs::{FSManager, OpenFlags};
    use polyhal::{
        debug_console::DebugConsole,
        trapframe::{TrapFrame, TrapFrameArgs},
        Time,
    };
    use rcore_console::log;
    use rcore_task_manage::{ProcId, ThreadId};
    use signal::SignalNo;
    use spin::Mutex;
    use sync::{Condvar, Mutex as MutexTrait, MutexBlocking, Semaphore};
    use syscall::*;
    use xmas_elf::ElfFile;

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
        fn write(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            if fd == STDOUT {
                print!("{}", unsafe {
                    core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                        buf as *mut u8,
                        count,
                    ))
                });
                count as _
            } else if let Some(file) = &current.fd_table[fd] {
                let mut file = file.lock();
                if file.writable() {
                    let mut v: Vec<&'static mut [u8]> = Vec::new();
                    unsafe { v.push(core::slice::from_raw_parts_mut(buf as *mut u8, count)) };
                    file.write(UserBuffer::new(v)) as _
                } else {
                    log::error!("file not writable");
                    -1
                }
            } else {
                log::error!("unsupported fd: {fd}");
                -1
            }
        }

        fn read(&self, _caller: Caller, fd: usize, buf: usize, count: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            if fd == STDIN {
                for _ in 0..count {
                    if let Some(c) = DebugConsole::getchar() {
                        let c = c as u8;
                        let mut ptr = buf as *mut u8;
                        unsafe {
                            *ptr = c;
                            ptr = ptr.add(1);
                        }
                    }
                }
                count as _
            } else if let Some(file) = &current.fd_table[fd] {
                let mut file = file.lock();
                if file.readable() {
                    let mut v: Vec<&'static mut [u8]> = Vec::new();
                    unsafe { v.push(core::slice::from_raw_parts_mut(buf as *mut u8, count)) };
                    file.read(UserBuffer::new(v)) as _
                } else {
                    log::error!("file not readable");
                    -1
                }
            } else {
                log::error!("unsupported fd: {fd}");
                -1
            }
        }

        fn open(&self, _caller: Caller, path: usize, flags: usize) -> isize {
            // FS.open(, flags)
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let mut string = String::new();
            let mut raw_ptr: *mut u8 = path as *mut u8;
            loop {
                unsafe {
                    let ch = *raw_ptr;
                    if ch == 0 {
                        break;
                    }
                    string.push(ch as char);
                    raw_ptr = (raw_ptr as usize + 1) as *mut u8;
                }
            }
            if let Some(fd) = FS.open(string.as_str(), OpenFlags::from_bits(flags as u32).unwrap())
            {
                let new_fd = current.fd_table.len();
                current.fd_table.push(Some(Mutex::new(fd.as_ref().clone())));
                new_fd as isize
            } else {
                -1
            }
        }

        #[inline]
        fn close(&self, _caller: Caller, fd: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            if fd >= current.fd_table.len() || current.fd_table[fd].is_none() {
                return -1;
            }
            current.fd_table[fd].take();
            0
        }
    }

    impl Process for SyscallContext {
        #[inline]
        fn exit(&self, _caller: Caller, exit_code: usize) -> isize {
            exit_code as isize
        }

        fn fork(&self, _caller: Caller) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let _proc = current.fork();
            let (mut child_proc, mut thread) = _proc.unwrap();
            let pid = child_proc.pid;
            let tid = thread.tid;
            let context = &mut thread.trap_cx;
            context[TrapFrameArgs::ARG0] = 0 as _;
            unsafe {
                println!("pid:{:?},tid:{:?}", pid, tid);
                PROCESSOR.add_proc(pid, child_proc, current.pid);
                PROCESSOR.add(tid, thread, pid);
            }
            pid.get_usize() as isize
        }

        fn exec(&self, _caller: Caller, path: usize, count: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let ptr = path as *const u8;
            let len = unsafe { str_len(ptr) };
            Some(unsafe { from_utf8_unchecked(slice::from_raw_parts(ptr, len)) })
                .and_then(|name| {
                    FS.open(name, OpenFlags::RDONLY)
                })
                .map_or_else(
                    || {
                        log::error!("unknown app, select one in the list: ");
                        FS.readdir("")
                            .unwrap()
                            .into_iter()
                            .for_each(|app| println!("{app}"));
                        println!();
                        -1
                    },
                    |fd| {
                        current.exec(ElfFile::new(&read_all(fd)).unwrap());
                        0
                    },
                )
        }

        fn wait(&self, _caller: Caller, pid: isize, exit_code_ptr: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
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
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
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

    impl Signal for SyscallContext {
        fn kill(&self, _caller: Caller, pid: isize, signum: u8) -> isize {
            if let Some(target_task) =
                unsafe { PROCESSOR.get_proc(ProcId::from_usize(pid as usize)) }
            {
                if let Ok(signal_no) = SignalNo::try_from(signum) {
                    if signal_no != SignalNo::ERR {
                        target_task.signal.add_signal(signal_no);
                        return 0;
                    }
                }
            }
            -1
        }

        fn sigaction(
            &self,
            _caller: Caller,
            signum: u8,
            action: usize,
            old_action: usize,
        ) -> isize {
            if signum as usize > signal::MAX_SIG {
                return -1;
            }
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            if let Ok(signal_no) = SignalNo::try_from(signum) {
                if signal_no == SignalNo::ERR {
                    return -1;
                }
                // 如果需要返回原来的处理函数，则从信号模块中获取
                if old_action as usize != 0 {
                    let ptr = old_action as *mut SignalAction;
                    if let Some(signal_action) = current.signal.get_action_ref(signal_no) {
                        unsafe {
                            if let Some(ptr) = ptr.as_mut() {
                                *ptr = signal_action.clone();
                            }
                        }
                    } else {
                        // 如果返回了 None，说明 signal_no 无效
                        return -1;
                    }
                }
                // 如果需要设置新的处理函数，则设置到信号模块中
                if action as usize != 0 {
                    let ptr = action as *mut SignalAction;
                    // 如果返回了 false，说明 signal_no 无效
                    if !current.signal.set_action(signal_no, &unsafe { *ptr }) {
                        return -1;
                    }
                }
                return 0;
            }
            -1
        }

        fn sigprocmask(&self, _caller: Caller, mask: usize) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            current.signal.update_mask(mask) as isize
        }

        fn sigreturn(&self, _caller: Caller) -> isize {
            let current = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let current_thread = unsafe { PROCESSOR.current().unwrap() };
            // 如成功，则需要修改当前用户程序的 LocalContext
            if current.signal.sig_return(&mut current_thread.trap_cx) {
                0
            } else {
                -1
            }
        }
    }

    impl syscall::Thread for SyscallContext {
        fn thread_create(&self, _caller: Caller, entry: usize, arg: usize) -> isize {
            // 主要的问题是用户栈怎么分配，这里不增加其他的数据结构，直接从规定的栈顶的位置从下搜索是否被映射
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let proc_stack_addr = current_proc.usr_stack;
            let pid = current_proc.pid;
            let stack = proc_stack_addr - 0x1000 * 4;
            current_proc.usr_stack = stack;
            let mut ctx = TrapFrame::new();
            ctx[TrapFrameArgs::SEPC] = entry;
            ctx[TrapFrameArgs::SP] = stack;
            ctx[TrapFrameArgs::ARG0] = arg;
            let thread = Thread::new(pid, ctx);
            let tid = thread.tid;
            unsafe {
                PROCESSOR.add(tid, thread, current_proc.pid);
            }
            tid.get_usize() as _
        }

        fn gettid(&self, _caller: Caller) -> isize {
            let current_thread = unsafe { PROCESSOR.current().unwrap() };
            current_thread.tid.get_usize() as _
        }

        fn waittid(&self, _caller: Caller, tid: usize) -> isize {
            let current_thread = unsafe { PROCESSOR.current().unwrap() };
            // 线程不能自己等待自己
            if tid == current_thread.tid.get_usize() {
                return -1;
            }
            // 在当前的进程中查找 tid 对应的线程
            if let Some(exit_code) = unsafe { PROCESSOR.waittid(ThreadId::from_usize(tid)) } {
                exit_code
            } else {
                -1
            }
        }
    }

    impl SyncMutex for SyscallContext {
        fn semaphore_create(&self, _caller: Caller, res_count: usize) -> isize {
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let id = if let Some(id) = current_proc
                .semaphore_list
                .iter()
                .enumerate()
                .find(|(_, item)| item.is_none())
                .map(|(id, _)| id)
            {
                current_proc.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
                id
            } else {
                current_proc
                    .semaphore_list
                    .push(Some(Arc::new(Semaphore::new(res_count))));
                current_proc.semaphore_list.len() - 1
            };
            id as isize
        }

        fn semaphore_up(&self, _caller: Caller, sem_id: usize) -> isize {
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let sem = Arc::clone(current_proc.semaphore_list[sem_id].as_ref().unwrap());
            if let Some(tid) = sem.up() {
                // 释放锁之后，唤醒某个阻塞在此信号量上的线程
                unsafe {
                    PROCESSOR.re_enque(tid);
                }
            }
            0
        }

        fn semaphore_down(&self, _caller: Caller, sem_id: usize) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            let tid = current.tid;
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let sem = Arc::clone(current_proc.semaphore_list[sem_id].as_ref().unwrap());
            if !sem.down(tid) {
                -1
            } else {
                0
            }
        }
        // 虽然提供了标志位来创建不同的锁，但是目前是不支持自旋锁的
        fn mutex_create(&self, _caller: Caller, blocking: bool) -> isize {
            let new_mutex: Option<Arc<dyn MutexTrait>> = if blocking {
                Some(Arc::new(MutexBlocking::new()))
            } else {
                // 本来应该是自旋锁，但是目前还不支持，所以先返回 None
                None
            };
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            if let Some(id) = current_proc
                .mutex_list
                .iter()
                .enumerate()
                .find(|(_, item)| item.is_none())
                .map(|(id, _)| id)
            {
                current_proc.mutex_list[id] = new_mutex;
                id as isize
            } else {
                current_proc.mutex_list.push(new_mutex);
                current_proc.mutex_list.len() as isize - 1
            }
        }

        fn mutex_unlock(&self, _caller: Caller, mutex_id: usize) -> isize {
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let mutex = Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap());
            if let Some(tid) = mutex.unlock() {
                // 释放锁之后，唤醒某个阻塞在此信号量上的线程
                unsafe {
                    PROCESSOR.re_enque(tid);
                }
            }
            0
        }

        fn mutex_lock(&self, _caller: Caller, mutex_id: usize) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            let tid = current.tid;
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let mutex = Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap());
            if !mutex.lock(tid) {
                -1
            } else {
                0
            }
        }

        fn condvar_create(&self, _caller: Caller, _arg: usize) -> isize {
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let id = if let Some(id) = current_proc
                .condvar_list
                .iter()
                .enumerate()
                .find(|(_, item)| item.is_none())
                .map(|(id, _)| id)
            {
                current_proc.condvar_list[id] = Some(Arc::new(Condvar::new()));
                id
            } else {
                current_proc
                    .condvar_list
                    .push(Some(Arc::new(Condvar::new())));
                current_proc.condvar_list.len() - 1
            };
            id as isize
        }

        fn condvar_signal(&self, _caller: Caller, condvar_id: usize) -> isize {
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let condvar = Arc::clone(current_proc.condvar_list[condvar_id].as_ref().unwrap());
            if let Some(tid) = condvar.signal() {
                // 释放锁之后，唤醒某个阻塞在此信号量上的线程
                unsafe {
                    PROCESSOR.re_enque(tid);
                }
            }
            0
        }

        fn condvar_wait(&self, _caller: Caller, condvar_id: usize, mutex_id: usize) -> isize {
            let current = unsafe { PROCESSOR.current().unwrap() };
            let tid = current.tid;
            let current_proc = unsafe { PROCESSOR.get_current_proc().unwrap() };
            let condvar = Arc::clone(current_proc.condvar_list[condvar_id].as_ref().unwrap());
            let mutex = Arc::clone(current_proc.mutex_list[mutex_id].as_ref().unwrap());
            let (flag, waking_tid) = condvar.wait_with_mutex(tid, mutex);
            if let Some(waking_tid) = waking_tid {
                unsafe {
                    PROCESSOR.re_enque(waking_tid);
                }
            }
            if !flag {
                -1
            } else {
                0
            }
        }
    }
}
