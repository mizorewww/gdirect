//! The window's one view: state shared by the pages, the channel background
//! threads report through, navigation, theme and dialogs.
use crate::i18n::t;
use crate::tf;
use crate::{
    client_form::{ClientForm, ClientFormEvent},
    files::Undoable,
    keymap,
    places::{Filters, Location, Place},
    settings::{Settings, Theme},
    transfers::{self, Queues},
    ui,
};
use gdirect::{
    api::{About, Api, File, SharedDrive},
    route,
    store::{Endpoint, Store},
};
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::color::dynamic::{SeedScheme, Variant};
use gpui_kit_material::prelude::*;
use gpui_kit_material::theme;
use std::{
    collections::{HashMap, HashSet},
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
    pub fn value(self) -> &'static str {
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
    Shortcuts,
    About,
}

pub enum Dialog {
    NewFolder,
    Rename(Box<File>),
    DeleteForever(Vec<File>),
    EmptyTrash,
    Logout(String),
    /// The OAuth client form: to sign in (another account), or to change
    /// the saved client without signing in.
    Client {
        sign_in: bool,
    },
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
    /// The client the next sign-in uses.
    pub oauth_client: Option<gdirect::auth::Client>,
    /// The welcome page's client form.
    pub client_form: Entity<ClientForm>,
    /// The account dialog's, apart from the welcome page's.
    pub dialog_form: Entity<ClientForm>,
    /// The account dialog shows the saved client it signs in with, not the
    /// form, until Change… opens the form.
    pub client_summary: bool,
    /// Set to stop waiting for the browser sign-in in flight.
    pub login_cancel: Option<Arc<AtomicBool>>,
    /// Bumped by each sign-in, and when one is cancelled or dropped; what an
    /// older one comes to is dropped.
    pub login_serial: u64,
    /// The waiting sign-in's page, to open again.
    pub login_url: Option<String>,
    /// Why the last sign-in failed, shown on the welcome page or in the dialog.
    pub login_error: Option<SharedString>,
    /// The dialog's scrolling body; its dividers show when it scrolls.
    dialog_scroll: ScrollHandle,
    dialog_overflow: bool,
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
    pub location: Location,
    pub entries: Vec<File>,
    pub loading: bool,
    /// Bumped by each listing; pages of an older one are dropped.
    pub generation: u64,
    /// The listing's next page, fetched as the list nears its end.
    pub next_page: Option<String>,
    pub loading_more: bool,
    pub list_error: Option<String>,
    /// Filters the folder on screen; the search field searches all of Drive.
    pub filter_input: Entity<InputState>,
    pub filter: String,
    pub search_input: Entity<InputState>,
    /// Bumped by each edit of the search; it runs once typing pauses on the latest.
    pub search_serial: u64,
    pub filters: Filters,
    /// Where a search started, as it was shown (its sort), to return to
    /// when it is cleared.
    pub before_search: Option<crate::files::Visit>,
    /// The search chip whose menu is open.
    pub chip_menu: Option<&'static str>,
    pub focus_search: bool,
    /// The folder filter is open: a narrow window shows the field, not its button.
    pub filter_open: bool,
    /// The filter field takes focus after the next frame.
    pub focus_filter: bool,
    /// The search field shows a search that no longer applies; empty it.
    pub clear_search: bool,
    /// The search field is to show this search again (Back to a search).
    pub restore_search: Option<String>,
    /// The last trash or move, which the snackbar offers to undo.
    pub undo: Option<Undoable>,
    /// Read once per sign-in: Shared drives, the storage quota, and My
    /// Drive's real ID (Drive reports parents by it, not by "root").
    pub drives: Vec<SharedDrive>,
    pub about: Option<About>,
    pub root_id: Option<String>,
    /// Folder names for the Location column by ID, and those being read.
    pub folder_names: HashMap<String, String>,
    pub naming: HashSet<String>,
    pub selected: Vec<String>,
    pub grid: bool,
    pub sort: (SortKey, bool),
    pub clipboard: Option<(Vec<File>, bool)>,
    pub busy: usize,
    pub new_menu: bool,
    pub account_menu: bool,
    pub row_menu: Option<String>,
    /// The file whose right-click menu is open, highlighted meanwhile.
    pub context_row: Option<String>,
    pub dialog: Option<Dialog>,
    pub name_input: Entity<InputState>,
    /// The filter field shows text that no longer applies; empty it.
    pub clear_filter: bool,
    /// The dialog's name field takes focus once the dialog is up, with this
    /// many bytes of its name (the name less its extension) selected.
    focus_name: Option<usize>,
    /// A question that destroys something opens with Cancel focused, so
    /// Enter answers it safely.
    focus_cancel: bool,
    cancel_focus: FocusHandle,
    /// Bumped by each connection attempt; results from older ones are dropped.
    pub session: u64,
    /// The window's width in logical pixels at the last frame; pages lay out
    /// for it (see [`Gdirect::wide`]).
    pub width: f32,
    /// The window's height in logical pixels at the last frame.
    pub height: f32,
    pub rows: UniformListScrollHandle,
    /// The grid's and the Routes page's scrolling, for the dividers under
    /// the bars they scroll beneath.
    pub grid_scroll: ScrollHandle,
    pub routes_scroll: ScrollHandle,
    /// The settings index's scrolling, kept while a pane is shown alone so
    /// Back returns to the item it came from.
    pub settings_scroll: ScrollHandle,
    /// The settings pane's scrolling, for the divider under its header,
    /// and the pane it belongs to (none while no pane is shown).
    pub pane_scroll: ScrollHandle,
    pub pane_scrolled: Option<Pane>,
    /// The window's focus when nothing in it has any; its key context holds
    /// the global shortcuts.
    pub focus: FocusHandle,
    /// The Files page's focus: its shortcuts apply while it or the filter has it.
    pub files_focus: FocusHandle,
    /// The page's root takes focus after its next frame, unless a text
    /// field in the window has it: set by each page change and each closed
    /// dialog, so the keys always reach the page and the menu bar.
    pub focus_page: bool,
    /// Locations left by navigating, for Back, and those Back left, for Forward.
    pub history: Vec<crate::files::Visit>,
    pub future: Vec<crate::files::Visit>,
    /// The row the arrow keys move from, and the end a range selection grows from.
    pub cursor: Option<String>,
    pub anchor: Option<String>,
    /// The cursor row was last moved by the keyboard, so it shows a focus ring.
    pub keyboard_cursor: bool,
    /// Items to select once the next listing arrives: the folder just left
    /// by going up, or items just restored.
    pub pending_select: Vec<String>,
    /// The next listing puts the cursor on its first row: a folder opened
    /// from the keyboard.
    pub cursor_first: bool,
    /// The item being opened was opened from the keyboard.
    pub keyboard_open: bool,
    /// The filter field's placeholder as last set; it names the folder or not.
    pub filter_hint: &'static str,
    pub sort_menu: bool,
    pub crumb_menu: bool,

    // Navigation
    /// The rail is expanded beside the content in a wide window (a setting).
    pub rail_expanded: bool,
    /// The expanded rail floats over a narrow window.
    pub rail_modal: bool,

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

        let filter_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(crate::files::filter_placeholder()));
        let search_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(crate::files::search_placeholder()));
        let name_input = cx.new(|cx| InputState::new(window, cx));
        let client_form = cx.new(|cx| ClientForm::new(true, window, cx));
        let dialog_form = cx.new(|cx| ClientForm::new(false, window, cx));
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
                |this, state, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => {
                        this.filter = state.read(cx).value().to_string();
                        cx.notify();
                    }
                    // An empty filter left closes back to its button.
                    InputEvent::Blur if this.filter_open && this.filter.is_empty() => {
                        this.filter_open = false;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &search_input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.search_typed(cx),
                    InputEvent::PressEnter { .. } => this.search_now(cx),
                    _ => {}
                },
            ),
            // Escape closes a settings pane shown alone, back to the list.
            // Taken ahead of the key bindings, which would match it first.
            {
                let view = cx.weak_entity();
                cx.intercept_keystrokes(move |event, _, cx| {
                    let k = &event.keystroke;
                    if k.key != "escape"
                        || k.modifiers.modified()
                        || crate::settings_page::recording_shortcut()
                    {
                        return;
                    }
                    let closed = view
                        .update(cx, |this, cx| {
                            let close = this.dialog.is_none() && this.settings_pane_alone();
                            if close {
                                this.pane_open = false;
                                cx.notify();
                            }
                            close
                        })
                        .unwrap_or(false);
                    if closed {
                        cx.stop_propagation();
                    }
                })
            },
            cx.subscribe_in(
                &name_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.confirm_dialog(window, cx);
                    }
                },
            ),
            cx.observe(&client_form, |_, _, cx| cx.notify()),
            cx.observe(&dialog_form, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &client_form,
                window,
                |this, _, event: &ClientFormEvent, window, cx| match event {
                    ClientFormEvent::Submit if this.dialog.is_none() => {
                        this.submit_client(window, cx)
                    }
                    ClientFormEvent::Submit => {}
                    ClientFormEvent::Say(message) => this.say(message.clone(), cx),
                },
            ),
            cx.subscribe_in(
                &dialog_form,
                window,
                |this, _, event: &ClientFormEvent, window, cx| match event {
                    ClientFormEvent::Submit => this.submit_client(window, cx),
                    ClientFormEvent::Say(message) => this.say(message.clone(), cx),
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
            // The safety net under every button that removes itself (a
            // field's ×, a chip, the selection bar's actions, Load more):
            // focus left on an element the frame no longer draws reaches
            // nothing, so the page takes it back. GPUI reports it only when
            // the focused element is gone from the whole window, so focus in
            // a menu or popup drawn outside this view is left alone.
            cx.on_focus_lost(window, |this, window, cx| {
                if this.dialog.is_none() {
                    this.focus_now(window, cx);
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
            oauth_client: None,
            client_form,
            dialog_form,
            client_summary: false,
            login_cancel: None,
            login_serial: 0,
            login_url: None,
            login_error: None,
            dialog_scroll: ScrollHandle::new(),
            dialog_overflow: false,
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
            location: Location::new(Place::MyDrive),
            entries: Vec::new(),
            loading: false,
            generation: 0,
            next_page: None,
            loading_more: false,
            list_error: None,
            filter_input,
            filter: String::new(),
            search_input,
            search_serial: 0,
            filters: Filters::default(),
            before_search: None,
            chip_menu: None,
            focus_search: false,
            filter_open: false,
            focus_filter: false,
            restore_search: None,
            clear_search: false,
            undo: None,
            drives: Vec::new(),
            about: None,
            root_id: None,
            folder_names: HashMap::new(),
            naming: HashSet::new(),
            selected: Vec::new(),
            grid: false,
            sort: (SortKey::Name, true),
            clipboard: None,
            busy: 0,
            new_menu: false,
            account_menu: false,
            row_menu: None,
            context_row: None,
            dialog: None,
            name_input,
            focus_name: None,
            focus_cancel: false,
            cancel_focus: cx.focus_handle(),
            clear_filter: false,
            session: 0,
            width: 1280.,
            height: 800.,
            rows: UniformListScrollHandle::new(),
            grid_scroll: ScrollHandle::new(),
            routes_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
            pane_scroll: ScrollHandle::new(),
            pane_scrolled: None,
            focus: cx.focus_handle(),
            files_focus: cx.focus_handle(),
            focus_page: true,
            history: Vec::new(),
            future: Vec::new(),
            cursor: None,
            anchor: None,
            keyboard_cursor: false,
            pending_select: Vec::new(),
            cursor_first: false,
            keyboard_open: false,
            filter_hint: "",
            sort_menu: false,
            crumb_menu: false,
            rail_expanded: db.setting(crate::nav::RAIL_KEY).ok().flatten().as_deref() != Some("0"),
            rail_modal: false,
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
        let saved = this.oauth_client.clone();
        this.client_form
            .update(cx, |form, cx| form.fill(saved.as_ref(), window, cx));
        this.reload_endpoints();
        this.connect(cx);
        // The menu bar in the language the settings chose.
        crate::keymap::set_menus(cx);
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
        self.undo = None;
        self.show_toast(message.into(), 5, cx);
    }

    /// A message whose snackbar offers to undo what it reports, for longer.
    pub fn say_undo(
        &mut self,
        message: impl Into<SharedString>,
        undo: Undoable,
        cx: &mut Context<Self>,
    ) {
        self.undo = Some(undo);
        self.show_toast(message.into(), 10, cx);
    }

    fn show_toast(&mut self, message: SharedString, seconds: u64, cx: &mut Context<Self>) {
        self.toast = Some(message);
        self.toast_serial += 1;
        let serial = self.toast_serial;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(seconds))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toast_serial == serial {
                    this.toast = None;
                    this.undo = None;
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
        self.oauth_client = gdirect::auth::saved_client(&self.db, None).ok().flatten();
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

    /// Text held by components (rather than drawn from `t` each frame)
    /// follows a language switch.
    fn follow_language(&mut self, window: &mut Window, cx: &mut App) {
        let chinese = crate::i18n::chinese();
        if std::mem::replace(&mut self.drawn_chinese, chinese) == chinese {
            return;
        }
        crate::keymap::set_menus(cx);
        // The Files page sets the filter's again, for its location.
        self.filter_hint = "";
        let search = self.search_input.clone();
        window.on_next_frame(move |window, cx| {
            search.update(cx, |input, cx| {
                input.set_placeholder(crate::files::search_placeholder(), window, cx)
            });
        });
    }

    /// Whether the window is at least `px` logical pixels wide, less what an
    /// expanded rail takes beyond the collapsed one: the breakpoints are
    /// written for the window beside the collapsed rail.
    pub fn wide(&self, px: f32) -> bool {
        self.width >= px
    }

    /// The content pane's width, beside the rail and inside its margin.
    pub fn content_width(&self) -> f32 {
        self.width - RAIL_COLLAPSED_WIDTH - 8.
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
                        this.load_places(cx);
                        this.pump(cx);
                    }
                    // A reconnect that fails keeps the working client; the
                    // listing it dropped is made again through that one.
                    Err(e) if this.api.is_some() => {
                        // The cause alone; the chain behind it is for the Routes page.
                        let e = first_line(&format!("{e:#}"));
                        this.say(tf!("无法切换线路：{e}", "Couldn’t switch routes: {e}"), cx);
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
        self.focus_page = true;
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
            Dialog::NewFolder => Some((String::new(), 0)),
            Dialog::Rename(file) => Some((file.name.clone(), name_stem(file))),
            _ => None,
        };
        if let Some((text, stem)) = text {
            self.name_input.update(cx, |input, cx| {
                input.set_value(text, window, cx);
            });
            self.focus_name = Some(stem);
        }
        if let Dialog::Client { sign_in } = dialog {
            let saved = self.oauth_client.clone();
            self.dialog_form
                .update(cx, |form, cx| form.fill(saved.as_ref(), window, cx));
            self.client_summary = sign_in && saved.is_some();
            self.login_error = None;
            self.dialog_overflow = false;
            self.dialog_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.focus_cancel = matches!(dialog, Dialog::DeleteForever(_) | Dialog::EmptyTrash);
        self.dialog = Some(dialog);
        cx.notify();
    }

    pub fn close_dialog(&mut self, cx: &mut Context<Self>) {
        // The account dialog shows its sign-in's wait; closed, it stops it.
        if let Some(Dialog::Client { .. }) = self.dialog.take()
            && self.signing_in
        {
            self.cancel_login(cx);
        }
        self.focus_page = true;
        cx.notify();
    }

    pub fn confirm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Dialog::Client { .. }) = self.dialog {
            self.submit_client(window, cx);
            return;
        }
        let name = self.name_input.read(cx).value().trim().to_string();
        // A name dialog stays open until it has a name to act on.
        if name.is_empty() && matches!(self.dialog, Some(Dialog::NewFolder | Dialog::Rename(_))) {
            return;
        }
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        self.focus_page = true;
        self.focus_now(window, cx);
        match dialog {
            Dialog::NewFolder if !name.is_empty() => self.create_folder(name, cx),
            Dialog::Rename(file) if !name.is_empty() && name != file.name => {
                self.rename(*file, name, cx)
            }
            Dialog::DeleteForever(files) => self.delete_forever(files, cx),
            Dialog::EmptyTrash => self.empty_trash(cx),
            Dialog::Logout(account) => self.logout(account, cx),
            Dialog::Client { .. } => unreachable!("submitted above"),
            _ => {}
        }
        cx.notify();
    }

    fn render_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The dialog moves its surface into focus at the end of its first
        // frame. The field (or Cancel) takes it back at once, from inside
        // the dialog, whose contents render after it: typed right after the
        // shortcut, the first keys reach the field. The next frame checks
        // again, in case the surface took it later.
        let focus_name = self.focus_name.take().map(|stem| {
            let input = self.name_input.clone();
            let focus = move |window: &mut Window, cx: &mut App| {
                input.update(cx, |input, cx| {
                    if !input.focus_handle(cx).is_focused(window) {
                        input.focus(window, cx);
                        input.set_selected_range(0..stem, cx);
                    }
                });
            };
            window.on_next_frame(focus.clone());
            ui::FocusLater::new(focus)
        });
        let focus_cancel = std::mem::take(&mut self.focus_cancel).then(|| {
            let handle = self.cancel_focus.clone();
            let focus = move |window: &mut Window, cx: &mut App| {
                if !handle.is_focused(window) {
                    window.focus(&handle, cx);
                }
            };
            window.on_next_frame(focus.clone());
            ui::FocusLater::new(focus)
        });
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
            Some(Dialog::Rename(_)) => (
                t("重命名", "Rename").into(),
                None,
                t("重命名", "Rename"),
                true,
                false,
            ),
            Some(Dialog::DeleteForever(files)) => (
                t("永久删除？", "Delete forever?").into(),
                Some(
                    if files.len() == 1 {
                        tf!(
                            "“{}”将被永久删除，无法撤销。",
                            "“{}” will be deleted forever. This can’t be undone.",
                            files[0].name
                        )
                    } else {
                        tf!(
                            "{} 项将被永久删除，无法撤销。",
                            "{} items will be deleted forever. This can’t be undone.",
                            files.len()
                        )
                    }
                    .into(),
                ),
                t("永久删除", "Delete forever"),
                false,
                true,
            ),
            Some(Dialog::EmptyTrash) => (
                t("清空回收站？", "Empty trash?").into(),
                Some(
                    t(
                        "回收站中的所有内容都将被永久删除，无法撤销。",
                        "Everything in the trash will be deleted forever. This can’t be undone.",
                    )
                    .into(),
                ),
                t("清空回收站", "Empty trash"),
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
            Some(Dialog::Client { sign_in: true }) => (
                t("添加 Google 账号", "Add a Google account").into(),
                None,
                t("使用 Google 账号登录", "Sign in with Google"),
                false,
                false,
            ),
            Some(Dialog::Client { sign_in: false }) => (
                t("更换 OAuth 客户端", "Change OAuth client").into(),
                Some(
                    t(
                        "用于之后的登录，已登录的账号不受影响。",
                        "Used for future sign-ins; signed-in accounts are unaffected.",
                    )
                    .into(),
                ),
                t("保存", "Save"),
                false,
                false,
            ),
            None => (SharedString::default(), None, "", false, false),
        };
        let client = matches!(self.dialog, Some(Dialog::Client { .. }));
        let sign_in = matches!(self.dialog, Some(Dialog::Client { sign_in: true }));
        let summary = sign_in && self.client_summary && self.oauth_client.is_some();
        let waiting = sign_in && self.signing_in;
        let client_valid = summary || self.dialog_form.read(cx).valid(cx);
        // Dividers mark a body that scrolls, once a frame has shown it does.
        if client {
            let handle = self.dialog_scroll.clone();
            let view = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let overflow = handle.max_offset().y > px(0.5);
                let _ = view.update(cx, |this, cx| {
                    if this.dialog_overflow != overflow {
                        this.dialog_overflow = overflow;
                        cx.notify();
                    }
                });
            });
        }
        let view = cx.weak_entity();
        MaterialDialog::new("dialog")
            .modal(self.dialog.is_some())
            .size(DialogSize::Small)
            // A question's text reads on fewer, fuller lines than the name
            // field needs.
            .width(if client {
                480.
            } else if input {
                360.
            } else {
                440.
            })
            .headline(headline)
            .when(client, |d| {
                d.track_body_scroll(&self.dialog_scroll)
                    .dividers(self.dialog_overflow)
            })
            .when_some(supporting, |d, s| d.supporting_text(s))
            .when(input, |d| {
                d.content(
                    div()
                        .px(rem(24.))
                        .pt(rem(8.))
                        .child(
                            TextField::new("dialog-name", &self.name_input)
                                .variant(TextFieldVariant::Outlined)
                                .label(t("名称", "Name"))
                                .w_full(),
                        )
                        .children(focus_name),
                )
            })
            .when(client, |d| {
                let body = if summary {
                    div().child(self.render_client_summary(waiting, cx))
                } else {
                    let [file, paste] = crate::client_form::import_buttons(
                        &self.dialog_form,
                        ButtonVariant::Text,
                        waiting,
                    );
                    div()
                        .flex()
                        .flex_col()
                        .gap(rem(8.))
                        .child(crate::client_form::fields(
                            &self.dialog_form,
                            true,
                            waiting,
                            cx,
                        ))
                        // Waiting, the wait takes the place of the buttons.
                        .child(if waiting {
                            self.render_waiting(false, cx)
                        } else {
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(rem(4.))
                                .child(file)
                                .child(paste)
                        })
                };
                d.content(
                    div()
                        .px(rem(24.))
                        .when(!summary, |d| d.pt(rem(8.)))
                        .flex()
                        .flex_col()
                        .gap(rem(12.))
                        .child(body)
                        .when(waiting && summary, |d| {
                            d.child(self.render_waiting(false, cx))
                        })
                        .children(self.render_login_error()),
                )
            })
            .action(
                material_button("dialog-cancel", ButtonVariant::Text)
                    .track_focus(&self.cancel_focus)
                    .accessibility_label(t("取消", "Cancel"))
                    .child(t("取消", "Cancel"))
                    .children(focus_cancel)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.close_dialog(cx);
                        this.focus_now(window, cx);
                    })),
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
                // What cannot be undone is in the error colour.
                .when(danger, |b| {
                    b.bg(color(ColorRole::ERROR))
                        .text_color(color(ColorRole::ON_ERROR))
                })
                .disabled(client && !client_valid)
                .loading(
                    waiting,
                    t("正在等待浏览器登录", "Waiting for sign-in in the browser"),
                )
                .accessibility_label(confirm.to_owned())
                .child(confirm.to_owned())
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_dialog(window, cx)),
                ),
            )
            .on_dismiss(move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.close_dialog(cx);
                    this.focus_now(window, cx);
                });
            })
    }

    /// The saved client the account dialog signs in with, and Change… to
    /// sign in with another.
    fn render_client_summary(&self, waiting: bool, cx: &mut Context<Self>) -> Div {
        let id = self
            .oauth_client
            .as_ref()
            .map(|c| crate::client_form::short_id(&c.client_id))
            .unwrap_or_default();
        // Change… beside the label leaves the ID a line of its own.
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem(8.))
                    .child(
                        ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            t("将使用此客户端登录：", "Signs in with this client:"),
                        )
                        .flex_1()
                        .min_w_0(),
                    )
                    .child(
                        material_button("client-summary-change", ButtonVariant::Text)
                            .disabled(waiting)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.client_summary = false;
                                cx.notify();
                            }))
                            .accessibility_label(t("更换客户端", "Change client"))
                            .child(t("更换…", "Change…")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem(8.))
                    .child(div().flex_none().child(ui::icon(
                        "key",
                        20.,
                        ColorRole::ON_SURFACE_VARIANT,
                    )))
                    .child(
                        ui::latin(ui::line(TypeRole::BodyLarge, ColorRole::ON_SURFACE, id))
                            .flex_1(),
                    ),
            )
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
                    if parent.is_some() && parent.as_deref() == this.location.folder() {
                        this.refresh(cx);
                    }
                    this.pump(cx);
                    cx.notify();
                },
            );
        }
    }

    /// Keeps focus somewhere in the window, so the shortcuts and the menu
    /// bar reach the view; the Files page takes it when it is shown.
    ///
    /// Checked after each frame is drawn: focus on an element that frame no
    /// longer has (a page left, a dialog or menu closed) still counts as
    /// focus to GPUI, but no key reaches the view through it, and the menu
    /// bar finds none of its actions.
    fn manage_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        let files = self.page == Page::Files && !self.accounts.is_empty();
        let page = if files {
            self.files_focus.clone()
        } else {
            self.focus.clone()
        };
        let want = std::mem::take(&mut self.focus_page);
        // Typing keeps its field: a page change the typing caused (search
        // results arriving) never takes the keys from it.
        let welcome = self.accounts.is_empty() && self.page != Page::Settings;
        let form = self.client_form.read(cx);
        let form_fields = [
            form.id.read(cx).focus_handle(cx),
            form.secret.read(cx).focus_handle(cx),
        ];
        let mut fields = vec![
            self.search_input.read(cx).focus_handle(cx),
            self.filter_input.read(cx).focus_handle(cx),
        ];
        // The handles this frame does not draw: the Files page's off it, and
        // the name field and Cancel with their dialog gone. (Popups draw outside this
        // view, so its tree cannot tell their focus from a stale one.)
        let dialog_form = self.dialog_form.read(cx);
        let mut stale = vec![
            self.name_input.read(cx).focus_handle(cx),
            self.cancel_focus.clone(),
            dialog_form.id.read(cx).focus_handle(cx),
            dialog_form.secret.read(cx).focus_handle(cx),
        ];
        if !files {
            stale.push(self.files_focus.clone());
            stale.extend(fields.iter().cloned());
            fields.clear();
        }
        if welcome {
            fields.extend(form_fields);
        } else {
            stale.extend(form_fields);
        }
        window.on_next_frame(move |window, cx| {
            let attached = window.focused(cx).is_some()
                && !stale.iter().any(|handle| handle.is_focused(window));
            let typing = attached && fields.iter().any(|f| f.is_focused(window));
            if (want && !typing && !page.is_focused(window)) || !attached {
                window.focus(&page, cx);
            }
        });
    }

    /// Gives the page the keys at once. The check after the next frame
    /// does too, but an inactive window draws no frames: a second shortcut
    /// right after a page change must not meet the old page's focus.
    pub fn focus_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files = self.page == Page::Files && !self.accounts.is_empty();
        let form = self.client_form.read(cx);
        let typing = if files {
            [&self.search_input, &self.filter_input]
                .iter()
                .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
        } else {
            [&form.id, &form.secret]
                .iter()
                .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
        };
        if typing || self.dialog.is_some() {
            return;
        }
        let page = if files {
            self.files_focus.clone()
        } else {
            self.focus.clone()
        };
        window.focus(&page, cx);
    }

    /// A listener for a command that may change the page; the page then
    /// has the keys.
    fn page_action<A: Action>(
        cx: &mut Context<Self>,
        act: impl Fn(&mut Self, &A, &mut Context<Self>) + 'static,
    ) -> impl Fn(&A, &mut Window, &mut App) + 'static {
        cx.listener(move |this, action: &A, window, cx| {
            act(this, action, cx);
            this.focus_now(window, cx);
        })
    }

    /// The window-wide commands of the menu bar and the shortcuts.
    fn on_actions(root: Div, cx: &mut Context<Self>) -> Div {
        use crate::keymap::*;
        root.on_action(Self::page_action(cx, |this, _: &ShowFiles, cx| {
            this.go(Page::Files, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &ShowTransfers, cx| {
            if !this.accounts.is_empty() {
                this.go(Page::Transfers, cx);
            }
        }))
        .on_action(Self::page_action(cx, |this, _: &ShowRoutes, cx| {
            if !this.accounts.is_empty() {
                this.go(Page::Endpoints, cx);
            }
        }))
        .on_action(Self::page_action(cx, |this, _: &ShowSettings, cx| {
            this.go(Page::Settings, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &OpenSettings, cx| {
            this.go(Page::Settings, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &About, cx| {
            this.pane = Pane::About;
            this.pane_open = true;
            this.go(Page::Settings, cx);
        }))
        .on_action(cx.listener(|this, _: &Refresh, _, cx| match this.page {
            Page::Endpoints => {
                this.reload_endpoints();
                cx.notify();
            }
            _ => this.refresh(cx),
        }))
        .on_action(cx.listener(|this, _: &NewFolder, window, cx| {
            if this.api.is_some() {
                this.go(Page::Files, cx);
                this.writable_here(cx);
                this.open_dialog(Dialog::NewFolder, window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &UploadFiles, _, cx| {
            if this.api.is_some() {
                this.writable_here(cx);
                this.choose_upload(false, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &UploadFolder, _, cx| {
            if this.api.is_some() {
                this.writable_here(cx);
                this.choose_upload(true, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Cancel, _, cx| {
            // Escape closes the floating rail when nothing nearer took it.
            if this.rail_modal {
                this.rail_modal = false;
                cx.notify();
            } else {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &FocusFilter, _, cx| {
            if !this.accounts.is_empty() {
                this.go(Page::Files, cx);
                this.focus_search = true;
            }
        }))
        .on_action(cx.listener(|this, _: &FilterFolder, _, cx| {
            if !this.accounts.is_empty() {
                this.go(Page::Files, cx);
                this.open_filter(cx);
            }
        }))
        .on_action(Self::page_action(cx, |this, _: &ViewList, cx| {
            this.grid = false;
            this.go(Page::Files, cx);
        }))
        .on_action(Self::page_action(cx, |this, _: &ViewGrid, cx| {
            this.grid = true;
            this.go(Page::Files, cx);
        }))
        .on_action(Self::page_action(cx, |this, _: &GoBack, cx| {
            // A settings pane shown alone goes back to the list first.
            if this.settings_pane_alone() {
                this.pane_open = false;
                cx.notify();
            } else if !this.accounts.is_empty() {
                this.go_back(cx);
            }
        }))
        .on_action(Self::page_action(cx, |this, _: &GoForward, cx| {
            if !this.accounts.is_empty() {
                this.go_forward(cx);
            }
        }))
        .on_action(Self::page_action(cx, |this, _: &GoMyDrive, cx| {
            this.go_place_from_menu(Place::MyDrive, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &GoShared, cx| {
            this.go_place_from_menu(Place::Shared, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &GoStarred, cx| {
            this.go_place_from_menu(Place::Starred, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &GoRecent, cx| {
            this.go_place_from_menu(Place::Recent, cx)
        }))
        .on_action(Self::page_action(cx, |this, _: &GoTrash, cx| {
            this.go_place_from_menu(Place::Trash, cx)
        }))
        .on_action(|_: &Minimize, window, _| window.minimize_window())
        .on_action(|_: &Zoom, window, _| window.zoom_window())
    }
}

/// How much of a file's name a rename selects: the name less its extension
/// for files (as Finder and Drive do), all of a folder's or a dotfile's.
pub fn name_stem(file: &File) -> usize {
    let name = file.name.as_str();
    if file.folder() || crate::places::native(file) {
        return name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

/// The first part of an error chain (`a: b: c` from `{:#}`), for a toast.
pub fn first_line(error: &str) -> String {
    let line = error.lines().next().unwrap_or_default();
    line.split(": ").next().unwrap_or(line).trim().to_owned()
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
        let viewport = window.viewport_size();
        self.width = f32::from(viewport.width) - self.rail_extra(f32::from(viewport.width));
        self.height = f32::from(viewport.height);
        self.follow_language(window, cx);
        let labels = CaptionLabels {
            minimize: t("最小化", "Minimize").into(),
            maximize: t("最大化", "Maximize").into(),
            restore: t("还原", "Restore").into(),
            close: t("关闭", "Close").into(),
        };
        self.manage_focus(window, cx);
        let content = if self.accounts.is_empty() && self.page != Page::Settings {
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
        Self::on_actions(div(), cx)
            .track_focus(&self.focus)
            .key_context(keymap::APP)
            .size_full()
            .flex()
            .flex_col()
            .font_family(ui::font_family())
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_nav(viewport, cx))
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
                                let undo = self.undo.is_some().then(|| cx.weak_entity());
                                // Messages name files, accounts and addresses: set
                                // in user content's face, Latin first, so a name's
                                // hyphens do not read as dashes; the CJK face
                                // still draws the Chinese around them.
                                d.child(
                                    ui::latin(div().absolute().inset_0()).child(
                                        Snackbar::new("toast", message)
                                            .position(SnackbarPosition::Center)
                                            .when_some(undo, |s, view| {
                                                s.action(t("撤销", "Undo"), move |_, _, cx| {
                                                    let _ =
                                                        view.update(cx, |this, cx| this.undo(cx));
                                                })
                                            })
                                            .on_dismiss(move |_, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.toast = None;
                                                    this.undo = None;
                                                    cx.notify();
                                                });
                                            }),
                                    ),
                                )
                            }),
                    )
                    .children(self.render_modal_nav(viewport, cx)),
            )
            .children(dialog)
    }
}
