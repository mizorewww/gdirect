use crate::{ranges::Ranges, store::Store};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashSet, VecDeque},
    io::Read,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const LIMIT: usize = 256;
/// Verified endpoints to collect before a sweep stops.
const TARGET: usize = 8;
/// Upper bound on TCP probes in one sweep.
const MAX_PROBES: usize = 60_000;
/// Addresses swept per batch: TCP first, then full verification of what answered.
const BATCH: usize = 2048;
const TCP_WORKERS: usize = 256;
const TCP_TIMEOUT: Duration = Duration::from_secs(2);
/// Host numbers inside a /24 where Google's general front ends answer. .187,
/// .207 and .219 are Cloud Storage-only front ends and never serve Drive.
const OFFSETS: [u8; 7] = [100, 113, 155, 132, 138, 91, 123];
/// Blocks that hold most of Google's general front ends; swept before the
/// rest of Google's space.
const CORE: &str = "142.250.0.0/15 172.217.0.0/16 172.253.0.0/16 173.194.0.0/16 74.125.0.0/16 192.178.0.0/15 64.233.160.0/19 66.102.0.0/20 108.177.0.0/17 209.85.128.0/17 216.58.192.0/19 216.239.32.0/19 108.170.192.0/18 72.14.192.0/18";
/// Google's regional IPv6 front ends follow `PREFIX:SITE:SUBNET::HOST`, with
/// sites from 4001 and subnets from c00; every host of a front-end /64
/// answers. One host per /64 is probed, and the rest of a /64 that works.
const CORE6: [&str; 4] = ["2404:6800::", "2607:f8b0::", "2a00:1450::", "2800:3f0::"];
const SITES6: std::ops::RangeInclusive<u128> = 0x4001..=0x4020;
const SUBNETS6: std::ops::RangeInclusive<u128> = 0xc00..=0xc3f;
const OFFSETS6: [u16; 7] = [0x5f, 0x64, 0x84, 0x8a, 0x8b, 0x71, 0x54];
/// IPv4 core addresses swept per IPv6 candidate, so IPv4 keeps finding
/// endpoints quickly on networks where IPv6 is blocked.
const V4_PER_V6: usize = 3;
/// What a scan reports as it goes; [`run`] prints these.
#[derive(Clone, Debug)]
pub enum Event {
    /// A candidate from a file lies outside Google's address space.
    Skipped(IpAddr),
    /// Verifying the candidates of a file.
    Candidates(usize),
    /// Sweeping Google's front ends, over IPv6 too when it has a route.
    Sweeping {
        ipv6: bool,
        target: usize,
    },
    /// How connections leave this machine: bound to the physical interface
    /// past a TUN, or through a TUN because bypassing is off or not allowed.
    /// Reported once at the start, only when a TUN or a problem is involved.
    Network(crate::netif::Status),
    /// Almost every address answered TCP: a TUN proxy is probably
    /// intercepting. `bypassing`: even though connections were bound to the
    /// physical interface.
    Intercepted {
        bypassing: bool,
    },
    /// The test download failed, so endpoints were verified with the API probe only.
    ProbeWarning(String),
    Verified {
        ip: IpAddr,
        ms: i64,
        sni: String,
    },
    Unavailable(IpAddr),
    /// A sweep batch answered TCP and is being verified. [`run`] stays quiet
    /// about it; a live view can show it.
    Swept {
        probed: usize,
        open: usize,
    },
    /// One sweep batch is done.
    Progress {
        probed: usize,
        open: usize,
        good: usize,
    },
}
impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Event::Skipped(ip) => write!(f, "  skipping {ip}: not in Google's address space"),
            Event::Candidates(n) => write!(
                f,
                "Scanning {n} candidates (TLS + Drive API + test download)…"
            ),
            Event::Sweeping { ipv6, target } => write!(
                f,
                "Sweeping Google front ends ({}) for {target} that serve Drive from this network (TCP, then TLS + Drive API + test download)…",
                if *ipv6 {
                    "IPv4 and IPv6"
                } else {
                    "IPv4; no IPv6 route"
                }
            ),
            Event::Network(status) => match (status.bound(), &status.tun, &status.denied) {
                (Some(interface), Some(tun), _) => write!(
                    f,
                    "Connecting through {} directly, past the TUN interface {tun}",
                    interface.name
                ),
                (Some(interface), None, _) => {
                    write!(f, "Connecting through {} directly", interface.name)
                }
                (None, _, Some(denied)) if status.enabled => write!(
                    f,
                    "warning: cannot bind connections to {} ({denied}), so a TUN proxy can still capture them; on Linux grant CAP_NET_RAW (sudo setcap cap_net_raw+ep \"$(command -v gdirect)\")",
                    status
                        .physical
                        .as_ref()
                        .map_or("the network adapter", |i| i.name.as_str())
                ),
                (None, Some(tun), _) if !status.enabled => write!(
                    f,
                    "Bypassing TUN proxies is off: connections go through {tun}, where your proxy rules decide"
                ),
                _ => write!(f, "Connecting through the system route"),
            },
            Event::Intercepted { bypassing: true } => write!(
                f,
                "warning: almost every address answered TCP even though connections are bound to the network adapter, so a VPN or firewall is probably intercepting below gdirect; results reflect it, not your direct network"
            ),
            Event::Intercepted { bypassing: false } => write!(
                f,
                "warning: almost every address answered TCP, so a TUN proxy or VPN is probably intercepting; results reflect the proxy, not your direct network. Turn bypassing TUN proxies back on (gdirect scan --bypass-tun on) or route Google IPs DIRECT"
            ),
            Event::ProbeWarning(problem) => write!(
                f,
                "warning: Drive test file probe failed ({problem}); IPs were verified with the Drive API probe only"
            ),
            Event::Verified { ip, ms, sni } if sni.is_empty() => write!(f, "  OK  {ip}  {ms} ms"),
            Event::Verified { ip, ms, sni } => write!(f, "  OK  {ip}  {ms} ms (SNI {sni})"),
            Event::Unavailable(ip) => write!(f, "  --  {ip}  unavailable"),
            Event::Swept { probed, open } => {
                write!(f, "  … probed {probed}, {open} answered TCP, verifying")
            }
            Event::Progress { probed, open, good } => {
                write!(
                    f,
                    "  … probed {probed}, {open} answered TCP, {good} verified"
                )
            }
        }
    }
}
/// Endpoints transfers may use, fastest first: only Google's own front ends.
/// Anything else stored by older releases (third-party relays) is ignored.
pub fn usable(routes: Vec<crate::store::Route>, google: &Ranges) -> Vec<crate::store::Route> {
    routes
        .into_iter()
        .filter(|r| google.contains(r.ip))
        .collect()
}
/// Whether this machine has a global IPv6 route. Connecting a UDP socket
/// sends nothing; it only picks a source address. A TUN proxy that hands out
/// a private (ULA) address does not count.
fn ipv6_route(interface: Option<&crate::netif::Interface>) -> bool {
    crate::netif::ipv6_source(interface).is_some_and(public)
}
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !(ip.octets()[0] == 198 && (ip.octets()[1] == 18 || ip.octets()[1] == 19))
        }
        IpAddr::V6(ip) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
        }
    }
}
/// Candidate IPs from a file; addresses outside Google's space are skipped.
fn file_candidates(
    path: &Path,
    google: &Ranges,
    report: &mut dyn FnMut(Event),
) -> Result<Vec<IpAddr>> {
    let mut ips = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let value = line.split('#').next().unwrap_or("").trim();
        if !value.is_empty() {
            let ip = value
                .parse::<IpAddr>()
                .context("candidate file must contain one IP per line")?;
            ensure!(public(ip), "candidate must be a public IP");
            if !google.contains(ip) {
                report(Event::Skipped(ip));
            } else if !ips.contains(&ip) {
                ips.push(ip);
            }
        }
    }
    ensure!(
        !ips.is_empty(),
        "no candidate IPs in Google's address space"
    );
    Ok(ips)
}
/// Small xorshift generator; the sweep order only needs to differ between
/// users so they do not all hammer the same front ends.
struct Shuffle(u64);
impl Shuffle {
    fn new() -> Self {
        let mut seed = [0; 8];
        let _ = getrandom::fill(&mut seed);
        Self(u64::from_le_bytes(seed) | 1)
    }
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
    fn apply<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.next(i + 1));
        }
    }
}
fn hosts(net: u32) -> impl Iterator<Item = IpAddr> {
    OFFSETS
        .iter()
        .map(move |&o| IpAddr::V4(Ipv4Addr::from(net | u32::from(o))))
}
fn hosts6(net: u128) -> impl Iterator<Item = IpAddr> {
    OFFSETS6
        .iter()
        .map(move |&o| IpAddr::V6(Ipv6Addr::from(net | u128::from(o))))
}
/// The same front ends as `ip`: the rest of its /24 or /64.
fn neighbours(ip: IpAddr) -> Vec<IpAddr> {
    match ip {
        IpAddr::V4(v4) => hosts(u32::from(v4) & !0xff).collect(),
        IpAddr::V6(v6) => hosts6(u128::from(v6) & !u128::from(u64::MAX)).collect(),
    }
}
/// The /64s the IPv6 front-end pattern describes.
fn core6_nets() -> Vec<u128> {
    let mut nets = Vec::new();
    for prefix in CORE6 {
        let base = u128::from(
            prefix
                .parse::<Ipv6Addr>()
                .expect("IPv6 core prefix must parse"),
        );
        for site in SITES6 {
            for subnet in SUBNETS6.clone() {
                nets.push(base | site << 80 | subnet << 64);
            }
        }
    }
    nets
}
/// Sweep order: earlier verified endpoints, DNS answers, then Google's core
/// IPv4 front-end blocks (with `ipv6`, interleaved with the IPv6 front-end
/// pattern) and finally the rest of its IPv4 space, each shuffled. No IPs
/// are built in: lists of working ones go stale, and a random batch of core
/// /24s already holds several that work. Only addresses inside `google` are
/// ever considered.
fn plan(
    google: &Ranges,
    known: &[IpAddr],
    dns: &[IpAddr],
    ipv6: bool,
    shuffle: &mut Shuffle,
) -> VecDeque<IpAddr> {
    let core = Ranges::parse(&CORE.replace(' ', "\n")).expect("core blocks must parse");
    let mut seen = HashSet::new();
    let mut order = VecDeque::new();
    let mut add = |ip: IpAddr, order: &mut VecDeque<IpAddr>| {
        if public(ip) && google.contains(ip) && seen.insert(ip) {
            order.push_back(ip);
        }
    };
    for &ip in known.iter().chain(dns) {
        add(ip, &mut order);
    }
    let (mut core_nets, mut rest_nets): (Vec<u32>, Vec<u32>) = google
        .slash24s()
        .partition(|n| core.contains(IpAddr::V4(Ipv4Addr::from(*n))));
    shuffle.apply(&mut core_nets);
    shuffle.apply(&mut rest_nets);
    let mut v6 = if ipv6 { core6_nets() } else { Vec::new() };
    shuffle.apply(&mut v6);
    let mut v6 = v6.into_iter().map(|net| hosts6(net).next().unwrap());
    for (i, ip) in core_nets.into_iter().flat_map(hosts).enumerate() {
        add(ip, &mut order);
        if (i + 1) % V4_PER_V6 == 0
            && let Some(ip) = v6.next()
        {
            add(ip, &mut order);
        }
    }
    for ip in v6.chain(rest_nets.into_iter().flat_map(hosts)) {
        add(ip, &mut order);
    }
    order
}
/// TCP connect to port 443 on every address; returns the ones that answered
/// with their connect time, fastest first.
fn tcp_sweep(ips: &[IpAddr]) -> Vec<(IpAddr, Duration)> {
    let interface = crate::netif::bound();
    let next = AtomicUsize::new(0);
    let open = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..TCP_WORKERS.min(ips.len()) {
            scope.spawn(|| {
                while let Some(&ip) = ips.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let start = Instant::now();
                    let address = SocketAddr::new(ip, 443);
                    if crate::netif::connect_via(&address, TCP_TIMEOUT, interface.as_ref()).is_ok()
                    {
                        open.lock().unwrap().push((ip, start.elapsed()));
                    }
                }
            });
        }
    });
    let mut open = open.into_inner().unwrap();
    open.sort_by_key(|&(_, t)| t);
    open
}
type Verdict = (
    IpAddr,
    std::result::Result<(i64, Option<String>), String>,
    &'static str,
);
/// Full probe of each address with 8 workers: the real SNI first, the forged
/// identity only when the real one cannot complete. Stops handing out work
/// once `enough` endpoints have verified.
fn verify(ips: &[IpAddr], enough: usize, cancel: &AtomicBool) -> Vec<Verdict> {
    let next = AtomicUsize::new(0);
    let good = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..8.min(ips.len()) {
            scope.spawn(|| {
                while good.load(Ordering::Relaxed) < enough && !cancel.load(Ordering::Relaxed) {
                    let Some(&ip) = ips.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    let (result, sni) = match probe(ip, false) {
                        Ok(ok) => (Ok(ok), ""),
                        Err(real) => match probe(ip, true) {
                            Ok(ok) => (Ok(ok), crate::api::FORGED_HOST),
                            Err(forged) => (Err(format!("{real:#} | forged: {forged:#}")), ""),
                        },
                    };
                    if result.is_ok() {
                        good.fetch_add(1, Ordering::Relaxed);
                    }
                    results.lock().unwrap().push((ip, result, sni));
                }
            });
        }
    });
    results.into_inner().unwrap()
}
// A missing or changed public test file is reported as a warning, not as a dead IP.
//
// `forged` selects the fallback identity: the URL (and therefore SNI and
// certificate name) is g.cn while the Host header still carries the real
// Google host. Google's front end routes by Host, so the same probes work.
fn probe(ip: IpAddr, forged: bool) -> Result<(i64, Option<String>)> {
    use crate::api::{API_HOST, FORGED_HOST};
    const FILE_HOST: &str = "drive.usercontent.google.com";
    let (api_url, file_url) = if forged {
        (FORGED_HOST, FORGED_HOST)
    } else {
        (API_HOST, FILE_HOST)
    };
    // Bound to the physical interface when bypassing: a TUN proxy would
    // otherwise carry the real-SNI probe and make a blocked endpoint pass.
    let client = crate::netif::client(crate::api::client_builder(), ip)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .resolve(api_url, SocketAddr::new(ip, 443))
        .resolve(file_url, SocketAddr::new(ip, 443))
        .build()?;
    let start = Instant::now();
    let response = client
        .get(format!("https://{api_url}/drive/v3/about?fields=user"))
        .header(reqwest::header::HOST, API_HOST)
        .send()
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status() == reqwest::StatusCode::UNAUTHORIZED,
        "unexpected Drive API status {}",
        response.status()
    );
    let data: serde_json::Value = response.json()?;
    ensure!(data["error"]["code"] == 401, "invalid Drive API response");
    let response = client
        .get(format!(
            "https://{file_url}/download?id=0B9P1L--7Wd2vU3VUVlFnbTgtS2c&export=download&confirm=t"
        ))
        .header(reqwest::header::HOST, FILE_HOST)
        .send()
        .map_err(|e| e.without_url())?;
    let status = response.status();
    let warning = if matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    ) {
        Some(format!("HTTP {status}"))
    } else {
        ensure!(status.is_success(), "Drive download probe HTTP {status}");
        let mut bytes = Vec::new();
        response.take(32).read_to_end(&mut bytes)?;
        (bytes != b"spam\n").then(|| "unexpected content".into())
    };
    Ok((start.elapsed().as_millis() as i64, warning))
}
/// Records verdicts and prints the working endpoints; returns how many verified.
fn record(
    db: &Store,
    verdicts: Vec<Verdict>,
    store_failures: impl Fn(IpAddr) -> bool,
    warned: &mut bool,
    report: &mut dyn FnMut(Event),
) -> Result<Vec<IpAddr>> {
    let mut good = Vec::new();
    for (ip, result, sni) in verdicts {
        match result {
            Ok((ms, problem)) => {
                if let Some(problem) = problem
                    && !*warned
                {
                    report(Event::ProbeWarning(problem));
                    *warned = true;
                }
                db.endpoint(&ip.to_string(), Some(ms), None, sni)?;
                report(Event::Verified {
                    ip,
                    ms,
                    sni: sni.to_owned(),
                });
                good.push(ip);
            }
            Err(error) if store_failures(ip) => {
                db.endpoint(&ip.to_string(), None, Some(&error), "")?;
                report(Event::Unavailable(ip));
            }
            Err(_) => {}
        }
    }
    Ok(good)
}
pub fn run(db: &Store, path: Option<&Path>) -> Result<()> {
    run_with(db, path, &AtomicBool::new(false), &mut |event| {
        if !matches!(event, Event::Swept { .. }) {
            eprintln!("{event}")
        }
    })?;
    eprintln!("Verified endpoints saved in SQLite.");
    Ok(())
}
/// [`run`], reporting progress to `report` instead of printing it. Setting
/// `cancel` stops verifying and sweeping; addresses already verified are kept.
pub fn run_with(
    db: &Store,
    path: Option<&Path>,
    cancel: &AtomicBool,
    report: &mut dyn FnMut(Event),
) -> Result<()> {
    let google = crate::ranges::current(db);
    let mut warned = false;
    // Endpoints from another network path are re-verified, not trusted.
    crate::netif::sync(db)?;
    let network = crate::netif::status();
    let bypassing = network.bound().is_some();
    let via = network.path();
    if network.tun.is_some() || (network.enabled && network.denied.is_some()) {
        report(Event::Network(network.clone()));
    }
    if let Some(path) = path {
        let ips = file_candidates(path, &google, report)?;
        report(Event::Candidates(ips.len()));
        let verdicts = verify(&ips, usize::MAX, cancel);
        record(db, verdicts, |_| true, &mut warned, report)?;
    } else {
        crate::ranges::refresh(db, db.best_route()?.map(|r| r.ip));
        let google = crate::ranges::current(db);
        let known: Vec<IpAddr> = db
            .rescan_candidates(LIMIT)?
            .iter()
            .filter_map(|ip| ip.parse().ok())
            .collect();
        // DNS supplies candidates only; poisoned answers fall outside Google's space.
        let dns: Vec<IpAddr> = ["www.googleapis.com", "drive.usercontent.google.com"]
            .iter()
            .filter_map(|host| (*host, 443).to_socket_addrs().ok())
            .flatten()
            .map(|a| a.ip())
            .collect();
        let ipv6 = ipv6_route(network.bound());
        let mut queue = plan(&google, &known, &dns, ipv6, &mut Shuffle::new());
        let mut tried: HashSet<IpAddr> = HashSet::new();
        let mut reached: HashSet<IpAddr> = HashSet::new();
        report(Event::Sweeping {
            ipv6,
            target: TARGET,
        });
        let (mut probed, mut open, mut good) = (0, 0, 0);
        let mut intercepted = false;
        while good < TARGET
            && probed < MAX_PROBES
            && !queue.is_empty()
            && !cancel.load(Ordering::Relaxed)
        {
            let batch: Vec<IpAddr> = std::iter::from_fn(|| queue.pop_front())
                .filter(|ip| tried.insert(*ip))
                .take(BATCH)
                .collect();
            probed += batch.len();
            let answered = tcp_sweep(&batch);
            open += answered.len();
            // Real networks block most of Google; a TUN proxy accepts every connect.
            if batch.len() >= 256 && answered.len() * 10 >= batch.len() * 9 && !intercepted {
                intercepted = true;
                report(Event::Intercepted { bypassing });
            }
            let candidates: Vec<IpAddr> = answered.into_iter().map(|(ip, _)| ip).collect();
            reached.extend(&candidates);
            report(Event::Swept { probed, open });
            let verdicts = verify(&candidates, TARGET - good, cancel);
            let working = record(db, verdicts, |ip| known.contains(&ip), &mut warned, report)?;
            good += working.len();
            // Front ends come in groups: the rest of a working /24 or /64 goes next.
            for &ip in working.iter().rev() {
                for neighbour in neighbours(ip) {
                    if !tried.contains(&neighbour) {
                        queue.push_front(neighbour);
                    }
                }
            }
            report(Event::Progress { probed, open, good });
        }
        // Earlier endpoints that no longer even answer TCP are marked dead.
        for ip in known
            .iter()
            .filter(|ip| tried.contains(ip) && !reached.contains(ip))
        {
            db.endpoint(&ip.to_string(), None, Some("no TCP answer"), "")?;
        }
    }
    crate::netif::record(db, via.as_deref())?;
    ensure!(
        !usable(db.routes(LIMIT)?, &google).is_empty(),
        "{}",
        if bypassing {
            "no usable Google front end found from this network; provide candidates with gdirect scan --ips FILE"
        } else {
            "no usable Google front end found; with a TUN proxy, turn bypassing it on (gdirect scan --bypass-tun on) or route Google IPs DIRECT, or provide candidates with gdirect scan --ips FILE"
        }
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_fake_and_local_addresses() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "198.18.0.1",
            "198.19.0.2",
            "::1",
            "fc00::1",
        ] {
            assert!(!public(s.parse().unwrap()));
        }
        assert!(public("142.250.27.100".parse().unwrap()));
    }
    #[test]
    fn sweep_order_keeps_to_google_space() {
        let google = crate::ranges::embedded();
        let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
        let known = [ip("142.250.27.100"), ip("183.56.143.147")];
        let dns = [
            ip("69.63.184.30"),
            ip("172.217.115.4"),
            ip("142.250.27.100"),
        ];
        let order = plan(&google, &known, &dns, false, &mut Shuffle(42));
        // Stored and DNS answers first, relays and poisoned answers dropped, no repeats.
        assert_eq!(order[0], ip("142.250.27.100"));
        assert_eq!(order[1], ip("172.217.115.4"));
        assert!(order.iter().all(|a| google.contains(*a)));
        assert_eq!(order.iter().collect::<HashSet<_>>().len(), order.len());
        // Core blocks come before the rest; Cloud Storage-only hosts are never swept.
        let core = Ranges::parse(&CORE.replace(' ', "\n")).unwrap();
        let first_other = order.iter().position(|a| !core.contains(*a)).unwrap();
        assert!(first_other > 1000);
        assert!(order.iter().skip(first_other).all(|a| !core.contains(*a)));
        assert!(order.iter().all(|a| match a {
            IpAddr::V4(v4) =>
                ![187, 207, 219].contains(&v4.octets()[3]) || known.contains(a) || dns.contains(a),
            _ => true,
        }));
        assert!(order.iter().all(IpAddr::is_ipv4));
    }
    #[test]
    fn ipv6_pattern_is_interleaved_with_ipv4() {
        let google = crate::ranges::embedded();
        let order = plan(&google, &[], &[], true, &mut Shuffle(42));
        let v6: Vec<_> = order.iter().filter(|a| a.is_ipv6()).collect();
        // Every pattern /64 is Google's own and gets one probe.
        assert_eq!(v6.len(), core6_nets().len());
        assert!(v6.iter().all(|a| a.to_string().ends_with("::5f")));
        assert!(
            order
                .iter()
                .any(|a| a.to_string() == "2404:6800:4003:c00::5f")
        );
        // The first batch is mostly IPv4.
        let first = order.iter().take(2048).filter(|a| a.is_ipv6()).count();
        assert_eq!(first, 2048 / (V4_PER_V6 + 1));
    }
    #[test]
    fn neighbours_share_the_network() {
        let ip = |s: &str| -> IpAddr { s.parse().unwrap() };
        let v4 = neighbours(ip("142.250.27.100"));
        assert!(v4.contains(&ip("142.250.27.113")));
        assert_eq!(v4.len(), OFFSETS.len());
        let v6 = neighbours(ip("2404:6800:4003:c00::abcd"));
        assert!(v6.contains(&ip("2404:6800:4003:c00::5f")));
        assert!(
            v6.iter()
                .all(|a| a.to_string().starts_with("2404:6800:4003:c00::"))
        );
    }
    #[test]
    fn only_google_endpoints_are_usable() {
        let google = crate::ranges::embedded();
        let route = |ip: &str| crate::store::Route {
            ip: ip.parse().unwrap(),
            sni: String::new(),
        };
        let ips = |routes: Vec<crate::store::Route>| -> Vec<String> {
            routes.iter().map(|r| r.ip.to_string()).collect()
        };
        let routes = || {
            vec![
                route("142.250.27.100"),
                route("1.2.3.4"),
                route("183.56.143.147"),
                route("2404:6800:4003:c00::5f"),
                route("192.179.60.132"),
            ]
        };
        // Order is kept (the store returns fastest first).
        assert_eq!(
            ips(usable(routes(), &google)),
            ["142.250.27.100", "2404:6800:4003:c00::5f", "192.179.60.132"]
        );
    }
    #[test]
    fn shuffle_is_a_permutation() {
        let mut items: Vec<u32> = (0..100).collect();
        Shuffle(7).apply(&mut items);
        assert_ne!(items, (0..100).collect::<Vec<_>>());
        items.sort();
        assert_eq!(items, (0..100).collect::<Vec<_>>());
    }
}
