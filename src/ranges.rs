//! Google's own service address space: what `scan` sweeps and what counts as
//! a genuine Google endpoint. A snapshot is embedded; `refresh` replaces it
//! weekly from Google's published lists (goog.json minus cloud.json).
use crate::store::{Store, now};
use anyhow::{Context, Result, ensure};
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const EMBEDDED: &str = include_str!("google-ranges.txt");
const FEEDS: &str = "www.gstatic.com";
const MAX_AGE: i64 = 7 * 86400;

/// Sorted, non-overlapping inclusive address intervals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ranges {
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}
fn cidr(text: &str) -> Result<(IpAddr, u8)> {
    let (addr, len) = text.split_once('/').context("prefix needs a length")?;
    let addr: IpAddr = addr.parse()?;
    let len: u8 = len.parse()?;
    ensure!(
        len <= if addr.is_ipv4() { 32 } else { 128 },
        "prefix length out of range"
    );
    Ok((addr, len))
}
fn interval4(addr: u32, len: u8) -> (u32, u32) {
    let host = if len == 0 {
        u32::MAX
    } else {
        (1u32 << (32 - len)) - 1
    };
    (addr & !host, addr | host)
}
fn interval6(addr: u128, len: u8) -> (u128, u128) {
    let host = if len == 0 {
        u128::MAX
    } else {
        (1u128 << (128 - len)) - 1
    };
    (addr & !host, addr | host)
}
fn merge<T: Copy + Ord + num::Step>(mut v: Vec<(T, T)>) -> Vec<(T, T)> {
    v.sort();
    let mut out: Vec<(T, T)> = Vec::with_capacity(v.len());
    for (start, end) in v {
        match out.last_mut() {
            Some(last) if T::adjacent_or_overlapping(last.1, start) => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}
fn subtract<T: Copy + Ord + num::Step>(keep: &[(T, T)], drop: &[(T, T)]) -> Vec<(T, T)> {
    let mut out = Vec::new();
    for &(mut start, end) in keep {
        let mut alive = true;
        for &(hole_start, hole_end) in drop {
            if !alive || hole_end < start || hole_start > end {
                continue;
            }
            if hole_start > start {
                out.push((start, T::before(hole_start)));
            }
            if hole_end >= end {
                alive = false;
            } else {
                start = T::after(hole_end);
            }
        }
        if alive {
            out.push((start, end));
        }
    }
    out
}
/// Interval arithmetic over u32 and u128 without pulling in a crate.
mod num {
    pub trait Step: Sized {
        fn before(self) -> Self;
        fn after(self) -> Self;
        fn adjacent_or_overlapping(end: Self, start: Self) -> bool;
    }
    macro_rules! step {
        ($t:ty) => {
            impl Step for $t {
                fn before(self) -> Self {
                    self - 1
                }
                fn after(self) -> Self {
                    self + 1
                }
                fn adjacent_or_overlapping(end: Self, start: Self) -> bool {
                    start <= end.saturating_add(1)
                }
            }
        };
    }
    step!(u32);
    step!(u128);
}
impl Ranges {
    fn from_prefixes<'a>(prefixes: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let (mut v4, mut v6) = (Vec::new(), Vec::new());
        for prefix in prefixes {
            match cidr(prefix)? {
                (IpAddr::V4(a), len) => v4.push(interval4(a.into(), len)),
                (IpAddr::V6(a), len) => v6.push(interval6(a.into(), len)),
            }
        }
        Ok(Self {
            v4: merge(v4),
            v6: merge(v6),
        })
    }
    /// One CIDR per line; `#` starts a comment.
    pub fn parse(text: &str) -> Result<Self> {
        Self::from_prefixes(
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim())
                .filter(|l| !l.is_empty()),
        )
    }
    /// Google's own space from the published feeds: everything Google owns
    /// minus what Google Cloud customers can be assigned.
    pub fn from_feeds(goog: &str, cloud: &str) -> Result<Self> {
        let list = |json: &str| -> Result<Self> {
            let data: serde_json::Value = serde_json::from_str(json)?;
            let prefixes = data["prefixes"]
                .as_array()
                .context("IP range feed has no prefixes")?;
            Self::from_prefixes(
                prefixes
                    .iter()
                    .filter_map(|p| p["ipv4Prefix"].as_str().or(p["ipv6Prefix"].as_str())),
            )
        };
        let (goog, cloud) = (list(goog)?, list(cloud)?);
        let ranges = Self {
            v4: subtract(&goog.v4, &cloud.v4),
            v6: subtract(&goog.v6, &cloud.v6),
        };
        // A truncated or hostile feed must not shrink the trusted space to nothing.
        ensure!(ranges.v4.len() >= 16, "IP range feed looks incomplete");
        Ok(ranges)
    }
    pub fn contains(&self, ip: IpAddr) -> bool {
        fn find<T: Copy + Ord>(v: &[(T, T)], x: T) -> bool {
            let i = v.partition_point(|&(start, _)| start <= x);
            i > 0 && x <= v[i - 1].1
        }
        match ip {
            IpAddr::V4(a) => find(&self.v4, u32::from(a)),
            IpAddr::V6(a) => find(&self.v6, u128::from(a)),
        }
    }
    /// Network address of every IPv4 /24 inside the ranges.
    pub fn slash24s(&self) -> impl Iterator<Item = u32> + '_ {
        self.v4
            .iter()
            .flat_map(|&(start, end)| ((start >> 8)..=(end >> 8)).map(|n| n << 8))
    }
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let mut emit = |start: u128, end: u128, bits: u32, v4: bool| {
            // Split each interval into the largest aligned CIDR blocks.
            let mut cur = start;
            while cur <= end {
                let mut len = bits;
                while len > 0 && bits - len + 1 < 128 {
                    let size = 1u128 << (bits - len + 1);
                    if !cur.is_multiple_of(size) || cur + size - 1 > end {
                        break;
                    }
                    len -= 1;
                }
                let addr: IpAddr = if v4 {
                    std::net::Ipv4Addr::from(cur as u32).into()
                } else {
                    std::net::Ipv6Addr::from(cur).into()
                };
                out.push_str(&format!("{addr}/{len}\n"));
                let size = 1u128 << (bits - len);
                match cur.checked_add(size) {
                    Some(next) => cur = next,
                    None => break,
                }
            }
        };
        for &(s, e) in &self.v4 {
            emit(s.into(), e.into(), 32, true);
        }
        for &(s, e) in &self.v6 {
            emit(s, e, 128, false);
        }
        out
    }
}
/// The ranges in effect: the last refreshed copy, else the embedded snapshot.
pub fn current(db: &Store) -> Ranges {
    db.setting("google_ranges")
        .ok()
        .flatten()
        .and_then(|text| Ranges::parse(&text).ok())
        .unwrap_or_else(embedded)
}
pub fn embedded() -> Ranges {
    Ranges::parse(EMBEDDED).expect("embedded Google ranges must parse")
}
/// Refreshes the stored ranges when they are a week old. Google's feeds are
/// tried over the system network (in mainland China www.gstatic.com resolves
/// to an in-ISP Google cache that serves them), then over `via`, a verified
/// endpoint. Failure keeps the previous ranges.
pub fn refresh(db: &Store, via: Option<IpAddr>) {
    let fresh = db
        .setting("google_ranges_at")
        .ok()
        .flatten()
        .and_then(|t| t.parse::<i64>().ok())
        .is_some_and(|at| now() - at < MAX_AGE);
    if fresh {
        return;
    }
    let fetch = |ip: Option<IpAddr>| -> Result<Ranges> {
        let mut builder = crate::api::client_builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15));
        if let Some(ip) = ip {
            builder = crate::netif::client(builder, ip)
                .no_proxy()
                .resolve(FEEDS, SocketAddr::new(ip, 443));
        }
        let client = builder.build()?;
        let get = |name: &str| -> Result<String> {
            let response = client
                .get(format!("https://{FEEDS}/ipranges/{name}"))
                .send()
                .map_err(|e| e.without_url())?
                .error_for_status()
                .map_err(|e| e.without_url())?;
            Ok(response.text()?)
        };
        Ranges::from_feeds(&get("goog.json")?, &get("cloud.json")?)
    };
    let result = fetch(None).or_else(|e| match via {
        Some(ip) => fetch(Some(ip)),
        None => Err(e),
    });
    if let Ok(ranges) = result {
        let _ = db.set("google_ranges", &ranges.to_text());
        let _ = db.set("google_ranges_at", &now().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }
    #[test]
    fn embedded_snapshot_covers_google_frontends_only() {
        let r = embedded();
        for google in [
            "142.250.27.100",
            "192.179.60.132",
            "108.177.9.138",
            "2404:6800:4005::1",
        ] {
            assert!(r.contains(ip(google)), "{google}");
        }
        // The old relay seed, its exit, a Cloud customer range and poisoned DNS answers.
        for other in [
            "183.56.143.147",
            "122.10.103.81",
            "34.120.0.1",
            "69.63.184.30",
            "1.1.1.1",
        ] {
            assert!(!r.contains(ip(other)), "{other}");
        }
    }
    #[test]
    fn feeds_subtract_cloud_space() {
        let goog = r#"{"prefixes":[{"ipv4Prefix":"10.0.0.0/8"},{"ipv4Prefix":"11.0.0.0/8"},{"ipv6Prefix":"2001:db8::/32"}]}"#;
        let cloud = r#"{"prefixes":[{"ipv4Prefix":"10.1.0.0/16"},{"ipv4Prefix":"11.0.0.0/8"}]}"#;
        let list = Ranges::parse("10.0.0.0/8\n# c\n11.0.0.0/8").unwrap();
        assert_eq!(list.v4, [(0x0a00_0000, 0x0bff_ffff)]);
        let r = Ranges {
            v4: subtract(
                &list.v4,
                &Ranges::parse("10.1.0.0/16\n11.0.0.0/8").unwrap().v4,
            ),
            v6: vec![],
        };
        assert!(r.contains(ip("10.0.255.255")));
        assert!(!r.contains(ip("10.1.2.3")));
        assert!(r.contains(ip("10.2.0.0")));
        assert!(!r.contains(ip("11.0.0.1")));
        assert_eq!(
            r.to_text(),
            "10.0.0.0/16\n10.2.0.0/15\n10.4.0.0/14\n10.8.0.0/13\n10.16.0.0/12\n10.32.0.0/11\n10.64.0.0/10\n10.128.0.0/9\n"
        );
        assert_eq!(Ranges::parse(&r.to_text()).unwrap(), r);
        // Too small to be Google's real feed.
        assert!(Ranges::from_feeds(goog, cloud).is_err());
    }
    #[test]
    fn slash24s_enumerate_networks() {
        let r = Ranges::parse("1.2.3.0/23\n9.9.9.9/32").unwrap();
        let nets: Vec<_> = r.slash24s().map(std::net::Ipv4Addr::from).collect();
        assert_eq!(
            nets,
            ["1.2.2.0", "1.2.3.0", "9.9.9.0"].map(|s| s.parse::<std::net::Ipv4Addr>().unwrap())
        );
    }
    #[test]
    fn embedded_round_trips() {
        let r = embedded();
        assert_eq!(Ranges::parse(&r.to_text()).unwrap(), r);
    }
}
