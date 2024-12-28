## 基于polyhal实现支持多架构的rcore_tutorial_in_single_workspace

​       与rcore_tutorial不同，rcore_tutorial_in_single_workspace把各个实验所使用的公共模块进行了抽离，成为了独立的模块供各个chapter使用，各个chapter只保留了该章节所需要的基本功能和逻辑，其它模块则有着高度的抽象化特征，在移植polyhal和支持多架构的过程中，我们需要分别对各个chapter和crate的代码进行polyhal和多架构的适配工作。因为这个特征，本文档以实现的功能分类，分别介绍各个功能如何进行具体的适配操作。

### 相关命令和测试：

在<arch>架构下运行章节<ch>，<arch>为riscv64，x86_64,aarch64,loongarch64中的一种，<ch>为1-8

```
./run.py --ch <ch> --arch <arch> --release qemu
```

运行ch8后，在shell界面输入usertests即可进行自动测试。

### 1.polyhal的接入和基本输入输出控制

与rcore_tutorial类似，为了使用polyhal，需要先在各个chapter和crate中引入polyhal，因此需要在cargo.toml中增加如下内容：

```toml
polyhal = { git = "https://github.com/xushanpu123/polyhal.git", features = ["kcontext", "trap", "boot"]}
```

由于polyhal还在不断更新中，为了防止出现冲突，本人fork了一份polyhal，并且需要指定特定的feature，kcontext表示可以使用内核上下文接口，trap表示具有中断机制和可以使用中断模块，boot表示自启动。值得注意的是，各个模块均需要引入polyhal，且各个模块引入polyhal时必须使用同样的链接和feature，否则就会造成一份代码链接多份进而产生链接bug。

引入polyhal后，会自动把#[polyhal::arch_entry]后的函数作为入口函数：

```rust
//The entry point
#[polyhal::arch_entry]
fn main(hartid: usize) {
    if hartid != 0 {
        return;
    }
    println!("[kernel] Hello, world!");
    polyhal::shutdown();
}
```

​    

polyhal以#[polyhal::arch_interrupt]后的函数作为中断服务的入口，即使helloworld暂时不需要中断，也必须包含一个该空函数，否则polyhal无法找到interrupt入口会发生链接错误

```rust
/// kernel interrupt
#[polyhal::arch_interrupt]   
fn kernel_interrupt(_ctx: &mut TrapFrame, _trap_type: TrapType) {

}
```

​    

关于输出，可以直接调用polyhal::DebugConsole提供的接口：

```rust
impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.bytes() {
            DebugConsole::putchar(c);
        }
        Ok(())
    }
}
.......
#[inline]
pub fn puts(buffer: &[u8]) {
    // use the main uart if it exists.
    for i in buffer {
        DebugConsole::putchar(*i);
    }
}
```

任务的终止也需要替换为polyhal提供的 Instruction::shutdown()接口。



### 2.ch4地址空间前的任务加载器支持

在支持地址空间之前，原实验并没有开启页映射机制，因为指令采用物理内存地址来访问内存，并且系统和应用都可以无权限限制的访问任意物理内存地址。然而，由于利用polyhal启动时，polyhal会自动开启页机制，因此没有映射的空间并不能够直接使用，没有给予用户访问权限的空间也不能由用户态程序直接访问，因此，在任务加载时，我们必须先对对应的内存地址映射，并给予用户访问权限，这样才能正常加载和使用这些程序。

为了实现这个功能，我们提前移植了页帧分配器和堆分配器，并修改了加载时的代码：

```rust
    polyhal::common::init(&PageAllocImpl);
    get_mem_areas().into_iter().for_each(|(start, size)| {
        info!(
            "frame alloocator add frame {:#x} - {:#x}",
            start,
            start + size
        );
        init_frame_allocator(start, start + size);
    });
```

首先需要在main函数中探测内存空间并初始化页帧分配器。

```rust
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
```

接下来，需要给程序加载的内存空间及其用户栈和内核栈提前进行地址映射并给用户态程序赋权。

而在ch3，多道程序设计的实验中，我们还需要考虑给不同的任务分配不同的页面和栈空间：

```rust
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
```

### 3.多任务调度执行逻辑与中断捕捉与处理

rcore_tutorial_in_single_workspace的主程序逻辑与rcore_tutorial有所不同，首先，它对于每个模块的引入都有显式的初始化过程，若不对对应模块进行初始化，则无法使用对应模块提供的功能。
其次，rcore_tutorial_in_single_workspace使用异界传送门作为内核程序和用户程序之间连接的跳板，异界传送门实际上是内核中分配出来的一块特定的区域，任何内核程序都通过跳转到异界传送门后返回用户程序。
另外，rcore_tutorial_in_single_workspace是通过一个大的循环语句来统一多任务调度切换和中断异常处理的，其基本结构如下面代码所示：

```rust
extern "C" fn rust_main()->!{
//初始化部分
.......
//将initproc装入调度器
......
loop{
//筛选出需要执行的任务
if let Some(task)=unsafe{PROCESSOR.find_next()}{
//跳转到该任务执行
unsafe{task.context.execute(portal,())};
//中断和信号处理部分
...
}
//若无任务,则退出大循环
else{
println!("notask");
break;
}
}

system_reset(Shutdown,NoReason);
unreachable!()
}
```

PROCESSOR``中包含一个就绪态任务队列，``find_next``()``方法会取出队首的任务并返回用户态执行，task_context.excute``()``后面作为中断入口，当发生中断或异常时，程序会跳转到这里执行，进行中断处理和信号处理，而更精妙的设计在于，由于中断和信号处理程序位于``loop``循环中，所以当中断处理和信号处理完成时，内核程序会重新回到``loop``处循环。中断处理和信号处理的返回值分为以下几种情况：
1``、无需调度和终止的中断或系统调用类型：这种情况下，会把当前任务放回任务队列的队首，当重新回到`loop处`循环时会重新执行该任务；
2``、当前任务需要调度但不需要终止：这种情况比如时钟中断下的时间片用完或任务主动放弃``CPU``，此时调用``make_current_suspend``()``接口将当前任务放到任务队列的末尾，则回到下一次``loop``循环时会执行原队列中的下一个任务；
3``、当前任务终止：这种情况下会调用``make_current_exit``()``，当前任务会被丢弃不会再放回任务队列，下一次``loop``会执行下一个任务；
4``、当前任务阻塞：这种情况下会调用``make_current_block``()``，当前任务会被阻塞进入阻塞态，该任务除非被唤醒返回就绪态否则不会再被调度执行，因此下一次``loop``会选择执行下一个任务。
若调用``find_next``()``接口返回``None``，则说明无法获取进程，说明没有处于就绪态的任务了，所有的任务均已经完成，使用``break``跳出循环，内核终止。

为了适配polyhal提供的接口，我们在不破坏其基本逻辑结构的前提下，对代码进行了改写，主要修改方式如下：

1、不再在polyhal::interrupt标记处进行中断处理，而是在返回内核后，利用run_user_task()的返回值进行中断类型的判断和中断（以及信号）处理；

2、为了适配调度接口context_switch_pt()，我们把内核模块变成了多处上下文函数，然后利用任务切换接口来进行跳转；

修改后的代码框架如下：

```rust
pub fn schedule()->!{
	loop{
	if let Some(task)=unsafe{PROCESSOR.find_next()}{
		let mut temp_ctx=KContext::blank();
		unsafe{
		temp_ctx[KContextArgs::KSP]=task.task_cx[KContextArgs::KSP];
		temp_ctx[KContextArgs::KTP]=task.task_cx[KContextArgs::KTP];
		temp_ctx[KContextArgs::KPC]=task_entryasusize;
		//let mut scheduler=&mut*SCHEDULER;
		let new_pagetable = PROCESSOR.get_proc(task.ppid).unwrap().memory_set.token();
		context_switch_pt(
		SCHEDULER.as_mut_ptr(),
		&mut temp_ctx,
		new_pagetable,
		);
	}
}	else{
		println!("notask");
		break;
		}
	}
	Instruction::shutdown();
}

pub fn task_entry(){
	let task=unsafe{PROCESSOR.current().unwrap()};
	unsafe{
		esr=run_user_task(&muttask.trap_cx);
	}
	……
	unsafe{context_switch(&mut_unusedas*mutKContext,SCHEDULER.as_mut_ptr())};
}
```

对于中断处理过程，也需要大量的修改，首先，移植过程中，我们删除了所有原代码中与“异界传送门”机制相关的代码，因为该代码具有强架构相关的特点，而且polyhal提供的run_user_task()接口已经提供了相应的功能，对于具体如何从内核态返回用户态，上述的代码框架已经有所体现。除此之外，在移植过程中，我们必须把所有中断帧结构都替换成polyhal提供的TrapFrame结构，并对所有初始化的行为都修改为对TrapFrame的初始化。更具体来说，除了中断处理层面之外，还需要对process和thread的结构体以及相关的处理函数进行结构体替换 ，如：

```rust
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
```

线程创建时，需要将其中断帧上下文相关的结构全部变更为TrapFrame结构。

### 4.系统调用的收集和处理

本小节只考虑系统调用在内核态下的处理。在本项目的处理中，我们利用run_user_task()提供的返回值esr来判断中断类型，它是一个自定义数据类型EscapeReason，记录了任务从用户态进入内核态的原因，如果该原因为EscapeReason::Syscall，则说明触发了系统调用，而系统调用的类型和参数会保存在中断帧中，因此访问中断帧TrapFrame中的参数即可获取系统调用的具体类型和参数：

```rust
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
            ....
        }
}
```

这样，就由具体的handle函数来完成系统调用的处理了，由于具体的系统调用处理属于不同的子系统，所以这里不做赘述。



### 5.地址空间模块的适配

对地址空间模块的适配性修改集中在kernel-vm中，主要包括四个文件：vpn_range.rs,page_table.rs，memory_set.rs和frame_allocator.rs，这里的主要修改为用polyhal提供的多架构结构和接口来替换掉原先基于riscv sv39页机制实现的接口功能，具体来说就是把所有的虚拟页和物理页修改为polyhal::VirtPage和polyhal::PhysPage，然后清空所有支持页表具体实现的代码，转而使用PageTable及其接口来替代它原先的位置，并进行对应的适配。本部分几乎修改了所有接口的具体实现，这里举例来体现这种修改：

```rust
    pub fn map(&mut self, page_table: &Arc<PageTableWrapper>) {
        // trace!("os::mm::memory_set::MapArea::map");
        for vpn in self.vpn_range {
            // self.map_one(page_table, vpn);
            let p_tracker = frame_alloc().expect("can't allocate frame");
            let flags = self.map_perm.into();
            page_table.map_page(vpn, p_tracker.ppn, flags, MappingSize::Page4KB);
            self.data_frames.insert(vpn, p_tracker);
        }
    }

.......几个
pub struct PageTableWrapper(pub PageTable);
```

例如实现将页表映射时，我们只需要调用polyhal::PageTable结构自带的map_page接口即可。



### 6.文件系统支持

rcore_tutorial_in_single_workspace的文件系统使用了高度模块化的easy_fs，它本来就可以接入任何系统只需要提供给它块设备接口read_block()和write_block()即可，原实验使用VirtIO  Block来作为riscv64架构下的块设备，在接入polyhal和支持多架构后，这一部分内容需要做一定修改：

首先，原实验的内核空间采用的是对等映射，即内核空间中逻辑地址和物理地址是等价的，但是polyhal构建的内核采用的是高半核机制，即内核空间位于整体地址空间的高半核，内核的逻辑地址与物理地址有一个VIRT_ADDR_START大小的偏移量，基于这一点我们需要修改VirtioHal特性中的地址相关内容：

```rust
struct VirtioHal;

impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize {
        // warn!("dma_alloc");
        let paddr: usize =
            unsafe { alloc_zeroed(Layout::from_size_align_unchecked(pages << 12, 1 << 12)) as _ };
        paddr - VIRT_ADDR_START
    }

    fn dma_dealloc(paddr: usize, pages: usize) -> i32 {
        // warn!("dma_dealloc");
        unsafe {
            dealloc(
                (paddr - VIRT_ADDR_START) as _,
                Layout::from_size_align_unchecked(pages << 12, 1 << 12),
            )
        }
        0
    }

    fn phys_to_virt(paddr: usize) -> usize {
        log::warn!("p2v paddr: {:#x}", paddr);
        paddr + VIRT_ADDR_START
    }

    fn virt_to_phys(vaddr: usize) -> usize {
        vaddr - VIRT_ADDR_START
    }
}
```

其次，不同架构的VirtIOBlock的内存映射地址有差异，因此必须依据架构来区分：

```rust
#[cfg(target_arch = "riscv64")]
const VIRTIO0: usize = VIRT_ADDR_START + 0x10001000;

#[cfg(target_arch = "aarch64")]
const VIRTIO0: usize = VIRT_ADDR_START + 0xa00_0000;
```

最后，x86_64和loogarch64并不能通过MMIO方式访问VirtIOBlock，因此我们为它提供了一个逻辑化的块设备RamdiskBlock：

```rust
pub static BLOCK_DEVICE: Lazy<Arc<dyn BlockDevice>> = Lazy::new(|| Arc::new(RamDiskBlock::new()));


// 虚拟IO设备
pub struct RamDiskBlock {
    start: usize,
    size: usize,
}

impl BlockDevice for RamDiskBlock {
    fn read_block(&self, sector_offset: usize, buf: &mut [u8]) {
        assert!(buf.len() == 0x200, "block size is not 0x200");
        let rlen = buf.len();
        if (sector_offset * 0x200 + rlen) >= self.size {
            panic!("can't out of ramdisk range")
        };
        unsafe {
            buf.copy_from_slice(
                core::ptr::slice_from_raw_parts((self.start + sector_offset * 0x200) as *const u8, buf.len())
                    .as_ref()
                    .expect("can't deref ptr in the Ramdisk"),
            );
        }pub struct RamDiskBlock {
    start: usize,
    size: usize,
}

impl BlockDevice for RamDiskBlock {
    fn read_block(&self, sector_offset: usize, buf: &mut [u8]) {
        assert!(buf.len() == 0x200, "block size is not 0x200");
        let rlen = buf.len();
        if (sector_offset * 0x200 + rlen) >= self.size {
            panic!("can't out of ramdisk range")
        };, buf: &[u8]) {
        let wlen = buf.len();
        if (sector_offset * 0x200 + wlen) >= self.size {
            panic!("can't out of ramdisk range")
        };
        unsafe {
            core::ptr::slice_from_raw_parts_mut((self.start + sector_offset * 0x200) as *mut u8, buf.len())
                .as_mut()
                .expect("can't deref ptr in the ramdisk")
                .copy_from_slice(buf);
            // let dest = (self.start as *mut [u8; 512]).add(sector_offset);
            // dest.as_mut().unwrap().copy_from_slice(buf);
        }
    }
}

impl RamDiskBlock {
    pub fn new() -> Self {
        extern "C" {
            fn ramdisk_start();
            fn ramdisk_end();
        }
        log::info!(
            "ramdisk range: {:#x} - {:#x}",
            ramdisk_start as usize, ramdisk_end as usize
        );
        let start = ramdisk_start as _;
        let size = ramdisk_end as usize - ramdisk_start as usize;
        assert_ne!(size, 0, "ramdisk size is 0");
        Self {
            start,
            size,
        }
    }
}

use core::arch::global_asm;



global_asm!(
    concat!(
        " .section .data
          .global ramdisk_start
          .global ramdisk_end
          .p2align 8
          ramdisk_start:
          .incbin \"", env!("TARGET_DIR"),"/fs.img\"
          ramdisk_end:  
        "
    )
);
```

该工作在内存中模拟了一个磁盘块设备，可以正确提供read_block()和write_block()接口。



### 7.其它零零碎碎的支持

在信号模块中，返回用户态时可能需要跳转到自定义函数位置去执行接口，因此UserSignal类型需要维护中断帧上下文来实现这一功能，因此需要用TrapFrame来替换：

```rust
pub enum HandlingSignal {
    Frozen,                   // 是内核信号，需要暂停当前进程
    UserSignal(TrapFrame), // 是用户信号，需要保存之前的用户栈
}
```

```rust
    fn handle_signals(&mut self, current_context: &mut TrapFrame) -> SignalResult {
//......
                _ => {
                    if let Some(action) = self.actions[signal as usize] {
                        // 如果用户给定了处理方式，则按照 SignalAction 中的描述处理
                        // 保存原来用户程序的上下文信息
                        self.handling = Some(HandlingSignal::UserSignal(current_context.clone()));
                        // 修改返回后的 pc 值为 handler，修改 a0 为信号编号
                        //println!("handle pre {:x}, after {:x}", current_context.pc(), action.handler);
                        current_context[TrapFrameArgs::SEPC] = action.handler;    //152行
                        current_context[TrapFrameArgs::ARG0] = signal as usize;
                        SignalResult::Handled
                    } else {
                        // 否则，使用自定义的 DefaultAction 类来处理
                        // 然后再转换成 SignalResult
                        DefaultAction::from(signal).into()
                    }
                }
            }
        } else {
            SignalResult::NoSignal
        }
    }
```

```rust
    fn sig_return(&mut self, current_context: &mut TrapFrame) -> bool {
        let handling_signal = self.handling.take();
        match handling_signal {
            Some(HandlingSignal::UserSignal(old_ctx)) => {
                //println!("return to {:x} a0 {}", old_ctx.pc(), old_ctx.a(0));
                *current_context = old_ctx;
                true
            }
            // 如果当前在处理内核信号，或者没有在处理信号，也就谈不上“返回”了
            _ => {
                self.handling = handling_signal;
                false
            }
        }
    }
```

在sync/up.rs模块，需要访问riscv架构下的sie寄存器，这里替换成polyhal：：IRQ下提供的接口来实现：

```rust
    pub fn enter(&mut self) {
        // let sie = sstatus::read().sie();
        // unsafe {
        //     sstatus::clear_sie();
        // }
        let sie = IRQ::int_enabled();     
        IRQ::int_disable();   
        if self.nested_level == 0 {
            self.sie_before_masking = sie;
        }
        self.nested_level += 1;
    }

    pub fn exit(&mut self) {
        self.nested_level -= 1;
        if self.nested_level == 0 && self.sie_before_masking {
            unsafe {
                // sstatus::set_sie();
                IRQ::int_enabled();
            }
        }
    }
```

时钟中断需要获取时间，在接入polyhal后，利用polyhal::Time模块提供的接口来支持：

```rust
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

```



### 8.多架构支持

原生的rcore_tutorial_in_single_workspace依靠xtask来实现编译和运行内核，由于该工具比较难以维护和使用，因此我们使用python重新写了一个编译和运行脚本来支持多架构编译和运行。

```python
def parseArgs():
    parser = argparse.ArgumentParser(
        description="Run rCore tutorial in the qemu",
        epilog="Use this if you think xtask is difficult",
    )

    # Global Aruguments
    parser.add_argument(
        "--arch",
        "--architecture",
        choices=["riscv64", "aarch64", "x86_64", "loongarch64"],
        required=True,
    )
    parser.add_argument("--ch", "--chatper", type=int)
    parser.add_argument("--release", action="store_true")

    subparsers = parser.add_subparsers(help="sub command to run the kernel.")
    # Build Parser to parser arguments for building
    buildParser = subparsers.add_parser(
        "build", help="Build the source code to binary and elf file."
    )
    buildParser.set_defaults(func=build)
    buildParser.add_argument(
        "-log",
        choices=["trace", "debug", "info", "warn", "error"],
        default="info",
        help="Set the log level for the kernel",
    )
    # qemu Parser to parse arguments for running
    qemuParser = subparsers.add_parser(
        "qemu", help="Run the kernel in the qemu. rely on the build command"
    )
    qemuParser.set_defaults(func=qemu)

    # packfs Parser to parse arguments for pack easyfs
    packfsParser = subparsers.add_parser(
        "packfs", help="Build the easyfs with user tests files."
    )
    packfsParser.set_defaults(func=packfs)
    return parser.parse_args()
```

通过parseArgs()接口，可以接收到命令行输入的目标架构，运行chapter等关键信息，

```python
        command = [
            "cargo",
            "build",
            "--package",
            "user_lib",
            "--target",
            getTarget(),
            "--bin",
            case,
        ]
```

构建用户程序时，利用getTarget()接口获取命令行传入的arch架构，并进行对应的目标架构程序构建。

```python
def build(args):
    packfs(args)
    env = os.environ.copy()
    command = [
        "cargo",
        "build",
        "--target",
        getTarget(),
        "--package",
        "ch" + str(chapter),
    ]
    if chapter >= 2:
        env["APP_ASM"] = getTargetPath() + "/app.asm"
    if args.release:
        command += ["--release"]
    if args.arch == "loongarch64":
        command += ["-Zbuild-std=core,alloc"]
    env["TARGET_DIR"] = getTargetPath()
    res = subprocess.run(command, env=env)
    res.check_returncode()
```

在构建内核时，同样依据传入的参数选择不同的目标架构构建方式。getTargetPath()获取了不同架构的链接文件等必要文件。

```python
# Run Kernel in the qemu
def qemu(args):
    build(args)
    command = ["qemu-system-" + args.arch, "-nographic"]

    objcopy(elfPath, binPath)
    if args.arch == "riscv64":
        command += ["--machine", "virt", "-kernel", binPath]
    elif args.arch == "aarch64":
        command += [
            "-cpu",
            "cortex-a72",
            "-machine",
            "virt",
            "-kernel",
            binPath,
        ]
        # @qemu-system-riscv64 -M 128m -machine virt,dumpdtb=virt.out
	    # fdtdump virt.out
    elif args.arch == "loongarch64":
        command += ["-kernel", elfPath]
    elif args.arch == "x86_64":
        command += ["-machine", "q35", "-kernel", elfPath, "-cpu", "IvyBridge-v2"]
    else:
        print(
            colored(
                "There is currently no support for %s! Comming soon!" % (args.arch),
                "red",
                attrs=["bold"],
            )
        )
        exit(0)

    # 64M is too small, so use 1G instead.
    command += ["-smp", "1", "-m", "1G", "-serial", "mon:stdio"]
    # Add debug arguments to qemu command and record the asm in the file
    command += ["-D", "qemu.log", "-d", "in_asm,int,pcall,cpu_reset,guest_errors"]

    # command += ["-s", "-S"]
    
    if chapter > 5 and args.arch not in ["x86_64", "loongarch64"]:
        command += [
            "-drive",
            "file=%s/fs.img,if=none,format=raw,id=x0" % (getTargetPath()),
            "-device",
            "virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0",
        ]
    print(command)
    subprocess.run(command)
```

关于运行环境的构建，使用同样的思路，依据架构选择不同的qemu类型来运行，并根据不同的架构特征使用不同的运行时参数。注意这里当chapter>5(包含文件系统)时，只有risc64和aarch64架构才连入fs.img镜像和virtio-blk-device，因为x86_64和loongarch64使用的是内存中模拟的ramdiskblock。



接下来需要为user任务发起系统调用提供不同架构的支持，因为它们使用的trap指令是架构相关的，该代码位于syscall模块的user.rs中：

```rust
#[cfg(target_arch = "riscv64")]
pub mod native {
    use crate::SyscallId;
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn syscall0(id: SyscallId) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            out("a0") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(id: SyscallId, a0: usize) -> isize {
        let ret: isize;
        asm!("ecall",
            inlateout("a0") a0 => ret,
            in("a7") id.0,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall2(id: SyscallId, a0: usize, a1: usize) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            inlateout("a0") a0 => ret,
            in("a1") a1,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(id: SyscallId, a0: usize, a1: usize, a2: usize) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(id: SyscallId, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall5(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
    ) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall6(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let ret: isize;
        asm!("ecall",
            in("a7") id.0,
            inlateout("a0") a0 => ret,
            in("a1") a1,
            in("a2") a2,
            in("a3") a3,
            in("a4") a4,
            in("a5") a5,
        );
        ret
    }
}


/// 这个模块包含调用系统调用的最小封装，用户可以直接使用这些函数调用自定义的系统调用。
#[cfg(target_arch = "aarch64")]
pub mod native {
    use crate::SyscallId;
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn syscall0(id: SyscallId) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            out("x0") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(id: SyscallId, a0: usize) -> isize {
        let ret: isize;
        asm!("svc #0",
            inlateout("x0") a0 => ret,
            in("x8") id.0,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall2(id: SyscallId, a0: usize, a1: usize) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            inlateout("x0") a0 => ret,
            in("x1") a1,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(id: SyscallId, a0: usize, a1: usize, a2: usize) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(id: SyscallId, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall5(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
    ) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall6(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let ret: isize;
        asm!("svc #0",
            in("x8") id.0,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            in("x5") a5,
        );
        ret
    }
}

/// 这个模块包含调用系统调用的最小封装，用户可以直接使用这些函数调用自定义的系统调用。
#[cfg(target_arch = "loongarch64")]
pub mod native {
    use crate::SyscallId;
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn syscall0(id: SyscallId) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            out("$r4") ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(id: SyscallId, a0: usize) -> isize {
        let ret: isize;
        asm!("syscall 0",
            inlateout("$r4") a0 => ret,
            in("$r11") id.0,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall2(id: SyscallId, a0: usize, a1: usize) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            inlateout("$r4") a0 => ret,
            in("$r5") a1,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(id: SyscallId, a0: usize, a1: usize, a2: usize) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            inlateout("$r4") a0 => ret,
            in("$r5") a1,
            in("$r6") a2,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(id: SyscallId, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            inlateout("$r4") a0 => ret,
            in("$r5") a1,
            in("$r6") a2,
            in("$r7") a3,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall5(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
    ) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            inlateout("$r4") a0 => ret,
            in("$r5") a1,
            in("$r6") a2,
            in("$r7") a3,
            in("$r8") a4,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall6(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let ret: isize;
        asm!("syscall 0",
            in("$r11") id.0,
            inlateout("$r4") a0 => ret,
            in("$r5") a1,
            in("$r6") a2,
            in("$r7") a3,
            in("$r8") a4,
            in("$r9") a5,
        );
        ret
    }
}


/// 这个模块包含调用系统调用的最小封装，用户可以直接使用这些函数调用自定义的系统调用。
#[cfg(target_arch = "x86_64")]
pub mod native {
    use crate::SyscallId;
    use core::arch::asm;

    #[inline(always)]
    pub unsafe fn syscall0(id: SyscallId) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
                inlateout("rax") id.0 => ret,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall1(id: SyscallId, a0: usize) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall2(id: SyscallId, a0: usize, a1: usize) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
            in("rsi") a1,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall3(id: SyscallId, a0: usize, a1: usize, a2: usize) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall4(id: SyscallId, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall5(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
    ) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8")  a4
        );
        ret
    }

    #[inline(always)]
    pub unsafe fn syscall6(
        id: SyscallId,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let ret: isize;
        asm!("  push r11
                push rcx
                syscall
                pop  rcx
                pop  r11",
            inlateout("rax") id.0 => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8") a4,
            in("r9") a5,
        );
        ret
    }
```

至此，基于polyhal支持多架构的rcore_tutorial_in_single_workspace完成。

### 9.CI说明

CI利用user/src/bin下的usertests.rs测例实现，该测例会进行若干正常测例和非正常退出测例的运行测试，并记录成功测例和失败测例的个数并对比。

CI代码在.github/wokrflows/test.yml中，实际上就是运行了四种架构下的usertests，不过这里比较特殊的是loongarch64架构，polyhal并不支持该架构的停机自动退出，所以CI无法获取最终结果，因此我们不断的检测usertests的输出，出现usertests Passed！即返回成功即可，相关代码如下：

```yml
      ./run.py --ch 8 --arch loongarch64 --release qemu | while IFS= read -r line; do
          echo "$line" # Output each line for debugging
          if [[ "$line" == *"Usertests passed!"* ]]; then
              echo "Detected success message. Exiting with success."
              pkill -P $$ # Kill the run.py process to prevent it from hanging
              exit 0      # Exit successfully
          fi
      done
```

