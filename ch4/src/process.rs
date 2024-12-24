use kernel_vm::MemorySet;
use polyhal::{kcontext::{read_current_tp, KContext, KContextArgs}, trapframe::{TrapFrame, TrapFrameArgs}};
use rcore_console::log;
use alloc::sync::Arc;
use xmas_elf::ElfFile;
use core::mem::size_of;
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
#[repr(align(64))]
pub struct Process {
    pub trap_cx: TrapFrame,
    pub task_cx: KContext,
    pub kernel_stack: KernelStack,
    pub memory_set: MemorySet,
}

impl Process {
    pub fn new(elf: ElfFile) -> Option<Self> {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf);
        println!("entry_point:{:#x}",entry_point);
        // alloc a kernel stack in kernel space
        let kstack = KernelStack::new();
        // push a task context which goes to trap_return to the top of kernel stack
        let mut process = Self {
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
        log::debug!("sp: {:#x}", user_sp);
        process.trap_cx[TrapFrameArgs::SP] = user_sp;
        Some(process)
    }
}
