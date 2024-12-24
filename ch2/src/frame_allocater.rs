use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use core::{
    fmt::{self, Debug, Formatter},
    mem::size_of,
};
use lazy_static::*;
use log::info;
use polyhal::{
    addr::{PhysAddr, PhysPage},
    consts::VIRT_ADDR_START,
    pagetable::PAGE_SIZE,
};
pub struct FrameTracker {
    pub ppn: PhysPage,
}

impl FrameTracker {
    pub fn new(ppn: PhysPage) -> Self {
        // page cleaning
        ppn.drop_clear();
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("FrameTracker:PPN={:?}", self.ppn))
    }
}

impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

trait FrameAllocator {
    fn new() -> Self;
    fn alloc(&mut self) -> Option<PhysPage>;
    fn dealloc(&mut self, ppn: PhysPage);
}

pub struct StackFrameAllocator {
    current: usize,
    end: usize,
    recycled: Vec<usize>,
}

impl StackFrameAllocator {
    pub fn init(&mut self, l: PhysPage, r: PhysPage) {
        self.current = l.as_num();
        self.end = r.as_num();
        println!("last {} Physical Frames.", self.end - self.current);
    }
}
impl FrameAllocator for StackFrameAllocator {
    fn new() -> Self {
        Self {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }
    fn alloc(&mut self) -> Option<PhysPage> {
        if let Some(ppn) = self.recycled.pop() {
            Some(ppn.into())
        } else if self.current == self.end {
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        }
    }
    fn dealloc(&mut self, ppn: PhysPage) {
        let ppn = ppn.as_num();
        // validity check
        if ppn >= self.current || self.recycled.iter().any(|&v| v == ppn) {
            panic!("Frame ppn={:#x} has not been allocated!", ppn);
        }
        // recycle
        self.recycled.push(ppn);
    }
}

type FrameAllocatorImpl = StackFrameAllocator;

lazy_static! {
    pub static ref FRAME_ALLOCATOR: UPSafeCell<FrameAllocatorImpl> =
        unsafe { UPSafeCell::new(FrameAllocatorImpl::new()) };
}

pub fn init_frame_allocator(mm_start: usize, mut mm_end: usize) {
    extern "C" {
        fn end();
    }
    let phys_end = end as usize;
    if phys_end >= mm_start && phys_end < mm_end {
        unsafe {
            core::slice::from_raw_parts_mut(
                phys_end as *mut u128,
                (mm_end - phys_end) / size_of::<u128>(),
            )
            .fill(0);
        }
        let start = ((phys_end + 0xfff) / PAGE_SIZE * PAGE_SIZE) & (!VIRT_ADDR_START);
        mm_end = (mm_end & (!VIRT_ADDR_START));
        info!("add frame mm: {:#x} - {:#x}", start, mm_end);
        FRAME_ALLOCATOR.exclusive_access().init(
            PhysAddr::new(start).into(),
            PhysAddr::new(mm_end & (!VIRT_ADDR_START)).into(),
        );
    }
}

pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
        .inspect(|x| x.ppn.drop_clear())
}

pub fn frame_alloc_persist() -> Option<PhysPage> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .inspect(|x| x.drop_clear())
}

pub fn frame_dealloc(ppn: PhysPage) {
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
}

#[allow(unused)]
pub fn frame_allocator_test() {
    let mut v: Vec<FrameTracker> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}
