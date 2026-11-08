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
    /// Bind Google connections to the network adapter so a TUN proxy cannot
    /// capture them; shared with `gdirect scan --bypass-tun`, and saved by
    /// `gdirect::netif::save` when switched rather than by [`Settings::save`].
    pub bypass_tun: bool,
    pub mount_path: PathBuf,
    pub read_only: bool,
    pub theme: Theme,
    pub language: Language,
    pub accent: usize,
}

const PREFIX: &str = "desktop.";

impl Settings {
    /// Reads the preferences. It also applies the TUN bypass choice to this
    /// process and retires endpoints verified over another network path, so
    /// that the route check that follows at launch sees them as gone.
    pub fn load(db: &Store) -> Self {
        let _ = gdirect::netif::sync(db);
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
            bypass_tun: gdirect::netif::saved(db),
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

/// The keyboard shortcuts the user changed: binding id → keystroke, where an
/// empty keystroke leaves the binding unset. Kept as one JSON object.
pub type KeymapOverrides = std::collections::BTreeMap<String, String>;

const KEYMAP: &str = "desktop.keymap";

pub fn load_keymap(db: &Store) -> KeymapOverrides {
    db.setting(KEYMAP)
        .ok()
        .flatten()
        .map(|json| parse_keymap(&json))
        .unwrap_or_default()
}

pub fn save_keymap(db: &Store, overrides: &KeymapOverrides) {
    let _ = if overrides.is_empty() {
        db.unset(KEYMAP)
    } else {
        db.set(KEYMAP, &serialize_keymap(overrides))
    };
}

/// Reads the stored overrides; anything that is not an object of strings is
/// skipped rather than failing the whole map.
pub fn parse_keymap(json: &str) -> KeymapOverrides {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .filter_map(|(id, keys)| Some((id, keys.as_str()?.to_owned())))
            .collect(),
        _ => KeymapOverrides::new(),
    }
}

pub fn serialize_keymap(overrides: &KeymapOverrides) -> String {
    serde_json::to_string(overrides).unwrap_or_else(|_| "{}".into())
}

/// The main window's last frame, in the coordinates of the display it was on.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
    /// The display's UUID, when the platform has one.
    pub display: Option<String>,
}

const WINDOW: &str = "desktop.window";

impl WindowState {
    pub fn load(db: &Store) -> Option<Self> {
        Self::parse(&db.setting(WINDOW).ok().flatten()?)
    }

    pub fn save(&self, db: &Store) {
        let _ = db.set(WINDOW, &self.serialize());
    }

    pub fn parse(json: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let num = |key: &str| {
            v.get(key)
                .and_then(|n| n.as_f64())
                .map(|n| n as f32)
                .filter(|n| n.is_finite())
        };
        Some(Self {
            x: num("x")?,
            y: num("y")?,
            w: num("w")?,
            h: num("h")?,
            maximized: v
                .get("maximized")
                .and_then(|m| m.as_bool())
                .unwrap_or(false),
            display: v.get("display").and_then(|d| d.as_str()).map(str::to_owned),
        })
    }

    pub fn serialize(&self) -> String {
        serde_json::json!({
            "x": self.x,
            "y": self.y,
            "w": self.w,
            "h": self.h,
            "maximized": self.maximized,
            "display": self.display,
        })
        .to_string()
    }

    /// The frame moved and shrunk onto a display of `display` = (x, y, w, h),
    /// at least `min` = (w, h) in size; None when too little of it would show
    /// there to grab the title bar.
    pub fn clamped(&self, display: (f32, f32, f32, f32), min: (f32, f32)) -> Option<Self> {
        let (dx, dy, dw, dh) = display;
        // Enough of the title bar on screen to drag the window back.
        let visible_w = (self.x + self.w).min(dx + dw) - self.x.max(dx);
        let visible_h = (self.y + self.h).min(dy + dh) - self.y.max(dy);
        if visible_w < 100. || visible_h < 40. || dw < 1. || dh < 1. {
            return None;
        }
        let w = self.w.max(min.0).min(dw.max(min.0));
        let h = self.h.max(min.1).min(dh.max(min.1));
        let x = self.x.clamp(dx, (dx + dw - w).max(dx));
        let y = self.y.clamp(dy, (dy + dh - h).max(dy));
        Some(Self {
            x,
            y,
            w,
            h,
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_round_trips() {
        let mut map = KeymapOverrides::new();
        map.insert("gdirect::Refresh secondary-r".into(), "cmd-shift-r".into());
        map.insert("files::Rename f2".into(), String::new());
        assert_eq!(parse_keymap(&serialize_keymap(&map)), map);
    }

    #[test]
    fn keymap_skips_junk() {
        assert!(parse_keymap("not json").is_empty());
        assert!(parse_keymap("[1, 2]").is_empty());
        let map = parse_keymap(r#"{"a": "cmd-k", "b": 3, "c": null}"#);
        assert_eq!(map.len(), 1);
        assert_eq!(map["a"], "cmd-k");
    }

    #[test]
    fn window_round_trips() {
        let state = WindowState {
            x: 10.,
            y: 20.,
            w: 1000.,
            h: 700.,
            maximized: true,
            display: Some("abc".into()),
        };
        assert_eq!(WindowState::parse(&state.serialize()), Some(state));
        assert_eq!(WindowState::parse(r#"{"x": 1}"#), None);
    }

    #[test]
    fn window_is_clamped_onto_the_display() {
        let display = (0., 0., 1440., 900.);
        let min = (600., 480.);
        let state = |x, y, w, h| WindowState {
            x,
            y,
            w,
            h,
            maximized: false,
            display: None,
        };
        // Fits: unchanged.
        let fits = state(100., 50., 1000., 700.);
        assert_eq!(fits.clamped(display, min), Some(fits.clone()));
        // Too small, and hanging off the right: grown and pulled back.
        let c = state(1200., 100., 300., 200.)
            .clamped(display, min)
            .unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (840., 100., 600., 480.));
        // Bigger than the display: shrunk to it.
        let c = state(0., 0., 3000., 2000.).clamped(display, min).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (0., 0., 1440., 900.));
        // Off screen entirely.
        assert_eq!(state(3000., 50., 800., 600.).clamped(display, min), None);
    }
}
