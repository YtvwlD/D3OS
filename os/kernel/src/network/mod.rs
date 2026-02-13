use alloc::collections::btree_map::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use smoltcp::phy::Loopback;
use smoltcp::socket::dns::GetQueryResultError;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::ops::{Deref, DerefMut};
use core::ptr;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use log::{info, warn};
use smoltcp::iface::{self, Interface, SocketHandle, SocketSet};
use smoltcp::socket;
use smoltcp::socket::{dhcpv4, dns, icmp, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{DnsQueryType, HardwareAddress, IpAddress, IpCidr, IpEndpoint};
use spin::{Once, RwLock};
use crate::device::rtl8139::Rtl8139;
use crate::process::process::Process;
use crate::{pci_bus, process_manager, scheduler, timer};
use crate::process::thread::Thread;

static INTERFACES: RwLock<BTreeMap<String, Arc<NetworkInterface>>> = RwLock::new(BTreeMap::new());
static ETHERNET_COUNT: AtomicU8 = AtomicU8::new(0);

/// This maps processes to their sockets.
/// process ID -> [`PerProcessSocketSet`]
static SOCKET_PROCESS: RwLock<BTreeMap<usize, RwLock<PerProcessSocketSet>>> = RwLock::new(BTreeMap::new());
static DNS_SOCKET: Once<SocketId> = Once::new();
static DHCP_SOCKET: Once<SocketId> = Once::new();

#[derive(PartialEq, Eq, PartialOrd, Ord, Debug, Copy, Clone)]
struct SocketId(u64);

/// This struct represents a single network interface.
/// 
/// It contains the actual device, the smoltcp interface and the sockets
/// associated with this device.
struct NetworkInterface {
    device: NetworkDevice,
    interface: RwLock<Interface>,
    sockets: RwLock<SocketSet<'static>>,
}

impl NetworkInterface {
    fn new(device: NetworkDevice, interface: Interface) -> Self {
        let sockets = RwLock::new(SocketSet::new(Vec::new()));
        Self { device, interface: RwLock::new(interface), sockets }
    }
    
    fn poll(&self) -> iface::PollResult {
        let time = Instant::from_millis(timer().systime_ms() as i64);
        // try to get both locks, else we might get a deadlock
        if let Some(mut interface) = self.interface.try_write()
            && let Some(mut sockets) = self.sockets.try_write() {
            self.device.poll(interface.deref_mut(), sockets.deref_mut(), time)
        } else {
            iface::PollResult::None
        }
    }
}

impl alloc::fmt::Debug for NetworkInterface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f
            .debug_struct("NetworkInterface")
            .field("device", &self.device)
            .field("sockets", &self.sockets.read().iter().count()).finish()
    }
}

enum NetworkDevice {
    Rtl8139(Arc<Rtl8139>),
    Loopback(Loopback),
}

impl NetworkDevice {
    fn poll(&self, interface: &mut Interface, sockets: &mut SocketSet<'static>, time: Instant) -> iface::PollResult {
        match self {
            NetworkDevice::Rtl8139(device) => {
                // The Smoltcp interface struct wants a mutable reference to the device.
                // However, the RTL8139 driver is designed to work with shared references.
                // Since smoltcp does not actually store the mutable reference anywhere,
                // we can safely cast the shared reference to a mutable one.
                // (Actually, I am not sure why the smoltcp interface wants a mutable reference to the device,
                // since it does not modify the device itself.)
                let device = unsafe { ptr::from_ref(device.deref().deref()).cast_mut().as_mut().unwrap() };
                
                interface.poll(time, device, sockets)
            },
            NetworkDevice::Loopback(loopback) => todo!(),
        }
    }
}

impl alloc::fmt::Debug for NetworkDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rtl8139(_) => f.debug_tuple("Rtl8139").finish(),
            Self::Loopback(_) => f.debug_tuple("Loopback").finish(),
        }
    }
}

/// This contains the sockets of a process.
/// 
/// The counter generates increasing IDs.
/// Each socket ID corresponds to one or more smoltcp SocketHandles.
#[derive(Debug, Default)]
struct PerProcessSocketSet {
    counter: AtomicU64,
    sockets: BTreeMap<SocketId, Vec<(Arc<NetworkInterface>, SocketHandle)>>,
}

impl PerProcessSocketSet {
    /// Insert these interface-handle-pairs, returning a new ID.
    fn insert(&mut self, interface_handle: Vec<(Arc<NetworkInterface>, SocketHandle)>) -> SocketId {
        let id = SocketId(self.counter.fetch_add(1, Ordering::SeqCst));
        self.sockets.try_insert(id, interface_handle)
            .expect("failed to insert socket in per-process socket set");
        id
    }
}

#[derive(Debug)]
#[repr(u8)]
#[non_exhaustive]
pub enum SocketType {
    Udp, Tcp, Icmp,
}

pub fn init() {
    for device in pci_bus().search_by_ids(0x10ec, 0x8139) {
        info!("Found Realtek RTL8139 network controller");
        let rtl8139 = Arc::new(Rtl8139::new(device));
        info!("RTL8139 MAC address: [{}]", rtl8139.read_mac_address());
        Rtl8139::plugin(Arc::clone(&rtl8139));
        
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
        let interface = Interface::new(conf, device, Instant::from_millis(time as i64));        
        
        let index = ETHERNET_COUNT.fetch_add(1, Ordering::SeqCst);
        INTERFACES.write().try_insert(format!("eth{index}"), Arc::new(NetworkInterface::new(
            NetworkDevice::Rtl8139(rtl8139), interface,
        ))).expect("network interface does already exist");
    }

    // Add DHCP and DNS on the first ethernet interface if we have one.
    if let Some((_name, interface)) = INTERFACES
        .read()
        .iter()
        .filter(|(_name, interface)| !matches!(interface.device, NetworkDevice::Loopback(_)))
        .next() {
        let pid = process_manager().read().current_process().id;
        let mut socket_process = SOCKET_PROCESS
            .write();
        let mut process_map = socket_process
            .try_insert(pid, RwLock::default())
            .expect("failed to create socket set for kernel process")
            .write();
        let mut sockets = interface.sockets.write();
        // setup DNS
        DNS_SOCKET.call_once(|| {
            let dns_socket = dns::Socket::new(&[], Vec::new());
            let dns_handle = sockets.add(dns_socket);
            process_map
                .insert(vec![(interface.clone(), dns_handle)])
        });
        // request an IP address via DHCP
        DHCP_SOCKET.call_once(|| {
            let dhcp_socket = dhcpv4::Socket::new();
            let dhcp_handle = sockets.add(dhcp_socket);
            process_map
                .insert(vec![(interface.clone(), dhcp_handle)])
        });
    }
    
    extern "sysv64" fn poll() {
        loop { poll_sockets(); scheduler().switch_thread_no_interrupt(); }
    }
    scheduler().ready(Thread::new_kernel_thread(poll, "network"));
}

fn check_ownership(handle: SocketHandle) {
    // TODO: these panics should probably kill the process that made the call, not the kernel
    let lock = SOCKET_PROCESS.read();
    let owning_process = lock
        .get(&handle)
        .expect("process tried accessing non-existent socket");
    if *owning_process != process_manager().read().current_process() {
        panic!("process tried to access socket of a different process");
    }
}

// for lifetime-reasons this must be a macro
macro_rules! get_socket_for_current_process {
    ($socket:ident, $handle:ident, $type:ty) => {
        check_ownership($handle);
        let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
        let $socket = sockets.get_mut::<$type>($handle);
    }
}

/// Get IP addresses for a host.
/// 
/// If host is none, get the addresses of the current host.
pub fn get_ip_addresses(host: Option<&str>) -> Vec<IpAddress> {
    if let Some(host) = host {
        let handle = DNS_SOCKET.get().expect("DNS socket does not exist yet");
        // first, start the queries
        let mut query_handles: Vec<_> = {
            let socket_process = SOCKET_PROCESS.read();
            let kernel_sockets = socket_process.get(&0)
                .expect("failed to get sockets of the kernel process")
                .read();
            let (interface_name, handle) = kernel_sockets.sockets
                .get(DNS_SOCKET.get().expect("failed to get DNS socket ID"))
                .expect("failed to get DNS sockets")
                .iter().next()
                .expect("failed to get DNS socket");
            let interfaces = INTERFACES.read();
            let mut interface = interfaces
                .get(interface_name)
                .expect("failed to get DNS interface")
                .write();
            let socket = interface.sockets.get_mut::<dns::Socket>(*handle);
            [DnsQueryType::Aaaa, DnsQueryType::A, DnsQueryType::Cname]
                .into_iter()
                .filter_map(|ty|
                        socket
                            .start_query(interface.interface.context(), host, ty)
                            .map_err(|e| {
                                warn!("DNS query for {host} {ty:?} failed: {e:?}");
                                e
                            })
                            .ok()
                )
                .collect()
        };
        // then, see if they've returned something
        let mut resulting_ips = Vec::new();
        loop {
            {
                let socket_process = SOCKET_PROCESS.read();
                let kernel_sockets = socket_process.get(&0)
                    .expect("failed to get sockets of the kernel process")
                    .read();
                let (interface_name, handle) = kernel_sockets.sockets
                    .get(DNS_SOCKET.get().expect("failed to get DNS socket ID"))
                    .expect("failed to get DNS sockets")
                    .iter().next()
                    .expect("failed to get DNS socket");
                let interfaces = INTERFACES.read();
                let mut interface = interfaces
                    .get(interface_name)
                    .expect("failed to get DNS interface")
                    .write();
                let socket = interface.sockets.get_mut::<dns::Socket>(*handle);
                let mut remaining: Vec<_> = query_handles
                    .drain(..)
                    .filter(|query| match socket.get_query_result(*query) {
                        // it's finished, get the results
                        Ok(ips) => {
                            // TODO: does a cname query really return an IP?
                            resulting_ips.extend_from_slice(&ips);
                            false
                        },
                        // if failed, log and and ignore
                        Err(GetQueryResultError::Failed) => {
                            warn!("DNS query for {host} failed");
                            false
                        },
                        // it's still ongoing
                        Err(GetQueryResultError::Pending) => true,
                    })
                    .collect();
                if remaining.is_empty() {
                    // we're done!
                    break;
                }
                // else, check for the remaining ones
                query_handles.clear();
                query_handles.append(&mut remaining);
            }
            // release the locks and sleep
            scheduler().sleep(50);
        }
        resulting_ips
    } else {
        let mut ip_addrs = Vec::new();
        for (_name, interface) in INTERFACES.read().iter() {
            let lock = interface.read();
            ip_addrs.extend(
                lock.interface.ip_addrs().iter().map(IpCidr::address)
            );
        }
        ip_addrs
    }
}

pub fn open_udp() -> SocketHandle {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");

    let rx_buffer = udp::PacketBuffer::new(
        vec![udp::PacketMetadata::EMPTY; 44],
        vec![0; 65535],
    );
    let tx_buffer = udp::PacketBuffer::new(
        vec![udp::PacketMetadata::EMPTY; 44],
        vec![0; 65535],
    );

    let handle = sockets.write().add(udp::Socket::new(rx_buffer, tx_buffer));
    SOCKET_PROCESS
        .write()
        .try_insert(handle, process_manager().read().current_process())
        .expect("failed to insert socket into socket-process map");
    handle
}

pub fn open_tcp() -> SocketHandle {
    let sockets = SOCKETS.get().expect("Socket set not initialized!");
    let rx_buffer = tcp::SocketBuffer::new(vec![0; 65535]);
    let tx_buffer = tcp::SocketBuffer::new(vec![0; 65535]);

    let handle = sockets.write().add(tcp::Socket::new(rx_buffer, tx_buffer));
    SOCKET_PROCESS
        .write()
        .try_insert(handle, process_manager().read().current_process())
        .expect("failed to insert socket into socket-process map");
    handle
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

    let handle = sockets.write().add(icmp::Socket::new(rx_buffer, tx_buffer));
    SOCKET_PROCESS
        .write()
        .try_insert(handle, process_manager().read().current_process())
        .expect("failed to insert socket into socket-process map");
    handle
}

pub fn close_socket(handle: SocketHandle) {
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();

    check_ownership(handle);

    let socket_ref = sockets.iter_mut()
        .find(|(h, _)| *h == handle)
        .map(|(_, s)| s);

    if let Some(socket) = socket_ref {
        match socket {
            socket::Socket::Tcp(s) => s.close(),
            socket::Socket::Udp(s) => s.close(),
            _ => {},
        }
    } else {
        warn!("Socket {} not found in SocketSet", handle);
    }

    // Remove permission for the process
    // The socket remains in the set until poll_sockets() garbage collects it.
    SOCKET_PROCESS.write().remove(&handle).unwrap();
}

pub fn bind_udp(handle: SocketHandle, addr: IpAddress, port: u16) -> Result<(), udp::BindError> {
    get_socket_for_current_process!(socket, handle, udp::Socket);
    let port = pick_port(port);
    match addr {
        // binding to 0.0.0.0 or :: means listening to all requests
        // but smoltcp doesn't understand it that way
        IpAddress::Ipv4(Ipv4Addr::UNSPECIFIED) | IpAddress::Ipv6(Ipv6Addr::UNSPECIFIED) => socket.bind(port),
        // else, bind to the specified address
        _ => socket.bind((addr, port)),
    }
}

pub fn bind_tcp(handle: SocketHandle, addr: IpAddress, port: u16) -> Result<(), tcp::ListenError> {
    get_socket_for_current_process!(socket, handle, tcp::Socket);
    let port = pick_port(port);
    match addr {
        // binding to 0.0.0.0 or :: means listening to all requests
        // but smoltcp doesn't understand it that way
        IpAddress::Ipv4(Ipv4Addr::UNSPECIFIED) | IpAddress::Ipv6(Ipv6Addr::UNSPECIFIED) => socket.listen(port),
        // else, bind to the specified address
        _ => socket.listen((addr, port)),
    }
}

pub fn bind_icmp(handle: SocketHandle, ident: u16) -> Result<(), icmp::BindError> {
    get_socket_for_current_process!(socket, handle, icmp::Socket);
    socket.bind(icmp::Endpoint::Ident(ident))
}

/// Accept a new connection from a TCP socket.
/// 
/// This returns the client that opened the new connection and a **new listening socket**.
pub fn accept_tcp(handle: SocketHandle) -> Result<(IpEndpoint, SocketHandle), tcp::ConnectError> {
    let (client, listen) = loop {
        // this extra block is needed so that we don't block all sockets
        {
            get_socket_for_current_process!(socket, handle, tcp::Socket);
            if socket.is_active() {
                break (
                    socket.remote_endpoint().expect("failed to get remote endpoint"),
                    socket.listen_endpoint(),
                );
            }
        }
        scheduler().sleep(100);
    };
    // now we have a socket that is connected
    // but we need to have to create a new one to be able to accept additional connections
    let listen_handle = open_tcp();
    bind_tcp(listen_handle, listen.addr.unwrap_or(
        IpAddress::Ipv6(Ipv6Addr::UNSPECIFIED)
    ), listen.port).expect("failed to create new listening socket");
    Ok((client, listen_handle))
}

pub fn connect_tcp(handle: SocketHandle, host: IpAddress, port: u16) -> Result<IpEndpoint, tcp::ConnectError> {    get_socket_for_current_process!(socket, handle, tcp::Socket);
    let mut interfaces = IFACES.write();
    let interface = interfaces.get_mut(0).ok_or(tcp::ConnectError::InvalidState)?;
    let local_port = pick_port(0);

    socket.connect(interface.context(), (host, port), local_port)?;
    Ok(socket.local_endpoint().unwrap())
}

pub fn send_datagram(handle: SocketHandle, destination: IpAddress, port: u16, data: &[u8]) -> Result<(), udp::SendError> {
    get_socket_for_current_process!(socket, handle, udp::Socket);
    socket.send_slice(data, (destination, port))
}

pub fn send_tcp(handle: SocketHandle, data: &[u8]) -> Result<usize, tcp::SendError> {
    loop {
        // this extra block is needed so that we don't block all sockets
        {
            get_socket_for_current_process!(socket, handle, tcp::Socket);
            if socket.can_send() {
                break;
            }
        }
        scheduler().sleep(100);
    }
    get_socket_for_current_process!(socket, handle, tcp::Socket);
    socket.send_slice(data)
}

pub fn send_icmp(handle: SocketHandle, destination: IpAddress, data: &[u8]) -> Result<(), icmp::SendError> {
    get_socket_for_current_process!(socket, handle, icmp::Socket);
    socket.send_slice(data, destination)
}

pub fn receive_datagram(handle: SocketHandle, data: &mut [u8]) -> Result<(usize, udp::UdpMetadata), udp::RecvError> {
    get_socket_for_current_process!(socket, handle, udp::Socket);
    socket.recv_slice(data)
}

pub fn receive_tcp(handle: SocketHandle, data: &mut [u8]) -> Result<usize, tcp::RecvError> {
    loop {
        // this extra block is needed so that we don't block all sockets
        {
            get_socket_for_current_process!(socket, handle, tcp::Socket);
            if socket.can_recv() {
                break;
            }
        }
        scheduler().sleep(100);
    }
    get_socket_for_current_process!(socket, handle, tcp::Socket);
    socket.recv_slice(data)
}

pub fn receive_icmp(handle: SocketHandle, data: &mut [u8]) -> Result<(usize, IpAddress), icmp::RecvError> {
    get_socket_for_current_process!(socket, handle, icmp::Socket);
    socket.recv_slice(data)
}

pub fn can_recv(handle: SocketHandle, protocol: SocketType) -> bool {
    match protocol {
        SocketType::Udp => {
            get_socket_for_current_process!(socket, handle, udp::Socket);
            socket.can_recv()
        },
        SocketType::Tcp => {
            get_socket_for_current_process!(socket, handle, tcp::Socket);
            socket.can_recv()
        }
        SocketType::Icmp => {
            get_socket_for_current_process!(socket, handle, icmp::Socket);
            socket.can_recv()
        }
    }
}

pub fn can_send(handle: SocketHandle, protocol: SocketType) -> bool {
    match protocol {
        SocketType::Udp => {
            get_socket_for_current_process!(socket, handle, udp::Socket);
            socket.can_send()
        },
        SocketType::Tcp => {
            get_socket_for_current_process!(socket, handle, tcp::Socket);
            socket.can_send()
        }
        SocketType::Icmp => {
            get_socket_for_current_process!(socket, handle, icmp::Socket);
            socket.can_send()
        }
    }
}

/// Try to poll all sockets.
fn poll_sockets() -> () {
    for interface in INTERFACES.read().values() {
        let mut poll_budget = 16;
        while poll_budget > 0 {
            match interface.poll() {
                iface::PollResult::None => break,
                iface::PollResult::SocketStateChanged => {
                    poll_budget -= 1;
                },
            }
        }
    }
    
    // DHCP handling is based on https://github.com/smoltcp-rs/smoltcp/blob/main/examples/dhcp_client.rs
    // This code assumes that DHCP and DNS are on the same single interface.
    let dhcp_id = DHCP_SOCKET.get().expect("DHCP socket does not exist yet");
    let pid = process_manager().read().current_process().id;
    let socket_process = SOCKET_PROCESS
        .read();
    let process_sockets = socket_process
        .get(&pid)
        .expect("failed to get socket set for kernel process")
        .write();
    let interfaces = INTERFACES
        .read();
    let (dhcp_interface, dhcp_handle) = process_sockets.sockets
        .get(dhcp_id)
        .expect("failed to get DHCP sockets")
        .iter().next()
        .expect("failed to get DHCP socket");
    let mut sockets_write = dhcp_interface.sockets.write();
    let dhcp_socket = sockets_write.get_mut::<dhcpv4::Socket>(*dhcp_handle);
    let mut interface_write = dhcp_interface.interface.write();
    if let Some(event) = dhcp_socket.poll() {
        match event {
            dhcpv4::Event::Deconfigured => {
                info!("lost DHCP lease");
                interface_write.update_ip_addrs(|addrs| addrs.clear());
                interface_write.routes_mut().remove_default_ipv4_route();
            },
            dhcpv4::Event::Configured(config) => {
                info!("acquired DHCP lease:");
                info!("IP address: {}", config.address);
                interface_write.update_ip_addrs(|addrs| {
                    addrs.clear();
                    addrs.push(IpCidr::Ipv4(config.address)).unwrap();
                });

                if let Some(router) = config.router {
                    info!("default gateway: {router}");
                    interface_write
                        .routes_mut()
                        .add_default_ipv4_route(router)
                        .unwrap();
                } else {
                    info!("no default gateway");
                    interface_write
                        .routes_mut()
                        .remove_default_ipv4_route();
                }
                info!("DNS servers: {:?}", config.dns_servers);
                let dns_servers: Vec<_> = config.dns_servers
                    .iter()
                    .map(|ip| IpAddress::Ipv4(*ip))
                    .collect();
                let dns_socket_id = DNS_SOCKET.get().expect("DNS socket does not exist yet");
                let (_, dns_handle) = process_sockets.sockets
                    .get(dns_socket_id)
                    .expect("failed to get DNS sockets")
                    .iter().next()
                    .expect("failed to get DNS socket");
                let dns_socket = sockets_write.get_mut::<dns::Socket>(*dns_handle);
                dns_socket.update_servers(&dns_servers);
            },
        }
    }

    // Remove closed sockets
    let mut sockets_to_remove = Vec::new();

    let socket_map = SOCKET_PROCESS.read();
    let dns_handle = DNS_SOCKET.get().expect("DNS socket does not exist yet");

    for (handle, socket) in sockets.iter() {
        // Skip system sockets
        if handle == *dhcp_handle || handle == *dns_handle {
            continue;
        }

        let is_closed = match socket {
            // Only remove TCP sockets that have fully traversed the state machine to the CLOSED state.
            socket::Socket::Tcp(s) => s.state() == tcp::State::Closed,
            // UDP sockets are stateless so whe can remove them immediately
            socket::Socket::Udp(s) => true,
            _ => false,
        };

        // only delete the socket if the process has released the handle
        if is_closed && !socket_map.contains_key(&handle) {
            sockets_to_remove.push(handle);
        }
    }

    for handle in sockets_to_remove {
        info!("Garbage collecting closed socket: {}", handle);
        sockets.remove(handle);
    }
}

pub(crate) fn close_sockets_for_process(process: &mut Process) {
    let mut lock = SOCKET_PROCESS.write();
    let mut sockets = SOCKETS.get().expect("Socket set not initialized!").write();
    let handles: Vec<_> = lock
        .iter()
        .filter(|(_handle, proc)| ***proc == *process)
        .map(|(handle, _proc)| handle)
        .copied()
        .collect();
    for handle in handles {
        lock.remove(&handle).unwrap();
        sockets.remove(handle);
    }
}

/// Pick a random port if port == 0, else just use the passed port.
fn pick_port(port: u16) -> u16 {
    if port == 0 {
        // TODO: make sure that this isn't used yet
        timer().systime_ms() as u16
    } else {
        port
    }
}
