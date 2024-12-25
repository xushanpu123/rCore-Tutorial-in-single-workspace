use alloc::{
    alloc::{alloc_zeroed, dealloc},
    sync::Arc,
};
use polyhal::consts::VIRT_ADDR_START;
use core::{alloc::Layout, ptr::NonNull};
use easy_fs::BlockDevice;
use spin::{Lazy, Mutex};
use virtio_drivers::{Hal, VirtIOBlk, VirtIOHeader};

#[cfg(target_arch = "riscv64")]
const VIRTIO0: usize = VIRT_ADDR_START + 0x10001000;

#[cfg(target_arch = "aarch64")]
const VIRTIO0: usize = VIRT_ADDR_START + 0xa00_0000;


pub static BLOCK_DEVICE: Lazy<Arc<dyn BlockDevice>> = Lazy::new(|| {
    Arc::new(unsafe {
        VirtIOBlock(Mutex::new(
            {
            let v = VirtIOBlk::new(&mut *(VIRTIO0 as *mut VirtIOHeader)).unwrap();
            println!("create succes");
            v
            }
        ))
    })
});

struct VirtIOBlock(Mutex<VirtIOBlk<'static, VirtioHal>>);

impl BlockDevice for VirtIOBlock {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.0
            .lock()
            .read_block(block_id, buf)
            .expect("Error when reading VirtIOBlk");
    }
    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.0
            .lock()
            .write_block(block_id, buf)
            .expect("Error when writing VirtIOBlk");
    }
}

struct VirtioHal;

impl Hal for VirtioHal {
    fn dma_alloc(pages: usize) -> usize {
        // warn!("dma_alloc");
        let paddr: usize = unsafe {
            alloc_zeroed(Layout::from_size_align_unchecked(
                pages << 12,
                1 << 12,
            )) as _
        };
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
        paddr + VIRT_ADDR_START
    }

    fn virt_to_phys(vaddr: usize) -> usize {
        vaddr - VIRT_ADDR_START
    }
}
