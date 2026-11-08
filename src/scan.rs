use crate::{ranges::Ranges, store::Store};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{HashSet, VecDeque},
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs},
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
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
/// Endpoints transfers may use, fastest first: only Google's own front ends.
/// Anything else stored by older releases (third-party relays) is ignored.
pub fn usable(routes: Vec<crate::store::Route>, google: &Ranges) -> Vec<crate::store::Route> {
    routes
        .into_iter()
        .filter(|r| google.contains(r.ip))
        .collect()
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
fn file_candidates(path: &Path, google: &Ranges) -> Result<Vec<IpAddr>> {
    let mut ips = Vec::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let value = line.split('#').next().unwrap_or("").trim();
        if !value.is_empty() {
            let ip = value
                .parse::<IpAddr>()
                .context("candidate file must contain one IP per line")?;
            ensure!(public(ip), "candidate must be a public IP");
            if !google.contains(ip) {
                eprintln!("  skipping {ip}: not in Google's address space");
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
/// Sweep order: earlier verified endpoints, DNS answers, then Google's core
/// front-end blocks and finally the rest of its space, each shuffled. No IPs
/// are built in: lists of working ones go stale, and a random batch of core
/// /24s already holds several that work. Only addresses inside `google` are
/// ever considered.
fn plan(
    google: &Ranges,
    known: &[IpAddr],
    dns: &[IpAddr],
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
    for net in core_nets.into_iter().chain(rest_nets) {
        for ip in hosts(net) {
            add(ip, &mut order);
        }
    }
    order
}
/// TCP connect to port 443 on every address; returns the ones that answered
/// with their connect time, fastest first.
fn tcp_sweep(ips: &[IpAddr]) -> Vec<(IpAddr, Duration)> {
    let next = AtomicUsize::new(0);
    let open = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..TCP_WORKERS.min(ips.len()) {
            scope.spawn(|| {
                while let Some(&ip) = ips.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let start = Instant::now();
                    if TcpStream::connect_timeout(&SocketAddr::new(ip, 443), TCP_TIMEOUT).is_ok() {
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
fn verify(ips: &[IpAddr], enough: usize) -> Vec<Verdict> {
    let next = AtomicUsize::new(0);
    let good = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..8.min(ips.len()) {
            scope.spawn(|| {
                while good.load(Ordering::Relaxed) < enough {
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
    let client = crate::api::client_builder()
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
) -> Result<Vec<IpAddr>> {
    let mut good = Vec::new();
    for (ip, result, sni) in verdicts {
        match result {
            Ok((ms, problem)) => {
                if let Some(problem) = problem
                    && !*warned
                {
                    eprintln!(
                        "warning: Drive test file probe failed ({problem}); IPs were verified with the Drive API probe only"
                    );
                    *warned = true;
                }
                db.endpoint(&ip.to_string(), Some(ms), None, sni)?;
                if sni.is_empty() {
                    eprintln!("  OK  {ip}  {ms} ms");
                } else {
                    eprintln!("  OK  {ip}  {ms} ms (SNI {sni})");
                }
                good.push(ip);
            }
            Err(error) if store_failures(ip) => {
                db.endpoint(&ip.to_string(), None, Some(&error), "")?;
                eprintln!("  --  {ip}  unavailable");
            }
            Err(_) => {}
        }
    }
    Ok(good)
}
pub fn run(db: &Store, path: Option<&Path>) -> Result<()> {
    let google = crate::ranges::current(db);
    let mut warned = false;
    if let Some(path) = path {
        let ips = file_candidates(path, &google)?;
        eprintln!(
            "Scanning {} candidates (TLS + Drive API + test download)…",
            ips.len()
        );
        let verdicts = verify(&ips, usize::MAX);
        record(db, verdicts, |_| true, &mut warned)?;
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
        let mut queue = plan(&google, &known, &dns, &mut Shuffle::new());
        let mut tried: HashSet<IpAddr> = HashSet::new();
        let mut reached: HashSet<IpAddr> = HashSet::new();
        eprintln!(
            "Sweeping Google front ends for {TARGET} that serve Drive from this network (TCP, then TLS + Drive API + test download)…"
        );
        let (mut probed, mut open, mut good) = (0, 0, 0);
        let mut intercepted = false;
        while good < TARGET && probed < MAX_PROBES && !queue.is_empty() {
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
                eprintln!(
                    "warning: almost every address answered TCP, so a TUN proxy or VPN is probably intercepting; results reflect the proxy, not your direct network. Route Google IPs DIRECT for a real scan"
                );
            }
            let candidates: Vec<IpAddr> = answered.into_iter().map(|(ip, _)| ip).collect();
            reached.extend(&candidates);
            let verdicts = verify(&candidates, TARGET - good);
            let working = record(db, verdicts, |ip| known.contains(&ip), &mut warned)?;
            good += working.len();
            // Front ends come in groups: the rest of a working /24 goes next.
            for ip in working.iter().rev() {
                if let IpAddr::V4(v4) = ip {
                    for neighbour in hosts(u32::from(*v4) & !0xff) {
                        if !tried.contains(&neighbour) {
                            queue.push_front(neighbour);
                        }
                    }
                }
            }
            eprintln!("  … probed {probed}, {open} answered TCP, {good} verified");
        }
        // Earlier endpoints that no longer even answer TCP are marked dead.
        for ip in known
            .iter()
            .filter(|ip| tried.contains(ip) && !reached.contains(ip))
        {
            db.endpoint(&ip.to_string(), None, Some("no TCP answer"), "")?;
        }
    }
    ensure!(
        !usable(db.routes(LIMIT)?, &google).is_empty(),
        "no usable Google front end found; route Google IPs DIRECT if you use a TUN proxy, or provide candidates with gdirect scan --ips FILE"
    );
    eprintln!("Verified endpoints saved in SQLite.");
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
        let order = plan(&google, &known, &dns, &mut Shuffle(42));
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
                route("192.179.60.132"),
            ]
        };
        // Order is kept (the store returns fastest first).
        assert_eq!(
            ips(usable(routes(), &google)),
            ["142.250.27.100", "192.179.60.132"]
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
