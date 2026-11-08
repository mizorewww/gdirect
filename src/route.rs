//! Choosing the Google front end a command talks to.
use crate::{api, ranges, scan, store::Store};
use anyhow::{Context, Result, ensure};
use std::{net::IpAddr, path::PathBuf};

/// Effective route: endpoint, TLS identity (None = real hostname), whether
/// an automatic identity fallback is allowed on transport failure, and the
/// other verified endpoints to fail over to.
pub struct Direct {
    pub ip: IpAddr,
    pub sni: Option<String>,
    pub auto: bool,
    pub backups: Vec<api::Endpoint>,
}
impl Direct {
    /// Endpoints for a raw TLS tunnel, which always carries the real hostname:
    /// those verified with it first, those that needed the forged one last.
    pub fn tunnel_endpoints(&self) -> Vec<IpAddr> {
        let mut all: Vec<api::Endpoint> = std::iter::once((self.ip, self.sni.clone()))
            .chain(self.backups.iter().cloned())
            .collect();
        all.sort_by_key(|(_, sni)| sni.as_deref().is_some_and(|s| !s.is_empty()));
        all.into_iter().map(|(ip, _)| ip).collect()
    }
    pub fn api(&self, db: PathBuf, account: Option<String>) -> Result<api::Api> {
        // --no-sni is download-only; Drive API requests keep the real name.
        let sni = self.sni.clone().filter(|s| !s.is_empty());
        Ok(api::Api::new(db, account, Some(self.ip), sni, self.auto)?
            .with_backups(self.backups.clone()))
    }
}
/// Resolves the route: `ip` pins one Google front end, otherwise the fastest
/// verified one is used (scanning first when none is saved). `sni` is an
/// explicit identity choice: Some(name) to forge, Some("") for no SNI, None
/// to follow the scanned route and allow automatic fallback.
pub fn resolve(db: &Store, ip: Option<IpAddr>, sni: Option<String>) -> Result<Direct> {
    let auto = sni.is_none();
    // Endpoints verified over another network path (TUN bypass toggled, the
    // adapter changed) are retired first, so they are scanned again.
    crate::netif::sync(db)?;
    let google = ranges::current(db);
    if let Some(ip) = ip {
        ensure!(
            google.contains(ip),
            "{ip} is not in Google's address space; --ip takes a Google front end"
        );
        return Ok(Direct {
            ip,
            sni,
            auto,
            backups: Vec::new(),
        });
    }
    let mut routes = scan::usable(db.routes(16)?, &google);
    if routes.is_empty() {
        scan::run(db, None)?;
        routes = scan::usable(db.routes(16)?, &google);
    }
    let identity = |sni: String| (!sni.is_empty()).then_some(sni);
    let mut routes = routes.into_iter().map(|r| (r.ip, identity(r.sni)));
    let (ip, stored) = routes.next().context("no verified endpoint")?;
    Ok(Direct {
        ip,
        sni: sni.or(stored),
        auto,
        backups: routes.take(8).collect(),
    })
}
