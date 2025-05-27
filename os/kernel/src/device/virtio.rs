//! This modules makes virtio devices available.
//!
//! It doesn't actually contain the drivers – they're in the [`virtio_drivers`]
//! crate – this is just glue code.

use core::ptr::NonNull;

use log::info;
use virtio_drivers::{
    transport::{pci::{bus::DeviceFunctionInfo, virtio_device_type, VIRTIO_VENDOR_ID}, DeviceType}, BufferDirection, Hal
};

use crate::pci_bus;

/// Search the PCI bus for virtio devices and initialize them.
// TODO: There are other transports than PCI.
pub fn init() {
    let devices = pci_bus().search_by_vendor_id(VIRTIO_VENDOR_ID);
    // we could have used the PCI implementation of virtio_drivers
    for device in devices {
        let lock = device.read();
        let access = pci_bus().config_space();
        let (vendor_id, device_id) = lock.header().id(access);
        let (revision, class, subclass, prog_if) = lock.header().revision_and_class(access);
        match virtio_device_type(&DeviceFunctionInfo {
            vendor_id, device_id,
            class, subclass, prog_if, revision,
            header_type: type2type(lock.header().header_type(access)),
        }) {
            Some(d) => handle_virtio_pci_device(d),
            None => info!("ignoring virtio device {device_id:x}"),
        };
    }
}

/// Convert a [`pci_types::HeaderType`] to a [`virtio_drivers::transport::pci::bus::HeaderType`].
fn type2type(input: pci_types::HeaderType) -> virtio_drivers::transport::pci::bus::HeaderType {
    match input {
        pci_types::HeaderType::Endpoint => virtio_drivers::transport::pci::bus::HeaderType::Standard,
        pci_types::HeaderType::PciPciBridge => virtio_drivers::transport::pci::bus::HeaderType::PciPciBridge,
        pci_types::HeaderType::CardBusBridge => virtio_drivers::transport::pci::bus::HeaderType::PciCardbusBridge,
        pci_types::HeaderType::Unknown(v) => virtio_drivers::transport::pci::bus::HeaderType::Unrecognised(v),
        _ => virtio_drivers::transport::pci::bus::HeaderType::Unrecognised(0),
    }
}

/// Bring up a single virtio-pci device.
fn handle_virtio_pci_device(typ: DeviceType) {
    info!("found {typ:?} virtio device");
    match typ {
        t => info!("ignoring {t:?} virtio device"),
    }
}

struct D3OSHal {}

unsafe impl Hal for D3OSHal {
    fn dma_alloc(
        pages: usize,
        direction: BufferDirection,
    ) -> (virtio_drivers::PhysAddr, NonNull<u8>) {
        todo!()
    }

    unsafe fn dma_dealloc(
        paddr: virtio_drivers::PhysAddr,
        vaddr: NonNull<u8>,
        pages: usize,
    ) -> i32 {
        todo!()
    }

    unsafe fn mmio_phys_to_virt(paddr: virtio_drivers::PhysAddr, size: usize) -> NonNull<u8> {
        todo!()
    }

    unsafe fn share(buffer: NonNull<[u8]>, direction: BufferDirection) -> virtio_drivers::PhysAddr {
        todo!()
    }

    unsafe fn unshare(
        paddr: virtio_drivers::PhysAddr,
        buffer: NonNull<[u8]>,
        direction: BufferDirection,
    ) {
        todo!()
    }
}
