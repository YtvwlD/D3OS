//! This modules makes virtio devices available.
//!
//! It doesn't actually contain the drivers – they're in the [`virtio_drivers`]
//! crate – this is just glue code.

use core::ptr::NonNull;

use log::{error, info};
use pci_types::{capability::PciCapability, Bar, ConfigRegionAccess, EndpointHeader};
use spin::RwLockReadGuard;
use virtio_drivers::{
    BufferDirection, Hal,
    device::blk::VirtIOBlk,
    transport::{
        DeviceStatus, DeviceType, Transport,
        pci::{
            VIRTIO_PCI_CAP_COMMON_CFG, VIRTIO_PCI_CAP_DEVICE_CFG, VIRTIO_PCI_CAP_ISR_CFG,
            VIRTIO_PCI_CAP_NOTIFY_CFG, VIRTIO_VENDOR_ID, VirtioPciError, bus::DeviceFunctionInfo,
            virtio_device_type,
        }
    },
};
use zerocopy::{FromBytes, Immutable, IntoBytes};

use super::pci::ConfigurationSpace;
use super::super::pci_bus;

/// Search the PCI bus for virtio devices and initialize them.
// TODO: There are other transports than PCI.
pub fn init() {
    let devices = pci_bus().search_by_vendor_id(VIRTIO_VENDOR_ID);
    // we could have used the PCI implementation of virtio_drivers
    for device in devices {
        let lock = device.read();
        let config_space = pci_bus().config_space();
        let (vendor_id, device_id) = lock.header().id(config_space);
        let (revision, class, subclass, prog_if) = lock.header().revision_and_class(config_space);
        match virtio_device_type(&DeviceFunctionInfo {
            vendor_id, device_id,
            class, subclass, prog_if, revision,
            header_type: type2type(lock.header().header_type(config_space)),
        }) {
            Some(typ) => {
                info!("found {typ:?} virtio device");
                let transport = VirtioPci::new(lock, typ, config_space)
                    .expect("failed to setup virtio transport");
                match typ {
                    DeviceType::Block => VirtIOBlk::<VirtioHal, _>::new(&transport)
                    .map(VirtioDevice::Block)
                    .inspect_err(|e| error!("failed to initialize virtio device: {e:?}"))
                    .ok(),
                    t => {
                        info!("ignoring {t:?} virtio device");
                        None
                    }
                };
            }
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
    // this is taken from virtio_drivers::transport::Pci::PciTransport::new,
    // but modified to use our PCI implementation
    device_type: DeviceType,
    /// The common configuration structure within some BAR.
    common_cfg: Bar,
    /// The start of the queue notification region within some BAR.
    notify_region: Bar,
    notify_off_multiplier: u32,
    /// The ISR status register within some BAR.
    isr_status: Bar,
    /// The VirtIO device-specific configuration within some BAR.
    config_space: Option<Bar>,
}

/// Implements a [`virtio_drivers::transport::Transport`] for PCI.
///
/// We could have used the PCI implementation of virtio_drivers,
/// but then we would need to use MMIO instead of IO ports.
impl VirtioPci {
    /// Bring up the transport for a single virtio-pci device.
    fn new(
        device: RwLockReadGuard<'_, EndpointHeader>,
        device_type: DeviceType,
        config_space: &ConfigurationSpace,
    ) -> Result<Self, VirtioPciError> {
        // this is taken from virtio_drivers::transport::Pci::PciTransport::new,
        // but modified to use our PCI implementation
        
        // Find the PCI capabilities we need.
        let mut common_cfg = None;
        let mut notify_cfg = None;
        let mut notify_off_multiplier = 0;
        let mut isr_cfg = None;
        let mut device_cfg = None;
        for capability in device.capabilities(config_space) {
            if let PciCapability::Vendor(address) = capability {
                // we would need the extension, aka the private_header,
                // but capability doesn't expose this
                let capability_header = unsafe {
                    config_space.read(address.address, address.offset)
                };
                let private_header = (capability_header >> 16) as u16;

                let cap_len = private_header as u8;
                let cfg_type = (private_header >> 8) as u8;
                if cap_len < 16 {
                    continue;
                }

                /// The offset of the bar field within `virtio_pci_cap`.
                const CAP_BAR_OFFSET: u16 = 4;
                /// The offset of the offset field with `virtio_pci_cap`.
                const CAP_BAR_OFFSET_OFFSET: u16 = 8;
                /// The offset of the `length` field within `virtio_pci_cap`.
                const CAP_LENGTH_OFFSET: u16 = 12;
                /// The offset of the`notify_off_multiplier` field within `virtio_pci_notify_cap`.
                const CAP_NOTIFY_OFF_MULTIPLIER_OFFSET: u16 = 16;

                let struct_info = VirtioCapabilityInfo {
                    bar: unsafe {
                        config_space.read(
                            address.address,
                            address.offset + CAP_BAR_OFFSET,
                        )
                    } as u8,
                    offset: unsafe {
                        config_space.read(
                            address.address,
                            address.offset + CAP_BAR_OFFSET_OFFSET,
                        )
                    },
                    length: unsafe {
                        config_space.read(
                            address.address,
                            address.offset + CAP_LENGTH_OFFSET,
                        )
                    },
                };
                match cfg_type {
                    VIRTIO_PCI_CAP_COMMON_CFG if common_cfg.is_none() => {
                        common_cfg = Some(struct_info);
                    }
                    VIRTIO_PCI_CAP_NOTIFY_CFG if cap_len >= 20 && notify_cfg.is_none() => {
                        notify_cfg = Some(struct_info);
                        notify_off_multiplier = unsafe {
                            config_space.read(
                                address.address,
                                address.offset + CAP_NOTIFY_OFF_MULTIPLIER_OFFSET,
                            )
                        };
                    }
                    VIRTIO_PCI_CAP_ISR_CFG if isr_cfg.is_none() => {
                        isr_cfg = Some(struct_info);
                    }
                    VIRTIO_PCI_CAP_DEVICE_CFG if device_cfg.is_none() => {
                        device_cfg = Some(struct_info);
                    }
                    _ => {}
                }
            }
        }
        let common_cfg_bar = device
            .bar(
                common_cfg.ok_or(VirtioPciError::MissingCommonConfig)?.bar,
                config_space,
            )
            .ok_or(VirtioPciError::BarOffsetOutOfRange)?;
        if notify_off_multiplier % 2 != 0 {
            return Err(VirtioPciError::InvalidNotifyOffMultiplier(
                notify_off_multiplier,
            ));
        }
        let notify_region = device
            .bar(
                notify_cfg.ok_or(VirtioPciError::MissingNotifyConfig)?.bar,
                config_space,
            )
            .ok_or(VirtioPciError::MissingNotifyConfig)?;
        let isr_status = device
            .bar(
                isr_cfg.ok_or(VirtioPciError::MissingIsrConfig)?.bar,
                config_space,
            )
            .ok_or(VirtioPciError::MissingIsrConfig)?;
        let virtio_config_space = match device_cfg {
            Some(cfg) => Some(device.bar(cfg.bar, config_space)
                .ok_or(VirtioPciError::BarOffsetOutOfRange)?),
            None => None,
        };

        Ok(Self {
            device_type,
            common_cfg: common_cfg_bar,
            notify_region,
            notify_off_multiplier,
            isr_status,
            config_space: virtio_config_space,
        })
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

// taken from virtio_drivers::transport::pci
/// Information about a VirtIO structure within some BAR, as provided by a `virtio_pci_cap`.
#[derive(Clone, Debug, Eq, PartialEq)]
struct VirtioCapabilityInfo {
    /// The bar in which the structure can be found.
    pub bar: u8,
    /// The offset within the bar.
    pub offset: u32,
    /// The length in bytes of the structure within the bar.
    pub length: u32,
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
