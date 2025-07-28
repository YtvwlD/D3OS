use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Deref;
use core::ptr;
use log::info;
use smoltcp::iface::{self, Interface, SocketHandle, SocketSet};
use smoltcp::socket::{dhcpv4, icmp, tcp, udp, Socket};
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};
use spin::{Once, RwLock};
use crate::device::rtl8139::Rtl8139;
use crate::{pci_bus, scheduler, timer};
use crate::process::thread::Thread;

static RTL8139: Once<Arc<Rtl8139>> = Once::new();

static INTERFACES: RwLock<Vec<Interface>> = RwLock::new(Vec::new());
static SOCKETS: Once<RwLock<SocketSet>> = Once::new();

#[derive(Debug)]
#[repr(u8)]
#[non_exhaustive]
pub enum SocketType {
    Udp, Tcp, Icmp,
}

pub fn init() {
    SOCKETS.call_once(|| RwLock::new(SocketSet::new(Vec::new())));

    let devices = pci_bus().search_by_ids(0x10ec, 0x8139);
    if !devices.is_empty() {
        RTL8139.call_once(|| {
            info!("Found Realtek RTL8139 network controller");
            let rtl8139 = Arc::new(Rtl8139::new(devices[0]));
            info!("RTL8139 MAC address: [{}]", rtl8139.read_mac_address());

            Rtl8139::plugin(Arc::clone(&rtl8139));
            rtl8139
        });
    }

    if let Some(rtl8139) = RTL8139.get() {
        extern "sysv64" fn poll() {
            loop { poll_sockets(); }
        }
        scheduler().ready(Thread::new_kernel_thread(poll, "RTL8139"));
        
        // Set up network interface
        let time = timer().systime_ms();
        let mut conf = iface::Config::new(HardwareAddress::from(rtl8139.read_mac_address()));
        conf.random_seed = time as u64;

        // The Smoltcp interface struct wants a mutable reference to the device.
        // However, the RTL8139 driver is designed to work with shared references.
        // Since smoltcp does not actually store the mutable reference anywhere,
        // we can safely cast the shared reference to a mutable one.
        // (Actually, I am not sure why the smoltcp interface wants a mutable reference to the device,
        // since it does not modify the device itself.)
        let device = unsafe { ptr::from_ref(rtl8139.deref()).cast_mut().as_mut().unwrap() };
        add_interface(Interface::new(conf, device, Instant::from_millis(time as i64)));

        // request an IP address via DHCP
        let dhcp_socket = dhcpv4::Socket::new();
        SOCKETS
            .get()
            .expect("Socket set not initialized!")
            .write()
            .add(dhcp_socket);
    }
}

fn add_interface(interface: Interface) {
    INTERFACES.write().push(interface);
}

pub fn get_ip_addresses() -> Vec<IpAddress> {
    INTERFACES
        .read()
        .iter()
        .map(Interface::ip_addrs)
        .flatten()
        .map(IpCidr::address)
        .collect()
}

pub fn open_udp() -> SocketHandle {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");

    let rx_buffer = udp::PacketBuffer::new(
        vec![udp::PacketMetadata::EMPTY, udp::PacketMetadata::EMPTY],
        vec![0; 65535],
    );
    let tx_buffer = udp::PacketBuffer::new(
        vec![udp::PacketMetadata::EMPTY, udp::PacketMetadata::EMPTY],
        vec![0; 65535],
    );

    sockets.write().add(udp::Socket::new(rx_buffer, tx_buffer))
}

pub fn open_tcp() -> SocketHandle {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");
    let rx_buffer = tcp::SocketBuffer::new(vec![0; 65535]);
    let tx_buffer = tcp::SocketBuffer::new(vec![0; 65535]);

    sockets.write().add(tcp::Socket::new(rx_buffer, tx_buffer))
}

pub fn open_icmp() -> SocketHandle {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");
    
    let rx_buffer = icmp::PacketBuffer::new(
        vec![icmp::PacketMetadata::EMPTY, icmp::PacketMetadata::EMPTY],
        vec![0; 65535],
    );
    let tx_buffer = icmp::PacketBuffer::new(
        vec![icmp::PacketMetadata::EMPTY, icmp::PacketMetadata::EMPTY],
        vec![0; 65535],
    );

    sockets.write().add(icmp::Socket::new(rx_buffer, tx_buffer))
}

pub fn close_socket(handle: SocketHandle) {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");
    sockets.write().remove(handle);
}

pub fn bind_udp(handle: SocketHandle, port: u16) -> Result<(), udp::BindError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<udp::Socket>(handle);

    socket.bind(port)
}

pub fn bind_tcp(handle: SocketHandle, port: u16) -> Result<(), tcp::ListenError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<tcp::Socket>(handle);

    socket.listen(port)
}

pub fn bind_icmp(handle: SocketHandle, ident: u16) -> Result<(), icmp::BindError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<icmp::Socket>(handle);

    socket.bind(icmp::Endpoint::Ident(ident))
}

pub fn accept_tcp(handle: SocketHandle) -> Result<u16, tcp::ConnectError> {
    // TODO: smoltcp knows no backlog
    // all but the first connection will fail
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<tcp::Socket>(handle);

    while !socket.is_active() {
        scheduler().sleep(100);
    }
    // TODO: pass the remote addr
    Ok(socket.remote_endpoint().unwrap().port)
}

pub fn connect_tcp(handle: SocketHandle, host: IpAddress, port: u16) -> Result<u16, tcp::ConnectError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let mut interfaces = INTERFACES.write();

    let socket = sockets.get_mut::<tcp::Socket>(handle);

    let interface = interfaces.get_mut(0).ok_or(tcp::ConnectError::InvalidState)?;
    let local_port = 1797; // TODO

    socket.connect(interface.context(), (host, port), local_port)?;
    // TODO: pass the local addr
    Ok(socket.local_endpoint().unwrap().port)
}

pub fn send_datagram(handle: SocketHandle, destination: IpAddress, port: u16, data: &[u8]) -> Result<(), udp::SendError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<udp::Socket>(handle);

    socket.send_slice(data, (destination, port))
}

pub fn send_tcp(handle: SocketHandle, data: &[u8]) -> Result<usize, tcp::SendError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<tcp::Socket>(handle);

    socket.send_slice(data)
}

pub fn send_icmp(handle: SocketHandle, destination: IpAddress, data: &[u8]) -> Result<(), icmp::SendError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<icmp::Socket>(handle);

    socket.send_slice(data, destination)
}

pub fn receive_datagram(handle: SocketHandle, data: &mut [u8]) -> Result<(usize, udp::UdpMetadata), udp::RecvError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<udp::Socket>(handle);

    socket.recv_slice(data)
}

pub fn receive_tcp(handle: SocketHandle, data: &mut [u8]) -> Result<usize, tcp::RecvError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<tcp::Socket>(handle);

    socket.recv_slice(data)
}

pub fn receive_icmp(handle: SocketHandle, data: &mut [u8]) -> Result<(usize, IpAddress), icmp::RecvError> {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let socket = sockets.get_mut::<icmp::Socket>(handle);

    socket.recv_slice(data)
}

fn poll_sockets() {
    let rtl8139 = RTL8139.get().expect("RTL8139 not initialized");
    let mut interfaces = INTERFACES.write();
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let time = Instant::from_millis(timer().systime_ms() as i64);

    // Smoltcp expects a mutable reference to the device, but the RTL8139 driver is built
    // to work with a shared reference. We can safely cast the shared reference to a mutable.
    let device = unsafe { ptr::from_ref(rtl8139.deref()).cast_mut().as_mut().unwrap() };

    // DHCP handling is based on https://github.com/smoltcp-rs/smoltcp/blob/main/examples/dhcp_client.rs
    for interface in interfaces.iter_mut() {
        interface.poll(time, device, &mut sockets);
        for (_handle, socket) in sockets.iter_mut() {
            if let Socket::Dhcpv4(dhcp) = socket {
                if let Some(event) = dhcp.poll() {
                    match event {
                        dhcpv4::Event::Deconfigured => {
                            info!("lost DHCP lease");
                            interface.update_ip_addrs(|addrs| addrs.clear());
                            interface.routes_mut().remove_default_ipv4_route();
                        },
                        dhcpv4::Event::Configured(config) => {
                            info!("acquired DHCP lease:");
                            info!("IP address: {}", config.address);
                            interface.update_ip_addrs(|addrs| {
                                addrs.clear();
                                addrs.push(IpCidr::Ipv4(config.address)).unwrap();
                            });

                            if let Some(router) = config.router {
                                info!("default gateway: {}", router);
                                interface
                                    .routes_mut()
                                    .add_default_ipv4_route(router)
                                    .unwrap();
                            } else {
                                info!("no default gateway");
                                interface
                                    .routes_mut()
                                    .remove_default_ipv4_route();
                            }
                            // TODO: make use of this
                            info!("DNS servers: {:?}", config.dns_servers);
                        },
                    }
                }
            }
        }
    }
}