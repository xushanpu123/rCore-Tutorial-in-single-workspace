use alloc::{alloc::alloc_zeroed, sync::Arc};
use kernel_vm::MemorySet;
use polyhal::kcontext::{read_current_tp, KContextArgs};
use polyhal::trapframe::TrapFrameArgs;
use polyhal::{kcontext::KContext, trapframe::TrapFrame};
use rcore_console::log;
use core::{alloc::Layout, mem::size_of};
use core::str::FromStr;
use rcore_task_manage::ProcId;
use xmas_elf::{
    header::{self, HeaderPt2, Machine},
    program, ElfFile,
};

pub const KERNEL_STACK_SIZE: usize = 4096 * 2;
pub struct KernelStack {
    inner: Arc<[u128; KERNEL_STACK_SIZE / size_of::<u128>()]>,
}

impl KernelStack {
    pub fn new() -> Self {
        Self {
            inner: Arc::new([0u128; KERNEL_STACK_SIZE / size_of::<u128>()]),
        }
    }

    pub fn get_position(&self) -> (usize, usize) {
        let bottom = self.inner.as_ptr() as usize;
        (bottom, bottom + KERNEL_STACK_SIZE)
    }
}

/// 进程。
pub struct Process {
    /// 不可变
    pub pid: ProcId,
    pub trap_cx: TrapFrame,
    pub task_cx: KContext,
    pub kernel_stack: KernelStack,
    pub memory_set: MemorySet,
}

impl Process {
    pub fn exec(&mut self, elf: ElfFile) {
        let proc = Process::from_elf(elf).unwrap();
        self.memory_set = proc.memory_set;
        self.task_cx = proc.task_cx;
        self.trap_cx = proc.trap_cx;
        self.kernel_stack = proc.kernel_stack;
    }

    pub fn fork(&mut self) -> Option<Process> {
        // 子进程 pid
        let pid = ProcId::new();
        // 复制父进程地址空间
        let parent_addr_space = &self.memory_set;
        let mut address_space = MemorySet::from_existed_user(parent_addr_space);
        let kstack = KernelStack::new();
        // 复制父进程上下文
        Some(Self {
            pid,
            trap_cx: self.trap_cx.clone(),
            task_cx: {
                let mut context = KContext::blank();
                context[KContextArgs::KSP] = kstack.get_position().1;
                context[KContextArgs::KTP] = read_current_tp();
                context
            },
            kernel_stack:kstack,
            memory_set:address_space
        })
    }

    pub fn from_elf(elf: ElfFile) -> Option<Self> {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf);       
        let kstack = KernelStack::new();
        // push a task context which goes to trap_return to the top of kernel stack
        let mut process = Self {
            pid:ProcId::new(),
            trap_cx: TrapFrame::new(),
            task_cx: {
                let mut context = KContext::blank();
                context[KContextArgs::KSP] = kstack.get_position().1;
                context[KContextArgs::KTP] = read_current_tp();
                context
            },
            kernel_stack:kstack,
            memory_set
        };
        // prepare TrapContext in user space
        process.trap_cx[TrapFrameArgs::SEPC] = entry_point;
        log::debug!("user stack pointer: {:#x?}", user_sp);
        process.trap_cx[TrapFrameArgs::SP] = user_sp;
        Some(process)
    }
}
