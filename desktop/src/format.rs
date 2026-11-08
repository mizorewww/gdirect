//! How sizes, times and file kinds read in the client.
use crate::tf;
use chrono::{DateTime, Datelike, Local};
use gdirect::api::File;

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1000. {
            break;
        }
        value /= 1000.;
        unit = next;
    }
    if value < 10. {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

pub fn speed(n: u64) -> String {
    format!("{}/s", bytes(n))
}

pub fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => tf!("{seconds} 秒", "{seconds}s"),
        60..3600 => {
            let (m, s) = (seconds / 60, seconds % 60);
            tf!("{m} 分 {s} 秒", "{m}m {s}s")
        }
        _ => {
            let (h, m) = (seconds / 3600, seconds % 3600 / 60);
            tf!("{h} 小时 {m} 分", "{h}h {m}m")
        }
    }
}

/// Drive's RFC 3339 time as the file list shows it: a time today, a date
/// this year, a full date before that.
pub fn modified(time: Option<&str>) -> String {
    let Some(time) = time.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else {
        return "—".into();
    };
    let time = time.with_timezone(&Local);
    let now = Local::now();
    let (year, month, day) = (time.year(), time.month(), time.day());
    let mon = time.format("%b");
    if time.date_naive() == now.date_naive() {
        clock(time)
    } else if year == now.year() {
        tf!("{month}月{day}日", "{mon} {day}")
    } else {
        tf!("{year}年{month}月{day}日", "{mon} {day}, {year}")
    }
}

/// A moment of this session, for the finished-transfer list.
pub fn clock(time: DateTime<Local>) -> String {
    let hm = time.format("%H:%M");
    tf!("今天 {hm}", "Today {hm}")
}

pub fn size(file: &File) -> String {
    match file.size.as_deref().and_then(|s| s.parse().ok()) {
        Some(n) if !file.folder() => bytes(n),
        _ => "—".into(),
    }
}

/// A file's kind: its icon and the tonal pair its tile is painted with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Pdf,
    Sheet,
    Image,
    Video,
    Audio,
    Doc,
    GoogleDoc,
    Archive,
    Other,
}

impl Kind {
    pub fn of(file: &File) -> Self {
        Self::from_mime(&file.mime_type, &file.name)
    }

    pub fn from_mime(mime: &str, name: &str) -> Self {
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if mime == "application/vnd.google-apps.folder" {
            Self::Folder
        } else if mime.starts_with("application/vnd.google-apps.") {
            Self::GoogleDoc
        } else if mime == "application/pdf" || ext == "pdf" {
            Self::Pdf
        // Drive often stores a generic MIME type; the extension decides then.
        } else if mime.starts_with("image/")
            || matches!(
                ext.as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "heic"
                    | "heif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "svg"
            )
        {
            Self::Image
        } else if mime.starts_with("video/")
            || matches!(
                ext.as_str(),
                "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv"
            )
        {
            Self::Video
        } else if mime.starts_with("audio/")
            || matches!(
                ext.as_str(),
                "mp3" | "flac" | "wav" | "m4a" | "aac" | "ogg" | "opus" | "aiff"
            )
        {
            Self::Audio
        } else if mime.contains("spreadsheet") || matches!(ext.as_str(), "xlsx" | "xls" | "csv") {
            Self::Sheet
        } else if mime.contains("word")
            || mime.starts_with("text/")
            || matches!(
                ext.as_str(),
                "doc" | "docx" | "txt" | "md" | "rtf" | "pages"
            )
        {
            Self::Doc
        } else if matches!(ext.as_str(), "zip" | "7z" | "rar" | "gz" | "tar" | "dmg") {
            Self::Archive
        } else {
            Self::Other
        }
    }

    pub fn from_path(path: &std::path::Path) -> Self {
        if path.is_dir() {
            return Self::Folder;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let ext = name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        let mime = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" => "image/",
            "mp4" | "mov" | "mkv" | "avi" | "webm" => "video/",
            "mp3" | "m4a" | "wav" | "flac" | "aac" => "audio/",
            _ => "",
        };
        Self::from_mime(mime, &name)
    }

    /// The Material Symbol's name.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Folder => "folder",
            Self::Pdf => "picture_as_pdf",
            Self::Sheet => "table_chart",
            Self::Image => "image",
            Self::Video => "movie",
            Self::Audio => "graphic_eq",
            Self::Doc => "description",
            Self::GoogleDoc => "article",
            Self::Archive => "folder_zip",
            Self::Other => "draft",
        }
    }
}

/// The export a Google document is downloaded as: MIME type and extension.
pub fn export(file: &File, office: bool) -> Option<(&'static str, &'static str)> {
    let kind = file
        .mime_type
        .strip_prefix("application/vnd.google-apps.")?;
    Some(match (kind, office) {
        ("document", true) => (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docx",
        ),
        ("spreadsheet", true) => (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        ),
        ("presentation", true) => (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "pptx",
        ),
        _ => ("application/pdf", "pdf"),
    })
}

/// A path under the home directory, written with `~`.
pub fn home(path: &std::path::Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(|p| p.to_owned())) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_fall_back_to_the_extension() {
        assert_eq!(
            Kind::from_mime("application/octet-stream", "主题曲.flac"),
            Kind::Audio
        );
        assert_eq!(
            Kind::from_mime("application/octet-stream", "a.MOV"),
            Kind::Video
        );
        assert_eq!(
            Kind::from_mime("application/octet-stream", "b.heic"),
            Kind::Image
        );
        assert_eq!(
            Kind::from_mime("application/octet-stream", "c.bin"),
            Kind::Other
        );
    }

    #[test]
    fn sizes_read_in_decimal_units() {
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(4_200_000), "4.2 MB");
        assert_eq!(bytes(12_800_000), "13 MB");
        assert_eq!(bytes(1_400_000_000), "1.4 GB");
    }

    #[test]
    fn kinds_follow_mime_then_extension() {
        assert_eq!(Kind::from_mime("application/pdf", "a"), Kind::Pdf);
        assert_eq!(
            Kind::from_mime("application/octet-stream", "a.XLSX"),
            Kind::Sheet
        );
        assert_eq!(
            Kind::from_mime("application/vnd.google-apps.folder", "x"),
            Kind::Folder
        );
        assert_eq!(Kind::from_mime("video/mp4", "x"), Kind::Video);
    }

    /// Both languages in one test: the language is a process-wide global.
    #[test]
    fn times_read_in_either_language() {
        use crate::i18n::{Language, apply};
        use chrono::TimeZone;
        let at = |y, mo, d, h, mi| {
            Local
                .with_ymd_and_hms(y, mo, d, h, mi, 0)
                .unwrap()
                .to_rfc3339()
        };
        let now = Local::now();
        let today = at(now.year(), now.month(), now.day(), 15, 53);
        // A day this year that isn't today.
        let (mo, d) = if (now.month(), now.day()) == (11, 5) {
            (11, 6)
        } else {
            (11, 5)
        };
        let this_year = at(now.year(), mo, d, 12, 0);
        let past = at(2020, 11, 5, 12, 0);

        apply(Language::Chinese);
        assert_eq!(duration(42), "42 秒");
        assert_eq!(duration(125), "2 分 5 秒");
        assert_eq!(duration(3_720), "1 小时 2 分");
        assert_eq!(modified(Some(&today)), "今天 15:53");
        assert_eq!(modified(Some(&this_year)), format!("11月{d}日"));
        assert_eq!(modified(Some(&past)), "2020年11月5日");
        assert_eq!(modified(None), "—");

        apply(Language::English);
        assert_eq!(duration(42), "42s");
        assert_eq!(duration(125), "2m 5s");
        assert_eq!(duration(3_720), "1h 2m");
        assert_eq!(modified(Some(&today)), "Today 15:53");
        assert_eq!(modified(Some(&this_year)), format!("Nov {d}"));
        assert_eq!(modified(Some(&past)), "Nov 5, 2020");
        assert_eq!(modified(None), "—");
    }
}
