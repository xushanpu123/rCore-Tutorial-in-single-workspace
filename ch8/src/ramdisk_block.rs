use alloc::{
    alloc::{alloc_zeroed, dealloc},
    sync::Arc,
};
use core::{alloc::Layout, ptr::NonNull};
use easy_fs::BlockDevice;
use polyhal::consts::VIRT_ADDR_START;
use spin::{Lazy, Mutex};
use virtio_drivers::{Hal, VirtIOBlk, VirtIOHeader};

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
        }
    }

    fn write_block(&self, sector_offset: usize, buf: &[u8]) {
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

// #[cfg(target_arch = "loongarch64")]
// global_asm!(
//     "
//     .section .data
//     .global ramdisk_start
//     .global ramdisk_end
//     .p2align 8
//     ramdisk_start:
//     .incbin \"/home/yfblock/Code/xushanpu123/rCore-Tutorial-in-single-workspace/target/loongarch64-unknown-none/release/fs.img\"
//     ramdisk_end:
// "
// );

// #[cfg(target_arch = "x86_64")]
// global_asm!(
//     "
//     .section .data
//     .global ramdisk_start
//     .global ramdisk_end
//     .p2align 8
//     ramdisk_start:
//     .incbin \"/home/yfblock/Code/xushanpu123/rCore-Tutorial-in-single-workspace/target/x86_64-unknown-none/release/fs.img\"
//     ramdisk_end:
// "
// );

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