//! Keeping Google connections off a TUN proxy.
//!
//! A TUN-mode proxy or VPN (sing-box, Clash, WireGuard…) takes the default
//! route, so even a connection to a literal Google IP enters the tunnel, gets
//! its SNI sniffed and is sent wherever the proxy's rules say. Like
//! sing-box's own `auto_detect_interface`, gdirect binds the sockets that
//! carry Drive traffic to scanned endpoints to the physical interface the
//! TUN would otherwise capture, so they leave the machine directly:
//!
//! - macOS: `IP_BOUND_IF` / `IPV6_BOUND_IF`, interface from the routing table
//!   (the default routes, scoped ones included, minus utun/ipsec/… ones);
//! - Linux: `SO_BINDTOIFINDEX` / `SO_BINDTODEVICE` (needs `CAP_NET_RAW`;
//!   without it gdirect stays on the system route and says so once),
//!   interface from the main table's default route in `/proc/net/route`;
//! - Windows: `IP_UNICAST_IF` / `IPV6_UNICAST_IF`, interface from the IPv4
//!   routing table minus TUN/VPN adapters.
//!
//! Binding happens whenever bypassing is on and a physical interface is
//! found, TUN or not: without a TUN the physical interface carries the
//! default route anyway, so the bind changes nothing, and gdirect needs no
//! fragile guess about whether some virtual interface is a proxy. Sign-in and
//! the system-network fallbacks are never bound; they follow the proxy.
use crate::store::Store;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

/// Settings key: "1" (default) binds to the physical interface, "0" lets the
/// proxy's rules decide.
pub const SETTING: &str = "network.bypass_tun";
/// Settings key: the network path the saved endpoints were verified over,
/// see [`Status::path`].
pub const SCANNED_VIA: &str = "network.scanned_via";
/// Overrides the saved setting for one process (and is how a background
/// mount helper inherits it).
pub const ENV: &str = "GDIRECT_BYPASS_TUN";
/// How long a detected interface is trusted before the routing table is read
/// again. A failed connect re-reads it at once, see [`forget`].
const TTL: Duration = Duration::from_secs(30);

/// A network interface by name and index, with an address of each family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: String,
    pub index: u32,
    pub v4: Option<Ipv4Addr>,
    pub v6: Option<Ipv6Addr>,
}

/// A default route as the system reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultRoute {
    pub interface: String,
    pub index: u32,
    /// Through a gateway rather than straight onto the link (a TUN's own
    /// default route usually has none).
    pub gateway: bool,
    /// Bound to its interface (macOS `RTF_IFSCOPE`): used only by sockets
    /// bound there, so never the default for everyone else.
    pub scoped: bool,
    pub metric: u32,
    /// Known virtual from more than its name (Windows adapter type or
    /// description).
    pub virtual_adapter: bool,
}
impl DefaultRoute {
    pub fn is_virtual(&self) -> bool {
        self.virtual_adapter || virtual_name(&self.interface)
    }
}

/// Interface names that belong to tunnels, VPNs, bridges and other software
/// interfaces rather than to a network adapter.
pub fn virtual_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    const PREFIXES: [&str; 29] = [
        "utun",
        "tun",
        "tap",
        "ipsec",
        "ppp",
        "gif",
        "stf",
        "bridge",
        "awdl",
        "llw",
        "anpi",
        "lo",
        "wg",
        "tailscale",
        "zt",
        "docker",
        "br-",
        "veth",
        "virbr",
        "vmnet",
        "vboxnet",
        "meta",
        "mihomo",
        "clash",
        "sing",
        "nordlynx",
        "wintun",
        "ap1",
        "feth",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Windows adapter descriptions or names that give a TUN/VPN adapter away.
pub fn virtual_description(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "tun",
        "wintun",
        "sing-box",
        "clash",
        "meta",
        "mihomo",
        "wireguard",
        "tap-",
        "tap ",
        "openvpn",
        "tailscale",
        "zerotier",
        "vpn",
        "loopback",
    ]
    .iter()
    .any(|word| text.contains(word))
}

/// The routes that matter, out of a system's default routes: `primary`
/// carries everyone's traffic (a TUN, when one is on); `physical` is the
/// adapter to bind to. Unscoped routes come first, then lower metric, then
/// the system's own order.
pub struct Selection<'a> {
    pub primary: Option<&'a DefaultRoute>,
    pub physical: Option<&'a DefaultRoute>,
}
pub fn pick(routes: &[DefaultRoute]) -> Selection<'_> {
    let mut sorted: Vec<&DefaultRoute> = routes.iter().collect();
    sorted.sort_by_key(|r| (r.scoped, r.metric));
    let primary = sorted
        .iter()
        .find(|r| !r.scoped)
        .or(sorted.first())
        .copied();
    let physical = sorted
        .iter()
        .find(|r| !r.is_virtual() && r.gateway)
        .or_else(|| sorted.iter().find(|r| !r.is_virtual()))
        .copied();
    Selection { primary, physical }
}

/// What binding does right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Bypassing is on (the setting, or `GDIRECT_BYPASS_TUN`).
    pub enabled: bool,
    /// The adapter a TUN would capture traffic from.
    pub physical: Option<Interface>,
    /// The virtual interface that carries the default route, if any.
    pub tun: Option<String>,
    /// Why binding to `physical` does not work here (Linux without
    /// CAP_NET_RAW), so connections stay on the system route.
    pub denied: Option<String>,
}
impl Status {
    /// The interface sockets are bound to, if any.
    pub fn bound(&self) -> Option<&Interface> {
        if self.enabled && self.denied.is_none() {
            self.physical.as_ref()
        } else {
            None
        }
    }
    /// The network path connections take: `direct:IFACE` when they leave
    /// through the adapter (bound, or with no TUN in the way), `tun:IFACE`
    /// when a TUN carries them; None when nothing is known (offline).
    /// Endpoints verified over one path say nothing about another.
    pub fn path(&self) -> Option<String> {
        if let Some(bound) = self.bound() {
            return Some(format!("direct:{}", bound.name));
        }
        match (&self.tun, &self.physical) {
            (Some(tun), _) => Some(format!("tun:{tun}")),
            (None, Some(physical)) => Some(format!("direct:{}", physical.name)),
            (None, None) => None,
        }
    }
}

/// Whether endpoints verified over `stored` must be re-verified on `current`.
/// An unknown current path (offline, no routes) keeps them; a database that
/// never recorded its path (before this setting existed) re-verifies once.
pub fn stale(stored: Option<&str>, current: Option<&str>) -> bool {
    current.is_some() && stored != current
}

/// 0: not configured (environment, else on); 1: on; 2: off.
static MODE: AtomicU8 = AtomicU8::new(0);
struct Detected {
    at: Instant,
    physical: Option<Interface>,
    tun: Option<String>,
    denied: Option<String>,
}
static CACHE: Mutex<Option<Detected>> = Mutex::new(None);

fn env_choice() -> Option<bool> {
    let value = std::env::var(ENV).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "false" | "off" | "no" | "n" => Some(false),
        "1" | "true" | "on" | "yes" | "y" => Some(true),
        _ => None,
    }
}
/// Whether bypassing is on for this process.
pub fn enabled() -> bool {
    match MODE.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => env_choice().unwrap_or(true),
    }
}
/// Turns bypassing on or off for this process (the desktop switch).
pub fn set_enabled(on: bool) {
    MODE.store(if on { 1 } else { 2 }, Ordering::Relaxed);
}
/// The saved choice; `GDIRECT_BYPASS_TUN` overrides it.
pub fn saved(db: &Store) -> bool {
    db.setting(SETTING).ok().flatten().is_none_or(|v| v != "0")
}
/// Saves the choice and applies it to this process.
pub fn save(db: &Store, on: bool) -> anyhow::Result<()> {
    db.set(SETTING, if on { "1" } else { "0" })?;
    set_enabled(on);
    Ok(())
}
/// Applies the saved choice (or `GDIRECT_BYPASS_TUN`) to this process.
pub fn load(db: &Store) {
    set_enabled(env_choice().unwrap_or_else(|| saved(db)));
}
/// Forgets the detected interface, so the next connection reads the routing
/// table again (after a failed connect: the network may have changed).
pub fn forget() {
    *CACHE.lock().unwrap() = None;
}

/// The current state, from a cache refreshed every [`TTL`].
pub fn status() -> Status {
    let mut cache = CACHE.lock().unwrap();
    if cache.as_ref().is_none_or(|c| c.at.elapsed() > TTL) {
        let routes = sys::default_routes();
        let selection = pick(&routes);
        let tun = selection
            .primary
            .filter(|r| r.is_virtual())
            .map(|r| r.interface.clone());
        let physical = selection.physical.map(|r| {
            let (v4, v6) = sys::addresses(&r.interface, r.index);
            Interface {
                name: r.interface.clone(),
                index: r.index,
                v4,
                v6,
            }
        });
        let denied = physical.as_ref().and_then(|i| probe(i).err());
        *cache = Some(Detected {
            at: Instant::now(),
            physical,
            tun,
            denied,
        });
    }
    let detected = cache.as_ref().unwrap();
    Status {
        enabled: enabled(),
        physical: detected.physical.clone(),
        tun: detected.tun.clone(),
        denied: detected.denied.clone(),
    }
}
/// The interface to bind Google connections to, if any.
pub fn bound() -> Option<Interface> {
    status().bound().cloned()
}

/// Brings the saved endpoints in line with the current network path: loads
/// the setting and, when the endpoints were verified over another path (the
/// setting or the adapter changed, or a TUN came or went while bypassing is
/// off), marks them unverified so the next route scans again. Returns
/// whether it did. Cheap enough for every launch; nothing changes while the
/// path stays the same.
pub fn sync(db: &Store) -> anyhow::Result<bool> {
    load(db);
    let current = status().path();
    let stored = db.setting(SCANNED_VIA)?;
    if !stale(stored.as_deref(), current.as_deref()) {
        return Ok(false);
    }
    let retired = db.retire_endpoints("verified over another network path")?;
    if let Some(current) = &current {
        db.set(SCANNED_VIA, current)?;
    }
    Ok(retired > 0)
}
/// Records that the saved endpoints were verified over `path`.
pub fn record(db: &Store, path: Option<&str>) -> anyhow::Result<()> {
    match path {
        Some(path) => db.set(SCANNED_VIA, path),
        None => Ok(()),
    }
}

/// Binds `socket` to `interface` for connections to `v6` addresses or not.
fn bind(socket: &socket2::Socket, interface: &Interface, v6: bool) -> io::Result<()> {
    sys::bind(socket, interface, v6)
}
/// Whether this process may bind to `interface` (Linux needs CAP_NET_RAW).
fn probe(interface: &Interface) -> Result<(), String> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)
        .map_err(|e| e.to_string())?;
    bind(&socket, interface, false).map_err(|e| e.to_string())
}
fn socket_for(addr: &SocketAddr, interface: Option<&Interface>) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(
        socket2::Domain::for_address(*addr),
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    if let Some(interface) = interface {
        bind(&socket, interface, addr.is_ipv6())?;
    }
    Ok(socket)
}
/// `TcpStream::connect_timeout` to a Google endpoint, bound to the physical
/// interface when bypassing.
pub fn connect(addr: &SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
    connect_via(addr, timeout, bound().as_ref())
}
/// [`connect`] with an interface already looked up (scan sweeps).
pub fn connect_via(
    addr: &SocketAddr,
    timeout: Duration,
    interface: Option<&Interface>,
) -> io::Result<TcpStream> {
    // Loopback (tests, local tools) never leaves the machine.
    let interface = interface.filter(|_| !addr.ip().is_loopback());
    let socket = socket_for(addr, interface)?;
    socket.connect_timeout(&(*addr).into(), timeout)?;
    Ok(socket.into())
}
/// Whether `interface` (or, unbound, the system) has a route to the IPv6
/// internet; returns the source address it would use. Connecting a UDP
/// socket sends nothing.
pub fn ipv6_source(interface: Option<&Interface>) -> Option<IpAddr> {
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::DGRAM, None).ok()?;
    if let Some(interface) = interface {
        bind(&socket, interface, true).ok()?;
    }
    let target: SocketAddr = "[2001:4860:4860::8888]:443".parse().ok()?;
    socket.connect(&target.into()).ok()?;
    socket.local_addr().ok()?.as_socket().map(|a| a.ip())
}
/// Pins a reqwest client's connections to `ip` onto the physical interface
/// when bypassing.
pub fn client(
    builder: reqwest::blocking::ClientBuilder,
    ip: IpAddr,
) -> reqwest::blocking::ClientBuilder {
    match bound() {
        Some(interface) if !ip.is_loopback() => sys::bind_client(builder, &interface, ip),
        _ => builder,
    }
}

#[cfg(unix)]
fn unix_addresses(name: &str) -> (Option<Ipv4Addr>, Option<Ipv6Addr>) {
    let (mut v4, mut v6) = (None, None);
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, which is walked read-only and freed once.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return (None, None);
        }
        let mut entry = list;
        while !entry.is_null() {
            let ifa = &*entry;
            entry = ifa.ifa_next;
            if ifa.ifa_addr.is_null()
                || std::ffi::CStr::from_ptr(ifa.ifa_name).to_bytes() != name.as_bytes()
            {
                continue;
            }
            match i32::from((*ifa.ifa_addr).sa_family) {
                libc::AF_INET if v4.is_none() => {
                    let sin = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                    v4 = Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)));
                }
                libc::AF_INET6 => {
                    let sin6 = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                    let ip = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                    let global = !ip.is_unicast_link_local() && !ip.is_loopback();
                    if global && v6.is_none() {
                        v6 = Some(ip);
                    }
                }
                _ => {}
            }
        }
        libc::freeifaddrs(list);
    }
    (v4, v6)
}
#[cfg(target_vendor = "apple")]
fn index_name(index: u32) -> Option<String> {
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    // SAFETY: the buffer holds IF_NAMESIZE bytes, as if_indextoname requires.
    let name = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if name.is_null() {
        return None;
    }
    // SAFETY: on success the buffer holds a NUL-terminated name.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    Some(name.to_string_lossy().into_owned())
}

#[cfg(target_vendor = "apple")]
mod sys {
    use super::{DefaultRoute, Interface};
    use std::{io, net::IpAddr, num::NonZeroU32};

    /// Default IPv4 routes from the kernel's routing table (`NET_RT_DUMP`),
    /// scoped ones included: with a TUN up, the adapter's default route is
    /// still there, scoped to it.
    pub fn default_routes() -> Vec<DefaultRoute> {
        let mut mib = [
            libc::CTL_NET,
            libc::PF_ROUTE,
            0,
            libc::AF_INET,
            libc::NET_RT_DUMP,
            0,
        ];
        let mut buf = Vec::new();
        for _ in 0..3 {
            let mut len = 0usize;
            // SAFETY: a size query with a null buffer.
            let rv = unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    6,
                    std::ptr::null_mut(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rv != 0 {
                return Vec::new();
            }
            len += len / 4 + 1024;
            buf = vec![0u8; len];
            // SAFETY: `buf` holds `len` bytes.
            let rv = unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    6,
                    buf.as_mut_ptr().cast(),
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if rv == 0 {
                buf.truncate(len);
                break;
            }
            // The table grew between the calls: ask again.
            buf.clear();
        }
        parse_routes(&buf)
            .into_iter()
            .filter_map(|(index, flags)| {
                Some(DefaultRoute {
                    interface: super::index_name(index)?,
                    index,
                    gateway: flags & libc::RTF_GATEWAY != 0,
                    scoped: flags & libc::RTF_IFSCOPE != 0,
                    metric: 0,
                    virtual_adapter: false,
                })
            })
            .collect()
    }
    /// (interface index, flags) of each usable default route in a routing
    /// message dump.
    pub(super) fn parse_routes(buf: &[u8]) -> Vec<(u32, i32)> {
        let header = std::mem::size_of::<libc::rt_msghdr>();
        let mut routes = Vec::new();
        let mut offset = 0;
        while offset + header <= buf.len() {
            // SAFETY: at least a header's worth of bytes remains; unaligned read.
            let msg: libc::rt_msghdr =
                unsafe { std::ptr::read_unaligned(buf[offset..].as_ptr().cast()) };
            let len = usize::from(msg.rtm_msglen);
            if len == 0 || offset + len > buf.len() {
                break;
            }
            let body = &buf[offset + header..offset + len];
            offset += len;
            if msg.rtm_flags & libc::RTF_UP == 0 || msg.rtm_addrs & libc::RTA_DST == 0 {
                continue;
            }
            // Socket addresses follow in RTA_* bit order, each padded to 4 bytes.
            let mut addrs = [None::<&[u8]>; 3];
            let mut at = 0;
            for (bit, slot) in addrs.iter_mut().enumerate() {
                if msg.rtm_addrs & (1 << bit) == 0 {
                    continue;
                }
                let Some(&sa_len) = body.get(at) else { break };
                let sa_len = usize::from(sa_len);
                *slot = body.get(at..(at + sa_len).min(body.len()));
                at += if sa_len == 0 { 4 } else { (sa_len + 3) & !3 };
            }
            let zero = |sa: &[u8]| sa.iter().skip(4).all(|&b| b == 0);
            let dst_default = addrs[0].is_some_and(|sa| {
                sa.len() >= 8 && i32::from(sa[1]) == libc::AF_INET && zero(&sa[..8])
            });
            let mask_default = addrs[2].is_none_or(zero);
            if dst_default && mask_default && msg.rtm_index != 0 {
                routes.push((u32::from(msg.rtm_index), msg.rtm_flags));
            }
        }
        routes
    }
    pub fn addresses(
        name: &str,
        _index: u32,
    ) -> (Option<std::net::Ipv4Addr>, Option<std::net::Ipv6Addr>) {
        super::unix_addresses(name)
    }
    pub fn bind(socket: &socket2::Socket, interface: &Interface, v6: bool) -> io::Result<()> {
        let index = NonZeroU32::new(interface.index);
        if v6 {
            socket.bind_device_by_index_v6(index)
        } else {
            socket.bind_device_by_index_v4(index)
        }
    }
    pub fn bind_client(
        builder: reqwest::blocking::ClientBuilder,
        interface: &Interface,
        _ip: IpAddr,
    ) -> reqwest::blocking::ClientBuilder {
        // hyper-util sets IP_BOUND_IF / IPV6_BOUND_IF by the name's index.
        builder.interface(&interface.name)
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    use super::{DefaultRoute, Interface};
    use std::{io, net::IpAddr, num::NonZeroU32};

    /// Default routes of the main table. Policy-routing TUNs (sing-box,
    /// Clash) keep theirs in separate tables, so this is the uplink.
    pub fn default_routes() -> Vec<DefaultRoute> {
        let Ok(text) = std::fs::read_to_string("/proc/net/route") else {
            return Vec::new();
        };
        super::parse_proc_route(&text)
            .into_iter()
            .filter_map(|mut route| {
                let name = std::ffi::CString::new(route.interface.clone()).ok()?;
                // SAFETY: a NUL-terminated name.
                route.index = unsafe { libc::if_nametoindex(name.as_ptr()) };
                (route.index != 0).then_some(route)
            })
            .collect()
    }
    pub fn addresses(
        name: &str,
        _index: u32,
    ) -> (Option<std::net::Ipv4Addr>, Option<std::net::Ipv6Addr>) {
        super::unix_addresses(name)
    }
    pub fn bind(socket: &socket2::Socket, interface: &Interface, v6: bool) -> io::Result<()> {
        // SO_BINDTOIFINDEX applies to both families.
        let index = NonZeroU32::new(interface.index);
        if v6 {
            socket.bind_device_by_index_v6(index)
        } else {
            socket.bind_device_by_index_v4(index)
        }
    }
    pub fn bind_client(
        builder: reqwest::blocking::ClientBuilder,
        interface: &Interface,
        _ip: IpAddr,
    ) -> reqwest::blocking::ClientBuilder {
        // hyper-util sets SO_BINDTODEVICE.
        builder.interface(&interface.name)
    }
}

/// Default routes in `/proc/net/route` (Linux): destination and mask
/// 00000000, flag RTF_UP; index 0 until resolved.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android", test)),
    allow(dead_code)
)]
pub(crate) fn parse_proc_route(text: &str) -> Vec<DefaultRoute> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (name, dst, gateway, flags, metric, mask) = (
                f.first()?,
                f.get(1)?,
                f.get(2)?,
                f.get(3)?,
                f.get(6)?,
                f.get(7)?,
            );
            let flags = u32::from_str_radix(flags, 16).ok()?;
            (*dst == "00000000" && *mask == "00000000" && flags & 1 != 0).then(|| DefaultRoute {
                interface: (*name).to_owned(),
                index: 0,
                gateway: flags & 2 != 0 && *gateway != "00000000",
                scoped: false,
                metric: metric.parse().unwrap_or(0),
                virtual_adapter: false,
            })
        })
        .collect()
}

#[cfg(windows)]
mod sys {
    use super::{DefaultRoute, Interface};
    use std::{
        collections::HashMap,
        io,
        net::{IpAddr, Ipv4Addr, Ipv6Addr},
        os::windows::io::AsRawSocket,
    };
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::{
            FreeMibTable, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
            GetAdaptersAddresses, GetIpForwardTable2, IP_ADAPTER_ADDRESSES_LH,
            MIB_IPFORWARD_TABLE2,
        },
        NetworkManagement::Ndis::IfOperStatusUp,
        Networking::WinSock::{
            AF_INET, AF_INET6, AF_UNSPEC, IP_UNICAST_IF, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF,
            SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6, SOCKET, setsockopt,
        },
    };
    /// IF_TYPE_PROP_VIRTUAL and IF_TYPE_TUNNEL.
    const VIRTUAL_TYPES: [u32; 2] = [53, 131];

    struct Adapter {
        name: String,
        virtual_adapter: bool,
        metric: u32,
        up: bool,
        v4: Option<Ipv4Addr>,
        v6: Option<Ipv6Addr>,
    }
    fn wide(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: a NUL-terminated UTF-16 string owned by the adapter list.
        unsafe {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }
    }
    fn adapters() -> HashMap<u32, Adapter> {
        let mut map = HashMap::new();
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        let mut buf: Vec<u64> = Vec::new();
        for _ in 0..4 {
            buf = vec![0u64; (size as usize).div_ceil(8)];
            // SAFETY: `buf` holds `size` bytes, 8-byte aligned.
            let rv = unsafe {
                GetAdaptersAddresses(
                    u32::from(AF_UNSPEC),
                    flags,
                    std::ptr::null(),
                    buf.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            match rv {
                0 => break,
                111 => continue, // ERROR_BUFFER_OVERFLOW: `size` now fits.
                _ => return map,
            }
        }
        let mut entry = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        // SAFETY: a linked list inside `buf`, filled by GetAdaptersAddresses.
        unsafe {
            while !entry.is_null() {
                let a = &*entry;
                entry = a.Next;
                let index = a.Anonymous1.Anonymous.IfIndex;
                let name = wide(a.FriendlyName);
                let description = wide(a.Description);
                let virtual_adapter = VIRTUAL_TYPES.contains(&a.IfType)
                    || super::virtual_description(&description)
                    || super::virtual_description(&name);
                let (mut v4, mut v6) = (None, None);
                let mut unicast = a.FirstUnicastAddress;
                while !unicast.is_null() {
                    let u = &*unicast;
                    unicast = u.Next;
                    let sa = u.Address.lpSockaddr as *const SOCKADDR;
                    if sa.is_null() {
                        continue;
                    }
                    match (*sa).sa_family {
                        AF_INET if v4.is_none() => {
                            let sin = &*(sa as *const SOCKADDR_IN);
                            v4 = Some(Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr)));
                        }
                        AF_INET6 if v6.is_none() => {
                            let sin6 = &*(sa as *const SOCKADDR_IN6);
                            let ip = Ipv6Addr::from(sin6.sin6_addr.u.Byte);
                            if !ip.is_unicast_link_local() && !ip.is_loopback() {
                                v6 = Some(ip);
                            }
                        }
                        _ => {}
                    }
                }
                map.insert(
                    index,
                    Adapter {
                        name,
                        virtual_adapter,
                        metric: a.Ipv4Metric,
                        up: a.OperStatus == IfOperStatusUp,
                        v4,
                        v6,
                    },
                );
            }
        }
        map
    }
    /// Default IPv4 routes, ranked by route plus interface metric as
    /// Windows does, with the adapter's type and description to tell a TUN
    /// (wintun, sing-box, Clash, WireGuard…) from the uplink.
    pub fn default_routes() -> Vec<DefaultRoute> {
        let adapters = adapters();
        let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
        // SAFETY: GetIpForwardTable2 allocates the table; FreeMibTable frees it.
        let rows = unsafe {
            if GetIpForwardTable2(AF_INET, &mut table) != 0 || table.is_null() {
                return Vec::new();
            }
            let rows =
                std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
                    .iter()
                    .filter(|row| row.DestinationPrefix.PrefixLength == 0)
                    .map(|row| {
                        let gateway = row.NextHop.Ipv4.sin_addr.S_un.S_addr != 0;
                        (row.InterfaceIndex, row.Metric, gateway)
                    })
                    .collect::<Vec<_>>();
            FreeMibTable(table.cast());
            rows
        };
        rows.into_iter()
            .filter_map(|(index, metric, gateway)| {
                let adapter = adapters.get(&index).filter(|a| a.up)?;
                Some(DefaultRoute {
                    interface: adapter.name.clone(),
                    index,
                    gateway,
                    scoped: false,
                    metric: metric.saturating_add(adapter.metric),
                    virtual_adapter: adapter.virtual_adapter,
                })
            })
            .collect()
    }
    pub fn addresses(_name: &str, index: u32) -> (Option<Ipv4Addr>, Option<Ipv6Addr>) {
        adapters()
            .get(&index)
            .map_or((None, None), |a| (a.v4, a.v6))
    }
    pub fn bind(socket: &socket2::Socket, interface: &Interface, v6: bool) -> io::Result<()> {
        // IP_UNICAST_IF takes the index in network byte order; IPv6's does not.
        let (level, name, value) = if v6 {
            (IPPROTO_IPV6, IPV6_UNICAST_IF, interface.index)
        } else {
            (IPPROTO_IP, IP_UNICAST_IF, interface.index.to_be())
        };
        // SAFETY: a valid socket and a 4-byte option value.
        let rv = unsafe {
            setsockopt(
                socket.as_raw_socket() as SOCKET,
                level,
                name,
                (&value as *const u32).cast(),
                4,
            )
        };
        if rv == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    pub fn bind_client(
        builder: reqwest::blocking::ClientBuilder,
        interface: &Interface,
        ip: IpAddr,
    ) -> reqwest::blocking::ClientBuilder {
        // reqwest cannot set IP_UNICAST_IF; binding the adapter's own address
        // has Windows' strong host model send through that adapter.
        let local = match ip {
            IpAddr::V4(_) => interface.v4.map(IpAddr::V4),
            IpAddr::V6(_) => interface.v6.map(IpAddr::V6),
        };
        match local {
            Some(local) => builder.local_address(local),
            None => builder,
        }
    }
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    windows
)))]
mod sys {
    use super::{DefaultRoute, Interface};
    use std::{io, net::IpAddr};
    pub fn default_routes() -> Vec<DefaultRoute> {
        Vec::new()
    }
    pub fn addresses(
        _name: &str,
        _index: u32,
    ) -> (Option<std::net::Ipv4Addr>, Option<std::net::Ipv6Addr>) {
        (None, None)
    }
    pub fn bind(_socket: &socket2::Socket, _interface: &Interface, _v6: bool) -> io::Result<()> {
        Ok(())
    }
    pub fn bind_client(
        builder: reqwest::blocking::ClientBuilder,
        _interface: &Interface,
        _ip: IpAddr,
    ) -> reqwest::blocking::ClientBuilder {
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route(name: &str, gateway: bool, scoped: bool, metric: u32) -> DefaultRoute {
        DefaultRoute {
            interface: name.into(),
            index: 1,
            gateway,
            scoped,
            metric,
            virtual_adapter: false,
        }
    }
    #[test]
    fn virtual_interfaces_are_recognised() {
        for name in [
            "utun4",
            "tun0",
            "wg0",
            "tailscale0",
            "ipsec0",
            "awdl0",
            "llw0",
            "Meta",
            "bridge0",
            "lo0",
        ] {
            assert!(virtual_name(name), "{name}");
        }
        for name in [
            "en0", "en7", "eth0", "wlan0", "enp3s0", "wlp2s0", "Ethernet", "Wi-Fi",
        ] {
            assert!(!virtual_name(name), "{name}");
        }
        for desc in [
            "Wintun Userspace Tunnel",
            "sing-box TUN",
            "WireGuard Tunnel",
            "TAP-Windows Adapter V9",
            "Meta Tunnel",
        ] {
            assert!(virtual_description(desc), "{desc}");
        }
        assert!(!virtual_description("Intel(R) Wi-Fi 6 AX201 160MHz"));
        assert!(!virtual_description("Realtek PCIe GbE Family Controller"));
    }
    #[test]
    fn macos_tun_keeps_the_scoped_adapter_route() {
        // netstat -rn under sing-box: default link#20 UCSg utun4, default 192.168.31.1 UGScIg en0
        let routes = [route("utun4", false, false, 0), route("en0", true, true, 0)];
        let selection = pick(&routes);
        assert_eq!(selection.primary.unwrap().interface, "utun4");
        assert_eq!(selection.physical.unwrap().interface, "en0");
    }
    #[test]
    fn without_a_tun_the_default_route_is_the_adapter() {
        let routes = [
            route("en0", true, true, 0),
            route("en7", true, false, 0),
            route("utun3", false, true, 0),
        ];
        let selection = pick(&routes);
        assert_eq!(selection.primary.unwrap().interface, "en7");
        assert_eq!(selection.physical.unwrap().interface, "en7");
        assert!(!selection.primary.unwrap().is_virtual());
    }
    #[test]
    fn lowest_metric_wins_and_virtual_adapters_never_bind() {
        // Windows: a TUN with the lowest metric, Wi-Fi and Ethernet behind it.
        let mut tun = route("sing-box", true, false, 5);
        tun.virtual_adapter = true;
        let routes = [
            route("Wi-Fi", true, false, 55),
            tun,
            route("Ethernet", true, false, 25),
        ];
        let selection = pick(&routes);
        assert_eq!(selection.primary.unwrap().interface, "sing-box");
        assert_eq!(selection.physical.unwrap().interface, "Ethernet");
        // Nothing but tunnels: nothing to bind to.
        let routes = [
            route("utun4", false, false, 0),
            route("wg0", true, false, 0),
        ];
        assert!(pick(&routes).physical.is_none());
        assert!(pick(&[]).primary.is_none());
    }
    #[test]
    fn linux_proc_route_default_routes() {
        let text = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            wlan0\t00000000\t011FA8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
            eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
            wlan0\t001FA8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n\
            tun0\t00000000\t00000000\t0001\t0\t0\t0\t00000000\t0\t0\t0\n";
        let routes = parse_proc_route(text);
        assert_eq!(routes.len(), 3);
        assert!(routes.iter().all(|r| r.index == 0));
        let selection = pick(&routes);
        assert_eq!(selection.primary.unwrap().interface, "tun0");
        assert_eq!(selection.physical.unwrap().interface, "eth0");
        assert_eq!(selection.physical.unwrap().metric, 100);
    }
    fn status(enabled: bool, tun: Option<&str>, denied: bool) -> Status {
        Status {
            enabled,
            physical: Some(Interface {
                name: "en0".into(),
                index: 4,
                v4: None,
                v6: None,
            }),
            tun: tun.map(Into::into),
            denied: denied.then(|| "Operation not permitted".into()),
        }
    }
    #[test]
    fn network_path_names_what_carries_the_traffic() {
        // Bypassing: the adapter, TUN or not.
        assert_eq!(
            status(true, Some("utun4"), false).path().as_deref(),
            Some("direct:en0")
        );
        assert_eq!(
            status(true, None, false).path().as_deref(),
            Some("direct:en0")
        );
        // Not bypassing, or not allowed to: the TUN when there is one.
        assert_eq!(
            status(false, Some("utun4"), false).path().as_deref(),
            Some("tun:utun4")
        );
        assert_eq!(
            status(true, Some("tun0"), true).path().as_deref(),
            Some("tun:tun0")
        );
        assert!(status(true, Some("tun0"), true).bound().is_none());
        assert_eq!(
            status(false, None, false).path().as_deref(),
            Some("direct:en0")
        );
        let offline = Status {
            enabled: true,
            physical: None,
            tun: None,
            denied: None,
        };
        assert_eq!(offline.path(), None);
    }
    #[test]
    fn endpoints_go_stale_only_when_the_path_changes() {
        assert!(!stale(Some("direct:en0"), Some("direct:en0")));
        assert!(stale(Some("tun:utun4"), Some("direct:en0")));
        assert!(stale(Some("direct:en0"), Some("direct:en7")));
        // Saved before the path was recorded: verify once.
        assert!(stale(None, Some("direct:en0")));
        // Offline: keep what is saved.
        assert!(!stale(Some("direct:en0"), None));
        assert!(!stale(None, None));
    }
    #[test]
    fn sync_retires_endpoints_from_another_path() {
        let dir = std::env::temp_dir().join(format!("gdirect-netif-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Store::open(&dir.join("state.sqlite3")).unwrap();
        db.endpoint("142.250.1.1", Some(30), None, "").unwrap();
        db.set(SCANNED_VIA, "tun:utun-test").unwrap();
        // Whatever this machine's path is, it is not that one (offline aside).
        let current = super::status().path();
        let retired = sync(&db).unwrap();
        if current.is_some() {
            assert!(retired);
            assert!(db.routes(16).unwrap().is_empty());
            assert_eq!(db.setting(SCANNED_VIA).unwrap(), current);
            // Same path again: nothing changes.
            db.endpoint("142.250.1.2", Some(30), None, "").unwrap();
            assert!(!sync(&db).unwrap());
            assert_eq!(db.routes(16).unwrap().len(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[cfg(target_vendor = "apple")]
    #[test]
    fn reads_the_routing_table() {
        // Whatever the machine's network, reading the table must not fail.
        for route in sys::default_routes() {
            assert!(route.index > 0);
            assert!(!route.interface.is_empty());
        }
    }
}
