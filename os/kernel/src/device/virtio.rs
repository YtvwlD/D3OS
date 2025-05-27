//! This modules makes virtio devices available.
//!
//! It doesn't actually contain the drivers – they're in the [`virtio_drivers`]
//! crate – this is just glue code.

use core::ptr::NonNull;

use log::{error, info};
use virtio_drivers::{
    device::blk::VirtIOBlk, transport::{
        pci::{bus::DeviceFunctionInfo, virtio_device_type, VIRTIO_VENDOR_ID}, DeviceStatus, DeviceType, Transport
    }, BufferDirection, Error, Hal
};
use zerocopy::{FromBytes, Immutable, IntoBytes};

use super::super::pci_bus;

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
            Some(d) => {
                info!("found {d:?} virtio device");
                let transport = VirtioPci::new();
                match d {
                    DeviceType::Block => VirtIOBlk::<VirtioHal, _>::new(&transport)
                    .map(VirtioDevice::Block)
                    .inspect_err(|e| error!("failed to initialize virtio device: {e:?}"))
                    .ok(),
                    t => {
                        info!("ignoring {t:?} virtio device");
                        None
                    },
                };
            },
            None => info!("ignoring virtio device {device_id:x}"),
        };
    }
}

/// Convert a [`pci_types::HeaderType`] to a [`virtio_drivers::transport::pci::bus::HeaderType`].
fn type2type(input: pci_types::HeaderType) -> virtio_drivers::transport::pci::bus::HeaderType {
    match input {
        pci_types::HeaderType::Endpoint => {
            virtio_drivers::transport::pci::bus::HeaderType::Standard
        }
        pci_types::HeaderType::PciPciBridge => {
            virtio_drivers::transport::pci::bus::HeaderType::PciPciBridge
        }
        pci_types::HeaderType::CardBusBridge => {
            virtio_drivers::transport::pci::bus::HeaderType::PciCardbusBridge
        }
        pci_types::HeaderType::Unknown(v) => {
            virtio_drivers::transport::pci::bus::HeaderType::Unrecognised(v)
        }
        _ => virtio_drivers::transport::pci::bus::HeaderType::Unrecognised(0),
    }
}

struct VirtioPci {
}

/// Implements a [`virtio_drivers::transport::Transport`] for PCI.
///
/// We could have used the PCI implementation of virtio_drivers,
/// but then we would need to use MMIO instead of IO ports.
impl VirtioPci {
    /// Bring up a single virtio-pci device.
    fn new() -> Self {
        todo!()
    }
}

impl Transport for &VirtioPci {
    fn device_type(&self) -> DeviceType {
        todo!()
    }

    fn read_device_features(&mut self) -> u64 {
        todo!()
    }

    fn write_driver_features(&mut self, driver_features: u64) {
        todo!()
    }

    fn max_queue_size(&mut self, queue: u16) -> u32 {
        todo!()
    }

    fn notify(&mut self, queue: u16) {
        todo!()
    }

    fn get_status(&self) -> DeviceStatus {
        todo!()
    }

    fn set_status(&mut self, status: DeviceStatus) {
        todo!()
    }

    fn set_guest_page_size(&mut self, guest_page_size: u32) {
        todo!()
    }

    fn requires_legacy_layout(&self) -> bool {
        todo!()
    }

    fn queue_set(
        &mut self,
        queue: u16,
        size: u32,
        descriptors: virtio_drivers::PhysAddr,
        driver_area: virtio_drivers::PhysAddr,
        device_area: virtio_drivers::PhysAddr,
    ) {
        todo!()
    }

    fn queue_unset(&mut self, queue: u16) {
        todo!()
    }

    fn queue_used(&mut self, queue: u16) -> bool {
        todo!()
    }

    fn ack_interrupt(&mut self) -> bool {
        todo!()
    }

    fn read_config_generation(&self) -> u32 {
        todo!()
    }

    fn read_config_space<T: FromBytes + IntoBytes>(
        &self,
        offset: usize,
    ) -> virtio_drivers::Result<T> {
        todo!()
    }

    fn write_config_space<T: IntoBytes + Immutable>(
        &mut self,
        offset: usize,
        value: T,
    ) -> virtio_drivers::Result<()> {
        todo!()
    }
}

enum VirtioDevice<H: Hal, T: Transport> {
    Block(VirtIOBlk<H, T>),
    Unsupported,
}

struct VirtioHal {}

unsafe impl Hal for VirtioHal {
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
