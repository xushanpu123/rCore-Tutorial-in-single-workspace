#![no_std]

use polyhal::{common::get_fdt, consts::VIRT_ADDR_START};
use virtio_drivers::transport::pci::{bus::{Cam, Command, DeviceFunction, PciRoot}, virtio_device_type};

#[cfg(target_arch = "x86_64")]
pub const PCI_ADDR: usize = 0xb000_0000;

/// Initialize PCI Configuration.
pub fn init() {
    if let Some(fdt) = get_fdt() {
        if let Some(pci_node) = fdt.all_nodes().find(|x| x.name.starts_with("pci")) {
            let pci_addr = pci_node.reg().map(|mut x| x.next().unwrap()).unwrap();
            log::info!("PCI Address: {:#p}", pci_addr.starting_address);
            enumerate_pci((pci_addr.starting_address as usize | VIRT_ADDR_START) as *mut u8);
        }
    }
}

/// Enumerate the PCI devices
fn enumerate_pci(mmconfig_base: *mut u8) {
    log::info!("mmconfig_base = {:#x}", mmconfig_base as usize);

    let mut pci_root = unsafe { PciRoot::new(mmconfig_base, Cam::Ecam) };
    for (device_function, info) in pci_root.enumerate_bus(0) {
        let (status, command) = pci_root.get_status_command(device_function);
        log::info!(
            "Found {} at {}, status {:?} command {:?}",
            info, device_function, status, command
        );

        if let Some(virtio_type) = virtio_device_type(&info) {
            log::info!("  VirtIO {:?}", virtio_type);

            // Enable the device to use its BARs.
            pci_root.set_command(
                device_function,
                Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
            );
            dump_bar_contents(&mut pci_root, device_function, 4);
        }
    }
}

/// Dump bar Contents.
fn dump_bar_contents(root: &mut PciRoot, device_function: DeviceFunction, bar_index: u8) {
    let bar_info = root.bar_info(device_function, bar_index).unwrap();
    if bar_info.memory_address_size().map(|x| x.1).unwrap_or(0) == 0 {
        return;
    }
    log::trace!("Dumping bar {}: {:#x?}", bar_index, bar_info);
}
