//! Where the Files page is: a place in Drive (My Drive, Shared with me, a
//! Shared drive, a search…) and the folders opened below it, and what may be
//! done there.
use crate::i18n::t;
use crate::tf;
use gdirect::api::{File, Owner, Query, SearchKind, SearchSpec, SharedDrive};

pub const SHORTCUT: &str = "application/vnd.google-apps.shortcut";
const FOLDER: &str = "application/vnd.google-apps.folder";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    MyDrive,
    Shared,
    Starred,
    Recent,
    Trash,
    Drive { id: String, name: String },
    Search(SearchSpec),
}

impl Place {
    /// The name its breadcrumb and rail item start from.
    pub fn name(&self) -> String {
        match self {
            Self::MyDrive => t("我的云端硬盘", "My Drive").into(),
            Self::Shared => t("与我共享", "Shared with me").into(),
            Self::Starred => t("已加星标", "Starred").into(),
            Self::Recent => t("最近", "Recent").into(),
            Self::Trash => t("回收站", "Trash").into(),
            Self::Drive { name, .. } => name.clone(),
            Self::Search(_) => t("搜索结果", "Search results").into(),
        }
    }

    /// The navigation rail's value for it.
    pub fn value(&self) -> String {
        match self {
            Self::MyDrive => "place:mydrive".into(),
            Self::Shared => "place:shared".into(),
            Self::Starred => "place:starred".into(),
            Self::Recent => "place:recent".into(),
            Self::Trash => "place:trash".into(),
            Self::Drive { id, .. } => format!("drive:{id}"),
            Self::Search(_) => "search".into(),
        }
    }

    /// The place a rail value names; Shared drives are looked up in `drives`.
    pub fn from_value(value: &str, drives: &[SharedDrive]) -> Option<Self> {
        Some(match value {
            "place:mydrive" => Self::MyDrive,
            "place:shared" => Self::Shared,
            "place:starred" => Self::Starred,
            "place:recent" => Self::Recent,
            "place:trash" => Self::Trash,
            _ => {
                let id = value.strip_prefix("drive:")?;
                let drive = drives.iter().find(|d| d.id == id)?;
                Self::Drive {
                    id: drive.id.clone(),
                    name: drive.name.clone(),
                }
            }
        })
    }

    /// A tree of folders the place is the top of, rather than a list of
    /// items gathered from many folders.
    fn tree(&self) -> bool {
        matches!(self, Self::MyDrive | Self::Drive { .. })
    }
}

/// A folder opened below a place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub id: String,
    pub name: String,
    /// Whether files can be added to it; None when Drive did not say.
    pub writable: Option<bool>,
}

impl Crumb {
    pub fn of(file: &File) -> Self {
        Self {
            id: file.id.clone(),
            name: file.name.clone(),
            writable: file.capabilities.as_ref().map(|c| c.can_add_children),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub place: Place,
    pub path: Vec<Crumb>,
    /// The Shared drive the opened folders are in, which their listing names.
    pub drive: Option<String>,
}

impl Location {
    pub fn new(place: Place) -> Self {
        let drive = match &place {
            Place::Drive { id, .. } => Some(id.clone()),
            _ => None,
        };
        Self {
            place,
            path: Vec::new(),
            drive,
        }
    }

    /// The folder on screen, which new files go to; None for a list of
    /// items from many folders.
    pub fn folder(&self) -> Option<&str> {
        match (self.path.last(), &self.place) {
            (Some(crumb), _) => Some(&crumb.id),
            (None, Place::MyDrive) => Some("root"),
            (None, Place::Drive { id, .. }) => Some(id),
            _ => None,
        }
    }

    /// The folder's name, for messages.
    pub fn folder_name(&self) -> String {
        self.path
            .last()
            .map_or_else(|| self.place.name(), |c| c.name.clone())
    }

    /// The listing's query.
    pub fn query(&self) -> Query {
        if let Some(crumb) = self.path.last() {
            return Query::folder(&crumb.id, self.drive.as_deref());
        }
        match &self.place {
            Place::MyDrive => Query::folder("root", None),
            Place::Drive { id, .. } => Query::folder(id, Some(id)),
            Place::Shared => Query::shared_with_me(),
            Place::Starred => Query::starred(),
            Place::Recent => Query::recent(),
            Place::Trash => Query::trash(),
            Place::Search(spec) => Query::search(spec),
        }
    }

    /// Breadcrumb labels: the place's name, then each folder's.
    pub fn crumbs(&self) -> Vec<String> {
        std::iter::once(self.place.name())
            .chain(self.path.iter().map(|c| c.name.clone()))
            .collect()
    }

    /// Opens `folder` below the current one.
    pub fn enter(&mut self, folder: &File) {
        if folder.drive_id.is_some() {
            self.drive = folder.drive_id.clone();
        }
        self.path.push(Crumb::of(folder));
    }

    /// Returns to breadcrumb `depth`: 0 is the place itself.
    pub fn back_to(&mut self, depth: usize) {
        self.path.truncate(depth);
        if self.path.is_empty() && !matches!(self.place, Place::Drive { .. }) {
            self.drive = None;
        }
    }

    /// Items gathered from many folders, which a Location column places.
    pub fn flat(&self) -> bool {
        self.path.is_empty() && !self.place.tree()
    }

    pub fn trash(&self) -> bool {
        matches!(self.place, Place::Trash)
    }

    pub fn search(&self) -> Option<&SearchSpec> {
        match &self.place {
            Place::Search(spec) if self.path.is_empty() => Some(spec),
            _ => None,
        }
    }

    /// Whether files can be uploaded, created or pasted here: in My Drive
    /// and Shared drives, and in a folder Drive lets the user add to.
    pub fn writable(&self) -> bool {
        match self.path.last() {
            _ if self.trash() => false,
            Some(crumb) => crumb.writable.unwrap_or(self.place.tree()),
            None => self.place.tree(),
        }
    }
}

/// The real place of `folder`, found from its ancestors (None when they
/// could not be read): its Shared drive, My Drive (whose top is `root`), or
/// Shared with me.
pub fn located(
    folder: &File,
    ancestors: Option<Vec<(String, String)>>,
    root: Option<&str>,
    drives: &[SharedDrive],
) -> Location {
    let chain = ancestors.unwrap_or_default();
    let crumb = |(id, name): (String, String)| Crumb {
        id,
        name,
        writable: None,
    };
    let (place, above) = match &folder.drive_id {
        Some(drive) => {
            let name = drives
                .iter()
                .find(|d| d.id == *drive)
                .map(|d| d.name.clone())
                .or_else(|| chain.first().map(|c| c.1.clone()))
                .unwrap_or_default();
            let above = chain.into_iter().skip_while(|(id, _)| id == drive);
            (
                Place::Drive {
                    id: drive.clone(),
                    name,
                },
                above.collect::<Vec<_>>(),
            )
        }
        None if chain
            .first()
            .is_some_and(|(id, _)| Some(id.as_str()) == root) =>
        {
            (Place::MyDrive, chain.into_iter().skip(1).collect())
        }
        None => (Place::Shared, chain),
    };
    let mut location = Location::new(place);
    location.path = above.into_iter().map(crumb).collect();
    location.path.push(Crumb::of(folder));
    location
}

/// How long ago a search's results were last modified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modified {
    #[default]
    Any,
    Week,
    Month,
    Year,
}

impl Modified {
    pub const ALL: [Self; 4] = [Self::Any, Self::Week, Self::Month, Self::Year];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => t("任何时间", "Any time"),
            Self::Week => t("过去 7 天", "Last 7 days"),
            Self::Month => t("过去 30 天", "Last 30 days"),
            Self::Year => t("过去一年", "Last year"),
        }
    }

    fn days(self) -> Option<i64> {
        match self {
            Self::Any => None,
            Self::Week => Some(7),
            Self::Month => Some(30),
            Self::Year => Some(365),
        }
    }
}

/// The search chips' choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub kind: Option<SearchKind>,
    pub owner: Owner,
    pub modified: Modified,
    pub full_text: bool,
}

impl Filters {
    /// The search for `text` with these filters, as of `now`.
    pub fn spec(&self, text: &str, now: chrono::DateTime<chrono::Utc>) -> SearchSpec {
        SearchSpec {
            text: text.trim().to_owned(),
            full_text: self.full_text,
            kind: self.kind,
            owner: self.owner,
            modified_after: self.modified.days().map(|days| {
                (now - chrono::Duration::days(days))
                    .format("%Y-%m-%dT%H:%M:%S")
                    .to_string()
            }),
        }
    }

    /// Whether a search has anything to look for.
    pub fn any(&self) -> bool {
        self.kind.is_some() || self.owner != Owner::Any || self.modified != Modified::Any
    }
}

/// The type chip's choices, in its menu's order.
pub const KINDS: [SearchKind; 9] = [
    SearchKind::Folder,
    SearchKind::Document,
    SearchKind::Spreadsheet,
    SearchKind::Presentation,
    SearchKind::Pdf,
    SearchKind::Image,
    SearchKind::Video,
    SearchKind::Audio,
    SearchKind::Archive,
];

pub fn kind_label(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Folder => t("文件夹", "Folders"),
        SearchKind::Document => t("文档", "Documents"),
        SearchKind::Spreadsheet => t("电子表格", "Spreadsheets"),
        SearchKind::Presentation => t("演示文稿", "Presentations"),
        SearchKind::Pdf => "PDF",
        SearchKind::Image => t("图片", "Images"),
        SearchKind::Video => t("视频", "Videos"),
        SearchKind::Audio => t("音频", "Audio"),
        SearchKind::Archive => t("压缩文件", "Archives"),
    }
}

pub fn owner_label(owner: Owner) -> &'static str {
    match owner {
        Owner::Any => t("任何人", "Anyone"),
        Owner::Me => t("我", "Me"),
        Owner::NotMe => t("不是我", "Not me"),
    }
}

/// A Google Docs, Sheets, Slides… file, which opens in the browser.
pub fn native(file: &File) -> bool {
    file.workspace() && !file.folder() && file.mime_type != SHORTCUT
}

/// Where a shortcut points: its target's ID and whether that is a folder.
pub fn shortcut(file: &File) -> Option<(&str, bool)> {
    let details = file.shortcut_details.as_ref()?;
    (file.mime_type == SHORTCUT).then(|| {
        (
            details.target_id.as_str(),
            details.target_mime_type.as_deref() == Some(FOLDER),
        )
    })
}

/// What can be done with one file, from where it is and what Drive allows.
/// A file without capabilities (from an older cache) allows everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions {
    pub open: bool,
    pub download: bool,
    pub rename: bool,
    pub copy: bool,
    pub cut: bool,
    pub trash: bool,
    pub restore: bool,
    pub delete: bool,
    pub star: bool,
    pub link: bool,
    pub share: bool,
}

impl Actions {
    pub fn of(file: &File, in_trash: bool) -> Self {
        let caps = file.capabilities.as_ref();
        let can = |f: fn(&gdirect::api::Capabilities) -> bool| caps.is_none_or(f);
        if in_trash {
            return Self {
                restore: can(|c| c.can_untrash),
                delete: can(|c| c.can_delete),
                ..Self::default()
            };
        }
        let link = file.web_view_link.is_some();
        let download = match shortcut(file) {
            Some((_, folder)) => !folder,
            None => file.folder() || file.downloadable(),
        };
        Self {
            open: true,
            download: download && can(|c| c.can_download),
            rename: can(|c| c.can_rename),
            copy: !file.folder() && can(|c| c.can_copy),
            cut: can(|c| c.can_rename),
            trash: can(|c| c.can_trash),
            restore: false,
            delete: false,
            star: true,
            link,
            share: link && can(|c| c.can_share),
        }
    }

    /// What every one of `files` allows.
    pub fn all<'a>(files: impl IntoIterator<Item = &'a File>, in_trash: bool) -> Self {
        let mut files = files.into_iter();
        let Some(first) = files.next() else {
            return Self::default();
        };
        files.fold(Self::of(first, in_trash), |a, f| {
            let b = Self::of(f, in_trash);
            Self {
                open: a.open && b.open,
                download: a.download && b.download,
                rename: a.rename && b.rename,
                copy: a.copy && b.copy,
                cut: a.cut && b.cut,
                trash: a.trash && b.trash,
                restore: a.restore && b.restore,
                delete: a.delete && b.delete,
                star: a.star && b.star,
                link: a.link && b.link,
                share: a.share && b.share,
            }
        })
    }
}

/// The Location column's text: the name of the item's folder, from `names`
/// once it has been read.
pub fn parent_label(
    file: &File,
    root: Option<&str>,
    names: &std::collections::HashMap<String, String>,
) -> Option<String> {
    match file.parents.first() {
        None if file.shared_with_me_time.is_some() || !file.owners.iter().any(|o| o.me) => {
            Some(Place::Shared.name())
        }
        None => None,
        Some(id) if Some(id.as_str()) == root => Some(Place::MyDrive.name()),
        Some(id) => names.get(id).cloned(),
    }
}

/// "已使用 3.2 GB / 15 GB" or "Used 3.2 GB of 15 GB".
pub fn quota(usage: u64, limit: Option<u64>) -> String {
    let used = crate::format::bytes(usage);
    match limit {
        Some(limit) => {
            let limit = crate::format::bytes(limit);
            tf!("已使用 {used} / {limit}", "{used} of {limit} used")
        }
        None => tf!("已使用 {used}", "{used} used"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdirect::api::{Capabilities, ShortcutDetails};

    fn folder(id: &str, name: &str) -> File {
        File {
            id: id.into(),
            name: name.into(),
            mime_type: FOLDER.into(),
            ..File::default()
        }
    }

    fn writable(file: File, can: bool) -> File {
        File {
            capabilities: Some(Capabilities {
                can_add_children: can,
                ..Capabilities::default()
            }),
            ..file
        }
    }

    #[test]
    fn breadcrumbs_start_from_the_place() {
        let _language = crate::i18n::LANGUAGE_LOCK.lock().unwrap();
        // The place's name is in the interface language, which other tests
        // switch; compare with it rather than with either language.
        let shared = Place::Shared.name();
        let mut here = Location::new(Place::Shared);
        assert_eq!(here.crumbs(), [shared.as_str()]);
        assert_eq!(here.folder(), None);
        here.enter(&folder("a", "Team"));
        here.enter(&folder("b", "Q3"));
        assert_eq!(here.crumbs(), [shared.as_str(), "Team", "Q3"]);
        assert_eq!(here.folder(), Some("b"));
        here.back_to(1);
        assert_eq!(here.crumbs(), [shared.as_str(), "Team"]);
        here.back_to(0);
        assert_eq!(here.folder(), None);
        assert!(here.flat());

        let drive = Location::new(Place::Drive {
            id: "0AD".into(),
            name: "Design".into(),
        });
        assert_eq!(drive.crumbs(), ["Design"]);
        assert_eq!(drive.folder(), Some("0AD"));
        assert_eq!(drive.query(), Query::folder("0AD", Some("0AD")));
        assert_eq!(Location::new(Place::MyDrive).folder(), Some("root"));
        let search = Place::Search(SearchSpec::default());
        assert_eq!(Location::new(search.clone()).crumbs(), [search.name()]);
    }

    #[test]
    fn a_shared_drive_folder_lists_in_its_drive() {
        let mut here = Location::new(Place::Starred);
        let mut team = folder("f", "Team");
        team.drive_id = Some("0AD".into());
        here.enter(&team);
        assert_eq!(here.query(), Query::folder("f", Some("0AD")));
        here.back_to(0);
        assert_eq!(here.query(), Query::starred());
        assert_eq!(here.drive, None);
    }

    #[test]
    fn writing_follows_the_place_and_the_folder() {
        assert!(Location::new(Place::MyDrive).writable());
        for place in [
            Place::Shared,
            Place::Starred,
            Place::Recent,
            Place::Trash,
            Place::Search(SearchSpec::default()),
        ] {
            assert!(!Location::new(place).writable());
        }
        let mut shared = Location::new(Place::Shared);
        shared.enter(&writable(folder("a", "Read only"), false));
        assert!(!shared.writable());
        shared.back_to(0);
        shared.enter(&writable(folder("b", "Editor"), true));
        assert!(shared.writable());
        // Unknown capabilities: My Drive folders are the user's own.
        let mut mine = Location::new(Place::MyDrive);
        mine.enter(&folder("c", "Old cache"));
        assert!(mine.writable());
        let mut recent = Location::new(Place::Recent);
        recent.enter(&folder("c", "Old cache"));
        assert!(!recent.writable());
    }

    #[test]
    fn capabilities_gate_actions() {
        let mut file = File {
            id: "x".into(),
            name: "a.pdf".into(),
            mime_type: "application/pdf".into(),
            web_view_link: Some("https://drive.google.com/x".into()),
            ..File::default()
        };
        // No capabilities: an older cache, everything allowed.
        let all = Actions::of(&file, false);
        assert!(all.download && all.rename && all.trash && all.share && all.copy);
        assert!(!all.restore && !all.delete);

        file.capabilities = Some(Capabilities {
            can_download: true,
            can_copy: true,
            ..Capabilities::default()
        });
        let viewer = Actions::of(&file, false);
        assert!(viewer.download && viewer.copy && viewer.link && viewer.star);
        assert!(!viewer.rename && !viewer.trash && !viewer.cut && !viewer.share);

        let trashed = Actions::of(&file, true);
        assert_eq!(
            trashed,
            Actions {
                restore: false,
                delete: false,
                ..Actions::default()
            }
        );
        file.capabilities = Some(Capabilities {
            can_untrash: true,
            can_delete: true,
            ..Capabilities::default()
        });
        let trashed = Actions::of(&file, true);
        assert!(trashed.restore && trashed.delete && !trashed.open && !trashed.download);

        // Several files allow what each allows.
        let mine = File {
            capabilities: None,
            ..file.clone()
        };
        let viewer = File {
            capabilities: Some(Capabilities::default()),
            ..file.clone()
        };
        let both = Actions::all([&mine, &viewer], false);
        assert!(!both.rename && both.star);
    }

    #[test]
    fn shortcuts_act_on_their_target() {
        let mut file = File {
            id: "s".into(),
            name: "Link".into(),
            mime_type: SHORTCUT.into(),
            shortcut_details: Some(ShortcutDetails {
                target_id: "t".into(),
                target_mime_type: Some(FOLDER.into()),
            }),
            ..File::default()
        };
        assert_eq!(shortcut(&file), Some(("t", true)));
        assert!(!native(&file));
        assert!(!Actions::of(&file, false).download);
        file.shortcut_details.as_mut().unwrap().target_mime_type =
            Some("application/vnd.google-apps.document".into());
        assert_eq!(shortcut(&file), Some(("t", false)));
        assert!(Actions::of(&file, false).download);
        let doc = File {
            mime_type: "application/vnd.google-apps.document".into(),
            ..File::default()
        };
        assert!(native(&doc));
        assert!(!native(&folder("f", "F")));
    }

    #[test]
    fn chips_build_the_search() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let plain = Filters::default().spec("  report ", now);
        assert_eq!(
            plain,
            SearchSpec {
                text: "report".into(),
                ..SearchSpec::default()
            }
        );
        let filters = Filters {
            kind: Some(SearchKind::Pdf),
            owner: Owner::NotMe,
            modified: Modified::Week,
            full_text: true,
        };
        assert!(filters.any());
        assert!(!Filters::default().any());
        let spec = filters.spec("q", now);
        assert_eq!(spec.kind, Some(SearchKind::Pdf));
        assert_eq!(spec.owner, Owner::NotMe);
        assert!(spec.full_text);
        assert_eq!(spec.modified_after.as_deref(), Some("2026-11-01T12:00:00"));
        let year = Filters {
            modified: Modified::Year,
            ..Filters::default()
        };
        assert_eq!(
            year.spec("", now).modified_after.as_deref(),
            Some("2025-11-08T12:00:00")
        );
    }

    #[test]
    fn folders_open_in_their_real_place() {
        let _language = crate::i18n::LANGUAGE_LOCK.lock().unwrap();
        let (my_drive, shared_name) = (Place::MyDrive.name(), Place::Shared.name());
        let drives = [SharedDrive {
            id: "0AD".into(),
            name: "Design".into(),
        }];
        let chain = |ids: &[(&str, &str)]| {
            Some(
                ids.iter()
                    .map(|(a, b)| (a.to_string(), b.to_string()))
                    .collect::<Vec<_>>(),
            )
        };
        let target = writable(folder("t", "Target"), true);

        let mine = located(
            &target,
            chain(&[("0AR", "My Drive"), ("p", "Projects")]),
            Some("0AR"),
            &drives,
        );
        assert_eq!(mine.place, Place::MyDrive);
        assert_eq!(mine.crumbs(), [my_drive.as_str(), "Projects", "Target"]);
        assert_eq!(mine.path[1].writable, Some(true));

        let mut in_drive = target.clone();
        in_drive.drive_id = Some("0AD".into());
        let drive = located(
            &in_drive,
            chain(&[("0AD", "Drive"), ("p", "Specs")]),
            Some("0AR"),
            &drives,
        );
        assert_eq!(drive.crumbs(), ["Design", "Specs", "Target"]);
        assert_eq!(drive.query(), Query::folder("t", Some("0AD")));

        let shared = located(&target, chain(&[("o", "Team")]), Some("0AR"), &drives);
        assert_eq!(shared.place, Place::Shared);
        assert_eq!(shared.crumbs(), [shared_name.as_str(), "Team", "Target"]);

        let unknown = located(&target, None, Some("0AR"), &drives);
        assert_eq!(unknown.crumbs(), [shared_name.as_str(), "Target"]);
    }

    #[test]
    fn rail_values_round_trip() {
        let drives = [SharedDrive {
            id: "0AD".into(),
            name: "Design".into(),
        }];
        for place in [
            Place::MyDrive,
            Place::Shared,
            Place::Starred,
            Place::Recent,
            Place::Trash,
            Place::Drive {
                id: "0AD".into(),
                name: "Design".into(),
            },
        ] {
            assert_eq!(Place::from_value(&place.value(), &drives), Some(place));
        }
        assert_eq!(Place::from_value("drive:gone", &drives), None);
        assert_eq!(Place::from_value("files", &drives), None);
    }
}
