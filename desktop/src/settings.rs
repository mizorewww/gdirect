//! The client's preferences, kept in gdirect's SQLite settings table so the
//! command line and the client share what they have in common.
use crate::i18n::Language;
use gdirect::store::Store;
use std::path::PathBuf;

/// The TLS identity transfers present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// The real hostname, falling back to the forged one per IP when reset.
    Auto,
    /// Always the forged hostname (`--sni g.cn`).
    Forged,
    /// No SNI on downloads (`--no-sni`).
    None,
}

impl Identity {
    /// The explicit choice `route::resolve` takes.
    pub fn sni(self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Forged => Some("g.cn".into()),
            Self::None => Some(String::new()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub download_dir: PathBuf,
    /// Connections per download; shared with `gdirect browse`.
    pub connections: u8,
    /// Export Google documents as Office files instead of PDF.
    pub office: bool,
    pub md5: bool,
    pub identity: Identity,
    pub ca_cert: Option<PathBuf>,
    /// A pinned front end (`--ip`); None picks the fastest.
    pub pinned: Option<std::net::IpAddr>,
    pub mount_path: PathBuf,
    pub read_only: bool,
    pub theme: Theme,
    pub language: Language,
    pub accent: usize,
}

const PREFIX: &str = "desktop.";

impl Settings {
    pub fn load(db: &Store) -> Self {
        let get = |key: &str| db.setting(&format!("{PREFIX}{key}")).ok().flatten();
        let flag = |key: &str, default: bool| get(key).map_or(default, |v| v == "1");
        let home = dirs::home_dir().unwrap_or_default();
        Self {
            download_dir: get("download_dir")
                .map(PathBuf::from)
                .or_else(dirs::download_dir)
                .unwrap_or_else(|| home.join("Downloads")),
            connections: db
                .setting("connections")
                .ok()
                .flatten()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8u8)
                .clamp(1, 16),
            office: flag("office", false),
            md5: flag("md5", true),
            identity: match get("identity").as_deref() {
                Some("forged") => Identity::Forged,
                Some("none") => Identity::None,
                _ => Identity::Auto,
            },
            ca_cert: get("ca_cert").map(PathBuf::from),
            pinned: get("pinned").and_then(|ip| ip.parse().ok()),
            mount_path: get("mount_path")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("GoogleDrive")),
            read_only: flag("read_only", false),
            theme: match get("theme").as_deref() {
                Some("light") => Theme::Light,
                Some("dark") => Theme::Dark,
                _ => Theme::System,
            },
            language: match get("language").as_deref() {
                Some("zh") => Language::Chinese,
                Some("en") => Language::English,
                _ => Language::System,
            },
            accent: get("accent").and_then(|a| a.parse().ok()).unwrap_or(0),
        }
    }

    pub fn save(&self, db: &Store) {
        let put = |key: &str, value: Option<String>| {
            let key = format!("{PREFIX}{key}");
            let _ = match value {
                Some(value) => db.set(&key, &value),
                None => db.unset(&key),
            };
        };
        let flag = |on: bool| Some(if on { "1" } else { "0" }.to_owned());
        put(
            "download_dir",
            Some(self.download_dir.display().to_string()),
        );
        put("office", flag(self.office));
        put("md5", flag(self.md5));
        put(
            "identity",
            Some(
                match self.identity {
                    Identity::Auto => "auto",
                    Identity::Forged => "forged",
                    Identity::None => "none",
                }
                .into(),
            ),
        );
        put(
            "ca_cert",
            self.ca_cert.as_ref().map(|p| p.display().to_string()),
        );
        put("pinned", self.pinned.map(|ip| ip.to_string()));
        put("mount_path", Some(self.mount_path.display().to_string()));
        put("read_only", flag(self.read_only));
        put(
            "theme",
            Some(
                match self.theme {
                    Theme::System => "system",
                    Theme::Light => "light",
                    Theme::Dark => "dark",
                }
                .into(),
            ),
        );
        put(
            "language",
            Some(
                match self.language {
                    Language::System => "system",
                    Language::Chinese => "zh",
                    Language::English => "en",
                }
                .into(),
            ),
        );
        put("accent", Some(self.accent.to_string()));
        let _ = db.set("connections", &self.connections.to_string());
    }
}
