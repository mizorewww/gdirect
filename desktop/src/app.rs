//! The window's one view: state shared by the pages, the channel background
//! threads report through, navigation, theme and dialogs.
use crate::i18n::t;
use crate::tf;
use crate::{
    settings::{Settings, Theme},
    transfers::{self, Queues},
    ui,
};
use gdirect::{
    api::{Api, File},
    route,
    store::{Endpoint, Store},
};
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::color::dynamic::{SeedScheme, Variant};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use gpui_kit_material::theme;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

/// Work finished on another thread, applied to the view on the UI thread.
pub type Update = Box<dyn FnOnce(&mut Gdirect, &mut Context<Gdirect>) + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Files,
    Transfers,
    Endpoints,
    Settings,
}

impl Page {
    fn value(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Transfers => "transfers",
            Self::Endpoints => "endpoints",
            Self::Settings => "settings",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Account,
    Transfer,
    Network,
    Mount,
    Appearance,
    About,
}

pub enum Dialog {
    NewFolder,
    Rename(File),
    Trash(Vec<File>),
    Logout(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Modified,
    Size,
}

/// Progress of the endpoint scan in flight.
#[derive(Default)]
pub struct ScanState {
    pub probed: usize,
    pub open: usize,
    pub good: usize,
    pub target: usize,
    pub ipv6: bool,
    /// Verifying a candidate file of this many addresses.
    pub candidates: Option<usize>,
}

/// A recorded background mount.
pub struct MountInfo {
    pub path: PathBuf,
}

/// The accent colors the appearance pane offers: seed and name.
pub const ACCENTS: [(u32, AccentName); 5] = [
    (0x415F91, AccentName("蓝色", "Blue")),
    (0x006A65, AccentName("青色", "Teal")),
    (0x4C662B, AccentName("绿色", "Green")),
    (0x8F4C38, AccentName("陶土色", "Terracotta")),
    (0x65558F, AccentName("紫色", "Purple")),
];

/// An accent's name in Chinese and English; it turns into the text for the
/// current language.
#[derive(Clone, Copy)]
pub struct AccentName(pub &'static str, pub &'static str);

impl From<AccentName> for SharedString {
    fn from(name: AccentName) -> Self {
        t(name.0, name.1).into()
    }
}

pub struct Gdirect {
    pub db_path: PathBuf,
    pub db: Store,
    pub settings: Settings,
    pub accounts: Vec<String>,
    pub account: Option<String>,
    pub oauth_client: bool,
    /// The Drive client on the chosen route; None until connected.
    pub api: Option<Api>,
    pub connecting: bool,
    pub connect_error: Option<String>,
    pub signing_in: bool,
    pub page: Page,
    pub toast: Option<SharedString>,
    toast_serial: u64,
    tx: Sender<Update>,
    _poll: Task<()>,
    /// The language the window was last drawn in; see [`Gdirect::follow_language`].
    drawn_chinese: bool,
    theme_frame_pending: bool,
    _subs: Vec<Subscription>,

    // Files
    pub path: Vec<(String, String)>,
    pub entries: Vec<File>,
    pub loading: bool,
    pub list_error: Option<String>,
    pub filter_input: Entity<InputState>,
    pub filter: String,
    pub selected: Vec<String>,
    pub grid: bool,
    pub sort: (SortKey, bool),
    pub clipboard: Option<(Vec<File>, bool)>,
    pub busy: usize,
    pub new_menu: bool,
    pub account_menu: bool,
    pub row_menu: Option<String>,
    pub dialog: Option<Dialog>,
    pub name_input: Entity<InputState>,
    /// The filter field shows text that no longer applies; empty it.
    pub clear_filter: bool,
    /// The dialog's name field takes focus once the dialog is up.
    focus_name: bool,
    /// Bumped by each connection attempt; results from older ones are dropped.
    pub session: u64,
    /// The window's width in logical pixels at the last frame; pages lay out
    /// for it (see [`Gdirect::wide`]).
    pub width: f32,
    pub rows: UniformListScrollHandle,

    // Transfers
    pub queues: Queues,
    pub show_uploads: bool,
    /// Folders being listed for download: name and files found so far.
    pub listing: Vec<(String, usize)>,

    // Endpoints
    pub endpoints: Vec<Endpoint>,
    pub family: SharedString,
    pub scan: Option<(Arc<AtomicBool>, ScanState)>,
    pub scan_error: Option<String>,
    pub intercepted: bool,

    // Settings
    pub pane: Pane,
    /// A narrow window shows the settings list or one pane; this is the pane.
    pub pane_open: bool,
    pub mount: Option<MountInfo>,
    pub mount_busy: bool,
    pub connections: Entity<SliderState>,
}

impl Gdirect {
    pub fn new(db_path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let db = Store::open(&db_path).expect("open gdirect's state database");
        let settings = Settings::load(&db);
        let (tx, rx) = mpsc::channel::<Update>();
        let poll = cx.spawn(async move |this, cx| poll(rx, this, cx).await);

        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("按名称筛选", "Filter by name")));
        let name_input = cx.new(|cx| InputState::new(window, cx));
        let connections = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(16.)
                .step(1.)
                .default_value(settings.connections as f32)
        });
        let subs = vec![
            cx.subscribe_in(
                &filter_input,
                window,
                |this, state, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        this.filter = state.read(cx).value().to_string();
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &name_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.confirm_dialog(window, cx);
                    }
                },
            ),
            cx.subscribe(&connections, |this, _, event: &SliderEvent, cx| {
                let value = match event {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                let n = (value.round() as u8).clamp(1, 16);
                if n != this.settings.connections {
                    this.settings.connections = n;
                    this.settings.save(&this.db);
                    cx.notify();
                }
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.settings.theme == Theme::System {
                    this.apply_theme(window, cx);
                }
            }),
        ];

        let mut this = Self {
            db_path,
            settings,
            accounts: Vec::new(),
            account: None,
            oauth_client: false,
            api: None,
            connecting: false,
            connect_error: None,
            signing_in: false,
            page: Page::Files,
            toast: None,
            toast_serial: 0,
            tx,
            _poll: poll,
            drawn_chinese: crate::i18n::chinese(),
            theme_frame_pending: false,
            _subs: subs,
            path: vec![("root".into(), t("我的云盘", "My Drive").into())],
            entries: Vec::new(),
            loading: false,
            list_error: None,
            filter_input,
            filter: String::new(),
            selected: Vec::new(),
            grid: false,
            sort: (SortKey::Name, true),
            clipboard: None,
            busy: 0,
            new_menu: false,
            account_menu: false,
            row_menu: None,
            dialog: None,
            name_input,
            focus_name: false,
            clear_filter: false,
            session: 0,
            width: 1280.,
            rows: UniformListScrollHandle::new(),
            queues: Queues::default(),
            show_uploads: false,
            listing: Vec::new(),
            endpoints: Vec::new(),
            family: "all".into(),
            scan: None,
            scan_error: None,
            intercepted: false,
            pane: Pane::Transfer,
            pane_open: false,
            mount: None,
            mount_busy: false,
            connections,
            db,
        };
        this.reload_accounts();
        this.reload_endpoints();
        this.connect(cx);
        this
    }

    /// Runs `work` on its own thread and applies `done` to the view afterwards.
    pub fn background<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + Send + 'static,
    ) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = work();
            let _ = tx.send(Box::new(move |this, cx| done(this, result, cx)));
        });
    }

    pub fn sender(&self) -> Sender<Update> {
        self.tx.clone()
    }

    pub fn say(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast = Some(message.into());
        self.toast_serial += 1;
        let serial = self.toast_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |this, cx| {
                if this.toast_serial == serial {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn reload_accounts(&mut self) {
        self.accounts = self.db.accounts().unwrap_or_default();
        self.account = self.db.default_account().ok().flatten();
        self.oauth_client = self.db.setting("oauth_client").ok().flatten().is_some();
        self.mount = self.account.as_ref().and_then(|account| {
            let raw = self.db.setting(&Store::mount_key(account)).ok().flatten()?;
            let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
            Some(MountInfo {
                path: PathBuf::from(value["path"].as_str()?),
            })
        });
    }

    pub fn reload_endpoints(&mut self) {
        let google = gdirect::ranges::current(&self.db);
        self.endpoints = self
            .db
            .endpoints()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.ip.parse().is_ok_and(|ip| google.contains(ip)))
            .collect();
    }

    /// The rail's destinations, in the current language.
    fn nav_items() -> Vec<NavItem> {
        vec![
            NavItem::new("files", t("文件", "Files"), NavIcon::symbol(Symbol::FOLDER)),
            NavItem::new(
                "transfers",
                t("传输", "Transfers"),
                NavIcon::symbol(Symbol::SWAP_VERT),
            ),
            NavItem::new(
                "endpoints",
                t("线路", "Routes"),
                NavIcon::symbol(Symbol::NETWORK_PING),
            ),
            NavItem::new(
                "settings",
                t("设置", "Settings"),
                NavIcon::symbol(Symbol::SETTINGS),
            ),
        ]
    }

    /// Text held by components (rather than drawn from `t` each frame)
    /// follows a language switch.
    fn follow_language(&mut self, window: &mut Window) {
        let chinese = crate::i18n::chinese();
        if std::mem::replace(&mut self.drawn_chinese, chinese) == chinese {
            return;
        }
        let input = self.filter_input.clone();
        window.on_next_frame(move |window, cx| {
            input.update(cx, |input, cx| {
                input.set_placeholder(t("按名称筛选", "Filter by name"), window, cx)
            });
        });
    }

    /// Whether the window is at least `px` logical pixels wide.
    pub fn wide(&self, px: f32) -> bool {
        self.width >= px
    }

    /// Whether a front end verified in the last day is stored to connect through.
    pub fn has_route(&self) -> bool {
        let google = gdirect::ranges::current(&self.db);
        self.db
            .routes(16)
            .is_ok_and(|routes| !gdirect::scan::usable(routes, &google).is_empty())
    }

    /// Picks the route and builds the Drive client for the default account,
    /// then lists the current folder.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        // Results of earlier attempts, and listings made through them, no
        // longer apply.
        self.session += 1;
        let session = self.session;
        let Some(account) = self.account.clone() else {
            self.api = None;
            return;
        };
        // Without a fresh endpoint the route would scan unseen; scan in the
        // open instead, with progress and the interception warning. The scan
        // connects when it finds one.
        if self.settings.pinned.is_none() && !self.has_route() {
            self.start_scan(None, cx);
            return;
        }
        self.connecting = true;
        self.connect_error = None;
        let db_path = self.db_path.clone();
        let pinned = self.settings.pinned;
        let sni = self.settings.identity.sni();
        self.background(
            move || -> anyhow::Result<Api> {
                let db = Store::open(&db_path)?;
                let direct = route::resolve(&db, pinned, sni)?;
                let api = direct.api(db_path, Some(account))?;
                api.token()?;
                Ok(api)
            },
            move |this, result, cx| {
                if this.session != session {
                    return;
                }
                this.connecting = false;
                this.reload_endpoints();
                match result {
                    Ok(api) => {
                        this.api = Some(api);
                        this.refresh(cx);
                        this.pump(cx);
                    }
                    // A reconnect that fails keeps the working client; the
                    // listing it dropped is made again through that one.
                    Err(e) if this.api.is_some() => {
                        this.say(
                            tf!("无法切换线路：{e:#}", "Couldn’t switch routes: {e:#}"),
                            cx,
                        );
                        this.refresh(cx);
                    }
                    Err(e) => this.connect_error = Some(format!("{e:#}")),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// The front end in use and its latency.
    pub fn current_route(&self) -> Option<(String, Option<i64>, String)> {
        let (ip, sni, _) = self.api.as_ref()?.route();
        let ip = ip?.to_string();
        let endpoint = self.endpoints.iter().find(|e| e.ip == ip);
        Some((
            ip,
            endpoint.and_then(|e| e.latency_ms),
            sni.filter(|s| !s.is_empty())
                .or_else(|| endpoint.map(|e| e.sni.clone()).filter(|s| !s.is_empty()))
                .unwrap_or_default(),
        ))
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        if page == Page::Endpoints {
            self.reload_endpoints();
        }
        if page == Page::Settings {
            self.reload_accounts();
        }
        cx.notify();
    }

    // Theme

    fn target_scheme(&self, cx: &App) -> gpui_kit_material::color::Scheme {
        let dark = match self.settings.theme {
            Theme::System => theme::system_dark(cx),
            Theme::Dark => true,
            Theme::Light => false,
        };
        let seed = ACCENTS[self.settings.accent.min(ACCENTS.len() - 1)].0;
        SeedScheme::new(seed)
            .dark(dark)
            .variant(Variant::TonalSpot)
            .contrast(0.0)
            .scheme()
    }

    pub fn initial_scheme(settings: &Settings, cx: &App) -> gpui_kit_material::color::Scheme {
        let dark = match settings.theme {
            Theme::System => theme::system_dark(cx),
            Theme::Dark => true,
            Theme::Light => false,
        };
        let seed = ACCENTS[settings.accent.min(ACCENTS.len() - 1)].0;
        SeedScheme::new(seed)
            .dark(dark)
            .variant(Variant::TonalSpot)
            .contrast(0.0)
            .scheme()
    }

    pub fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = theme::advance_component_theme(self.target_scheme(cx), 1.0, window, cx);
        if running && !self.theme_frame_pending {
            self.theme_frame_pending = true;
            let view = cx.weak_entity();
            window.on_next_frame(move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.theme_frame_pending = false;
                    this.apply_theme(window, cx);
                });
            });
        }
    }

    // Dialogs

    pub fn open_dialog(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        let text = match &dialog {
            Dialog::NewFolder => Some(String::new()),
            Dialog::Rename(file) => Some(file.name.clone()),
            _ => None,
        };
        if let Some(text) = text {
            self.name_input.update(cx, |input, cx| {
                input.set_value(text, window, cx);
            });
            self.focus_name = true;
        }
        self.dialog = Some(dialog);
        cx.notify();
    }

    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        self.dialog = None;
        cx.notify();
    }

    pub fn confirm_dialog(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).value().trim().to_string();
        // A name dialog stays open until it has a name to act on.
        if name.is_empty() && matches!(self.dialog, Some(Dialog::NewFolder | Dialog::Rename(_))) {
            return;
        }
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match dialog {
            Dialog::NewFolder if !name.is_empty() => self.create_folder(name, cx),
            Dialog::Rename(file) if !name.is_empty() && name != file.name => {
                self.rename(file, name, cx)
            }
            Dialog::Trash(files) => self.trash(files, cx),
            Dialog::Logout(account) => self.logout(account, cx),
            _ => {}
        }
        cx.notify();
    }

    fn render_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dialog moves focus to its surface after its first frame; the
        // field takes it from there on the frame after.
        if std::mem::take(&mut self.focus_name) {
            let input = self.name_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            });
        }
        let (headline, supporting, confirm, input, danger): (
            SharedString,
            Option<SharedString>,
            &str,
            bool,
            bool,
        ) = match &self.dialog {
            Some(Dialog::NewFolder) => (
                t("新建文件夹", "New folder").into(),
                None,
                t("创建", "Create"),
                true,
                false,
            ),
            Some(Dialog::Rename(file)) => (
                t("重命名", "Rename").into(),
                Some(format!("“{}”", file.name).into()),
                t("重命名", "Rename"),
                true,
                false,
            ),
            Some(Dialog::Trash(files)) => (
                t("移至回收站？", "Move to trash?").into(),
                Some(
                    if files.len() == 1 {
                        tf!(
                            "“{}”会移到 Google 云盘的回收站，30 天内可以恢复。",
                            "“{}” will be moved to the Google Drive trash. You can restore it within 30 days.",
                            files[0].name
                        )
                    } else {
                        tf!(
                            "{} 项会移到 Google 云盘的回收站，30 天内可以恢复。",
                            "{} items will be moved to the Google Drive trash. You can restore them within 30 days.",
                            files.len()
                        )
                    }
                    .into(),
                ),
                t("移至回收站", "Move to trash"),
                false,
                true,
            ),
            Some(Dialog::Logout(account)) => (
                t("退出账号？", "Sign out?").into(),
                Some(
                    tf!(
                        "将删除 {account} 保存的凭据、访问令牌和上传会话。已扫描的线路会保留。",
                        "This removes the saved credentials, access tokens and upload sessions for {account}. Scanned routes are kept."
                    )
                    .into(),
                ),
                t("退出", "Sign out"),
                false,
                true,
            ),
            None => (SharedString::default(), None, "", false, false),
        };
        let view = cx.weak_entity();
        MaterialDialog::new("dialog")
            .modal(self.dialog.is_some())
            .size(DialogSize::Small)
            .headline(headline)
            .when_some(supporting, |d, s| d.supporting_text(s))
            .when(input, |d| {
                d.content(
                    div().pt(rem(8.)).child(
                        TextField::new("dialog-name", &self.name_input)
                            .variant(TextFieldVariant::Outlined)
                            .label(t("名称", "Name"))
                            .w_full(),
                    ),
                )
            })
            .action(
                material_button("dialog-cancel", ButtonVariant::Text)
                    .child(t("取消", "Cancel"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_dialog(cx))),
            )
            .action(
                material_button(
                    "dialog-confirm",
                    if danger {
                        ButtonVariant::Filled
                    } else {
                        ButtonVariant::Text
                    },
                )
                .when(danger, |b| b.tone(ButtonTone::Accent))
                .child(confirm.to_owned())
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_dialog(window, cx)),
                ),
            )
            .on_dismiss(move |_, cx| {
                let _ = view.update(cx, |this, cx| this.close_dialog(cx));
            })
    }

    // Transfers

    /// Starts the next queued download and upload when their lanes are free.
    pub fn pump(&mut self, _cx: &mut Context<Self>) {
        let (Some(api), Some(account)) = (self.api.clone(), self.account.clone()) else {
            return;
        };
        let options = transfers::Options {
            connections: self.settings.connections,
            md5: self.settings.md5,
            sni: match self.settings.identity {
                crate::settings::Identity::Auto => None,
                identity => identity.sni(),
            },
            ca_cert: self.settings.ca_cert.clone(),
        };
        if let Some(job) = self.queues.next_download(&account) {
            let id = job.id;
            let cancel = Arc::new(AtomicBool::new(false));
            job.cancel = Some(cancel.clone());
            let api = api.clone();
            let snapshot = transfers::Download {
                id,
                account: account.clone(),
                file: job.file.clone(),
                output: job.output.clone(),
                export: job.export.clone(),
                size: job.size,
                state: job.state.clone(),
                stats: Default::default(),
                started: None,
                cancel: None,
            };
            let tx = self.sender();
            let (progress_tx, progress_rx) = mpsc::channel();
            // Forward live statistics, coalesced, to the view.
            let forward = tx.clone();
            std::thread::spawn(move || {
                while let Ok(mut stats) = progress_rx.recv() {
                    while let Ok(newer) = progress_rx.try_recv() {
                        stats = newer;
                    }
                    let update: Update = Box::new(move |this, cx| {
                        if let Some(job) = this.queues.download(id) {
                            job.stats = stats;
                            cx.notify();
                        }
                    });
                    if forward.send(update).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
            self.background(
                move || transfers::run_download(&api, &snapshot, &options, cancel, progress_tx),
                move |this, result, cx| {
                    if let Some(job) = this.queues.download(id) {
                        let cancelled = job
                            .cancel
                            .take()
                            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
                        job.state = match result {
                            Ok((note, output)) => {
                                job.output = output;
                                transfers::State::Done {
                                    at: chrono::Local::now(),
                                    note,
                                }
                            }
                            Err(_) if cancelled => transfers::State::Paused,
                            Err(e) => transfers::State::Failed(transfers::explain(&e)),
                        };
                    }
                    this.pump(cx);
                    cx.notify();
                },
            );
        }
        if let Some(job) = self.queues.next_upload(&account) {
            let id = job.id;
            let path = job.path.clone();
            let parent = job.parent.0.clone();
            let folder = job.folder;
            let cancel = Arc::new(AtomicBool::new(false));
            job.cancel = Some(cancel.clone());
            let api = api.cancellable(cancel);
            let tx = self.sender();
            self.background(
                move || -> anyhow::Result<(String, String)> {
                    let report = |progress: gdirect::upload::TreeProgress| {
                        let _ = tx.send(Box::new(move |this: &mut Gdirect, cx| {
                            if let Some(job) = this.queues.upload(id) {
                                job.sample(progress);
                                cx.notify();
                            }
                        }));
                    };
                    if folder {
                        let result = api.upload_dir(&path, &parent, None, false, report)?;
                        Ok((result.id.clone(), transfers::folder_summary(&result)))
                    } else {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let file = api.upload_with(&path, &parent, None, |done, total| {
                            report(gdirect::upload::TreeProgress {
                                path: name.clone(),
                                files_done: 0,
                                files_total: 1,
                                bytes_done: done,
                                bytes_total: total,
                                file_done: done,
                                file_total: total,
                            })
                        })?;
                        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
                        Ok((
                            file.id,
                            tf!("{} · 新建文件", "{} · new file", crate::format::bytes(size)),
                        ))
                    }
                },
                move |this, result, cx| {
                    let parent = this.queues.upload(id).map(|u| u.parent.0.clone());
                    if let Some(job) = this.queues.upload(id) {
                        let cancelled = job
                            .cancel
                            .take()
                            .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
                        match result {
                            Ok((file_id, note)) => {
                                job.result = Some(file_id);
                                job.state = transfers::State::Done {
                                    at: chrono::Local::now(),
                                    note,
                                };
                            }
                            Err(_) if cancelled => job.state = transfers::State::Paused,
                            Err(e) => job.state = transfers::State::Failed(transfers::explain(&e)),
                        }
                    }
                    // The folder it went to may be on screen.
                    if parent.as_deref() == Some(this.folder()) {
                        this.refresh(cx);
                    }
                    this.pump(cx);
                    cx.notify();
                },
            );
        }
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = self.queues.pending();
        let items = Self::nav_items().into_iter().map(|item| {
            if item.value() == "transfers" && pending > 0 {
                item.badge(NavBadge::Large(pending.to_string().into()))
            } else {
                item
            }
        });
        let signed_in = self.account.is_some();
        let fab = Fab::new("new", Symbol::ADD, t("新建", "New"))
            .color(FabColor::PrimaryContainer)
            .disabled(!signed_in || self.api.is_none())
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    this.new_menu = !this.new_menu;
                    cx.notify();
                }
            }));
        let menu = MaterialMenu::new("new-menu")
            .width(220.)
            .item(
                MaterialMenuItem::new(t("新建文件夹", "New folder"))
                    .leading(ui::symbol("create_new_folder", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.new_menu = false;
                        this.go(Page::Files, cx);
                        this.open_dialog(Dialog::NewFolder, window, cx);
                    })),
            )
            .item(
                MaterialMenuItem::new(t("上传文件…", "Upload files…"))
                    .leading(ui::symbol("upload_file", false))
                    .divider(false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.new_menu = false;
                        this.choose_upload(false, cx);
                    })),
            )
            .item(
                MaterialMenuItem::new(t("上传文件夹…", "Upload folder…"))
                    .leading(ui::symbol("drive_folder_upload", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.new_menu = false;
                        this.choose_upload(true, cx);
                    })),
            );
        NavigationRail::new("rail", t("主导航", "Main navigation"))
            .container(ColorRole::SURFACE_CONTAINER)
            .items(items)
            .selected(self.page.value())
            .fab(
                MenuPopup::new("new-popup", fab, menu)
                    .open(self.new_menu)
                    .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                        // The menu ignores its trigger's disabled state.
                        this.new_menu = *open && this.api.is_some();
                        cx.notify();
                    })),
            )
            .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                let page = match value.as_ref() {
                    "transfers" => Page::Transfers,
                    "endpoints" => Page::Endpoints,
                    "settings" => Page::Settings,
                    _ => Page::Files,
                };
                this.go(page, cx);
            }))
    }
}

/// Drains the background threads' updates every few frames.
async fn poll(rx: Receiver<Update>, this: WeakEntity<Gdirect>, cx: &mut AsyncApp) {
    loop {
        cx.background_executor()
            .timer(Duration::from_millis(50))
            .await;
        let batch: Vec<Update> = rx.try_iter().collect();
        if batch.is_empty() {
            if this.upgrade().is_none() {
                break;
            }
            continue;
        }
        let alive = this
            .update(cx, |this, cx| {
                for update in batch {
                    update(this, cx);
                }
                cx.notify();
            })
            .is_ok();
        if !alive {
            break;
        }
    }
}

pub const TITLE_BAR: Pixels = px(40.);

impl Render for Gdirect {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.width = f32::from(window.viewport_size().width);
        self.follow_language(window);
        let labels = CaptionLabels {
            minimize: t("最小化", "Minimize").into(),
            maximize: t("最大化", "Maximize").into(),
            restore: t("还原", "Restore").into(),
            close: t("关闭", "Close").into(),
        };
        let content = if self.accounts.is_empty() {
            self.render_welcome(cx).into_any_element()
        } else {
            match self.page {
                Page::Files => self.render_files(window, cx).into_any_element(),
                Page::Transfers => self.render_transfers(cx).into_any_element(),
                Page::Endpoints => self.render_endpoints(cx).into_any_element(),
                Page::Settings => self.render_settings(cx).into_any_element(),
            }
        };
        // Mounted only while open, so it never animates in from stale content.
        let dialog = self
            .dialog
            .is_some()
            .then(|| self.render_dialog(window, cx).into_any_element());
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family("Roboto")
            .bg(color(ColorRole::SURFACE_CONTAINER))
            .text_color(color(ColorRole::ON_SURFACE))
            .child(
                TitleBar::new("title-bar", TITLE_BAR, labels)
                    .bg(color(ColorRole::SURFACE_CONTAINER))
                    .child(
                        ui::text(
                            TypeRole::TitleSmall,
                            ColorRole::ON_SURFACE_VARIANT,
                            "gdirect",
                        )
                        .flex_1(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_rail(cx))
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .mr(rem(8.))
                            .mb(rem(8.))
                            .rounded(rem(24.))
                            .bg(color(ColorRole::SURFACE))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(content)
                            .when_some(self.toast.clone(), |d, message| {
                                let view = cx.weak_entity();
                                d.child(
                                    Snackbar::new("toast", message)
                                        .position(SnackbarPosition::Center)
                                        .on_dismiss(move |_, cx| {
                                            let _ = view.update(cx, |this, cx| {
                                                this.toast = None;
                                                cx.notify();
                                            });
                                        }),
                                )
                            }),
                    ),
            )
            .children(dialog)
    }
}
