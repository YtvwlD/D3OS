//! This modules makes virtio devices available.
//!
//! It doesn't actually contain the drivers – they're in the [`virtio_drivers`]
//! crate – this is just glue code.

use core::ptr::NonNull;

use log::{error, info};
use pci_types::{ConfigRegionAccess, PciAddress};
use virtio_drivers::{
    transport::{
        pci::{
            bus::{ConfigurationAccess, DeviceFunction, DeviceFunctionInfo, PciRoot}, virtio_device_type, PciTransport, VIRTIO_VENDOR_ID
        }, DeviceType, Transport
    }, BufferDirection, Hal
};

use crate::pci_bus;

use super::pci::ConfigurationSpace;

/// Search the PCI bus for virtio devices and initialize them.
// TODO: There are other transports than PCI.
pub fn init() {
    let devices = pci_bus().search_by_vendor_id(VIRTIO_VENDOR_ID);
    // This is using the PCI implementation of virtio_drivers.
    // Could this cause conflicts with our one?
    let mut root = PciRoot::new(PciConfigurationAccess::new(pci_bus().config_space()));
    for device in devices {
        let bar = device.read().bar(0, pci_bus().config_space());
        info!("{bar:?}");
        if let Ok(transport) = PciTransport::new::<VirtioHal, _>(
            &mut root,
            address2function(device.read().header().address()),
        ) {
            let typ = transport.device_type();
            info!("found {typ:?} virtio device");
            match typ {
                t => info!("ignoring {t:?} virtio device"),
            }
        } else {
            error!(
                "failed to initialize transport for virtio device at {:?}",
                device.read().header().address()
            );
        }
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

/// Convert a [`DeviceFunction`] to a [`PciAddress`].
fn function2address(function: DeviceFunction) -> PciAddress {
    PciAddress::new(0x8000, function.bus, function.device, function.function)
}

/// Convert a [`PciAddress`] to a [`DeviceFunction`].
fn address2function(address: PciAddress) -> DeviceFunction {
    assert_eq!(address.segment(), 0x8000);
    DeviceFunction {
        bus: address.bus(),
        device: address.device(),
        function: address.function(),
    }
}

struct PciConfigurationAccess<'a> {
    config_space: &'a ConfigurationSpace,
}

impl<'a> PciConfigurationAccess<'a> {
    fn new(config_space: &'a ConfigurationSpace) -> Self {
        Self { config_space }
    }
}

impl ConfigurationAccess for PciConfigurationAccess<'_> {
    fn read_word(&self, function: DeviceFunction, offset: u8) -> u32 {
        unsafe {
            self.config_space
                .read(function2address(function), offset.into())
        }
    }

    fn write_word(&mut self, function: DeviceFunction, offset: u8, data: u32) {
        unsafe {
            self.config_space
                .write(function2address(function), offset.into(), data)
        };
    }

    unsafe fn unsafe_clone(&self) -> Self {
        todo!()
    }
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
        // TODO: check if this is a mmio region
        // since we have a 1:1 mapping, this is easy
        NonNull::new(paddr as *mut u8).unwrap()
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
