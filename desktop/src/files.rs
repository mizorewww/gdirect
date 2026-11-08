//! 文件: browsing Drive's places and folders, searching it, and acting on files.
use crate::i18n::t;
use crate::tf;
use crate::{
    app::{Dialog, Gdirect, Page, SortKey, first_line},
    format::{self, Kind},
    keymap,
    places::{self, Actions, Location, Modified, Place},
    transfers::State,
    ui,
};
use gdirect::api::{File, Owner};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::{collections::HashSet, path::PathBuf, time::Duration};

/// The height of the breadcrumb row and of the selection bar that replaces
/// it, the same so the list stays put when a file is selected.
const BAR_HEIGHT: f32 = 64.;

/// The window width from which the toolbar shows the route chip in words.
const ROUTE_CHIP: f32 = 760.;
/// Below this width the folder filter is an icon button that opens into
/// the field, which then takes the breadcrumb's place.
const FILTER_FIELD: f32 = 760.;

/// Name, Location, date and size; then the ⋮ button.
const COLUMNS: [f32; 4] = [180., 160., 112., 56.];
/// The first page of a listing, small so it shows quickly.
const FIRST_PAGE: u32 = 200;
/// Rows from the end at which the next page is fetched.
const PREFETCH: usize = 40;
/// The blank rows after the list's last, which make
/// [`Gdirect::LISTING_BOTTOM`]'s room (the grid leaves the same). Inert: nothing selects, opens or
/// highlights them, and the keys, Select All and the menus count only files.
const SPACER_ROWS: usize = 2;
/// A list row's height, which the spacer rows share (a uniform list's rows
/// are all one height).
const ROW_HEIGHT: f32 = 58.;
/// The grid's tiles' least width, the gap between them and the grid's margin.
const TILE_MIN: f32 = 176.;
const GRID_GAP: f32 = 12.;
const GRID_PAD: f32 = 16.;

/// A count with its unit: `{n}{zh}` in Chinese, "1 file" or "3 files" in English.
fn count(n: usize, zh: &str, one: &str, many: &str) -> String {
    if crate::i18n::chinese() {
        format!("{n} {zh}")
    } else if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "3 项" or "3 items".
fn item_count(n: usize) -> String {
    count(n, "项", "item", "items")
}

/// "3 个文件" or "3 files".
fn file_count(n: usize) -> String {
    count(n, "个文件", "file", "files")
}

pub fn filter_placeholder() -> &'static str {
    t("在此文件夹中筛选", "Filter this folder")
}

/// The filter's placeholder where the list is not one folder's.
fn filter_list_placeholder() -> &'static str {
    t("筛选", "Filter")
}

pub fn search_placeholder() -> &'static str {
    t("在云端硬盘中搜索", "Search in Drive")
}

/// What the snackbar's Undo reverses.
/// A location in Back and Forward's history, with how it was shown.
#[derive(Clone, Debug)]
pub struct Visit {
    pub location: Location,
    pub sort: (SortKey, bool),
    /// A search's chips, which its spec keeps only as a query.
    pub filters: places::Filters,
}

pub enum Undoable {
    /// Files moved to the trash.
    Untrash(Vec<File>),
    /// Items moved, with the folder each came from.
    MoveBack(Vec<(String, String)>),
}

impl Gdirect {
    /// The listing, filtered and sorted as shown.
    pub fn shown(&self) -> Vec<&File> {
        let needle = self.filter.to_lowercase();
        let trash = self.location.trash();
        let mut files: Vec<&File> = self
            .entries
            .iter()
            // The trash lists what is in trashed folders too; Drive shows the folders.
            .filter(|f| !trash || f.explicitly_trashed)
            .filter(|f| needle.is_empty() || f.name.to_lowercase().contains(&needle))
            .collect();
        let (key, ascending) = self.sort;
        let folders_first = !self.location.flat() || key == SortKey::Name;
        files.sort_by(|a, b| {
            let order = match key {
                SortKey::Name => format::natural_cmp(&a.name, &b.name),
                SortKey::Modified => self.date_of(a).cmp(&self.date_of(b)),
                SortKey::Size => {
                    let size = |f: &File| f.size.as_deref().and_then(|s| s.parse::<u64>().ok());
                    size(a).cmp(&size(b))
                }
            };
            let order = if ascending { order } else { order.reverse() };
            // In a folder, folders stay first whichever way it is sorted.
            if folders_first {
                b.folder().cmp(&a.folder()).then(order)
            } else {
                order
            }
        });
        files
    }

    /// The date column's time: when it was trashed, last opened or shared
    /// at the top of those places, and modified elsewhere.
    fn date_of<'a>(&self, file: &'a File) -> Option<&'a str> {
        let time = match (&self.location.place, self.location.path.is_empty()) {
            (Place::Trash, true) if self.trash_dated() => &file.trashed_time,
            (Place::Recent, true) => &file.viewed_by_me_time,
            (Place::Shared, true) => &file.shared_with_me_time,
            _ => &file.modified_time,
        };
        time.as_deref().or(file.modified_time.as_deref())
    }

    /// Drive reports when an item was trashed only for shared-drive items, so
    /// the trash shows that date only when every row has it.
    fn trash_dated(&self) -> bool {
        !self.entries.is_empty() && self.entries.iter().all(|f| f.trashed_time.is_some())
    }

    fn date_label(&self) -> &'static str {
        match (&self.location.place, self.location.path.is_empty()) {
            (Place::Trash, true) if self.trash_dated() => t("删除日期", "Trashed"),
            (Place::Recent, true) => t("上次打开", "Last opened"),
            (Place::Shared, true) => t("共享日期", "Shared"),
            _ => t("修改时间", "Modified"),
        }
    }

    fn picked(&self) -> Vec<File> {
        self.entries
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .cloned()
            .collect()
    }

    /// Lists the location again, showing what is on screen (or the cached
    /// listing) meanwhile, as far down as it was loaded.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        if self.entries.is_empty()
            && let Some(folder) = self.location.folder()
            && let Ok(cached) = api.cached(folder)
        {
            self.entries = cached;
        }
        let want = self.entries.len();
        self.load(want, cx);
    }

    /// Fetches the listing's pages until `want` items or the first page.
    fn load(&mut self, want: usize, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.generation += 1;
        let (generation, session) = (self.generation, self.session);
        let mut query = self.location.query();
        query.page_size = if want > FIRST_PAGE as usize {
            1000
        } else {
            query.page_size.min(FIRST_PAGE)
        };
        let cache = self.location.folder().map(str::to_owned);
        let account = self.account.clone().unwrap_or_default();
        self.loading = true;
        self.loading_more = false;
        self.background(
            move || -> Result<(Vec<File>, Option<String>), String> {
                let mut files = Vec::new();
                let mut token: Option<String> = None;
                loop {
                    let page = api
                        .query(&query, token.as_deref())
                        .map_err(|e| format!("{e:#}"))?;
                    files.extend(page.files);
                    token = page.next;
                    if token.is_none() || files.len() >= want.max(1) {
                        break;
                    }
                    query.page_size = 1000;
                }
                // A whole folder is kept to show at once next time.
                if token.is_none()
                    && let Some(folder) = cache
                    && let Ok(json) = serde_json::to_string(&files)
                    && let Ok(db) = gdirect::store::Store::open(&api.db)
                {
                    let _ = db.cache(&account, &folder, &json);
                }
                Ok((files, token))
            },
            move |this, result, cx| {
                if this.session != session || this.generation != generation {
                    return;
                }
                this.loading = false;
                match result {
                    Ok((files, next)) => {
                        this.entries = files;
                        this.next_page = next;
                        this.list_error = None;
                        this.selected
                            .retain(|id| this.entries.iter().any(|f| &f.id == id));
                        this.select_pending();
                        this.name_parents();
                    }
                    Err(e) => {
                        // A cached listing stays on screen; say why it may be stale.
                        if !this.entries.is_empty() {
                            let brief = first_line(&e);
                            this.say(
                                tf!(
                                    "无法刷新此文件夹：{brief}",
                                    "Couldn’t refresh this folder: {brief}"
                                ),
                                cx,
                            );
                        }
                        this.list_error = Some(e);
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Selects the items waiting for the listing (see
    /// [`Gdirect::pending_select`]) that it has, and scrolls to the first.
    fn select_pending(&mut self) {
        if std::mem::take(&mut self.cursor_first)
            && self.pending_select.is_empty()
            && self.cursor.is_none()
            && let Some(first) = self.shown().first().map(|f| f.id.clone())
        {
            // The ring only: a selection would cover the breadcrumb with the
            // selection bar. The first arrow selects the row it is on.
            self.cursor = Some(first);
            self.keyboard_cursor = true;
            return;
        }
        let want = std::mem::take(&mut self.pending_select);
        let found: Vec<String> = want
            .into_iter()
            .filter(|id| self.entries.iter().any(|f| &f.id == id))
            .collect();
        let Some(first) = found.first().cloned() else {
            return;
        };
        if let Some(ix) = self.shown().iter().position(|f| f.id == first) {
            self.scroll_into_view(ix);
        }
        self.cursor = Some(first.clone());
        self.anchor = Some(first);
        self.selected = found;
    }

    /// Fetches the listing's next page, as the list nears its end.
    fn load_more(&mut self) {
        if self.loading || self.loading_more {
            return;
        }
        let (Some(api), Some(token)) = (self.api.clone(), self.next_page.clone()) else {
            return;
        };
        self.loading_more = true;
        let (generation, session) = (self.generation, self.session);
        let mut query = self.location.query();
        query.page_size = 1000;
        self.background(
            move || {
                api.query(&query, Some(&token))
                    .map_err(|e| format!("{e:#}"))
            },
            move |this, result, cx| {
                if this.session != session || this.generation != generation {
                    return;
                }
                this.loading_more = false;
                match result {
                    Ok(page) => {
                        let known: HashSet<String> =
                            this.entries.iter().map(|f| f.id.clone()).collect();
                        this.entries
                            .extend(page.files.into_iter().filter(|f| !known.contains(&f.id)));
                        this.next_page = page.next;
                        this.name_parents();
                    }
                    // The Load more row stays, to try again.
                    Err(e) => {
                        let brief = first_line(&e);
                        this.list_error = Some(e);
                        this.say(
                            tf!("无法加载更多：{brief}", "Couldn’t load more: {brief}"),
                            cx,
                        );
                    }
                }
                cx.notify();
            },
        );
    }

    /// Reads the names of the folders a flat listing's items are in, for
    /// its Location column; each folder once.
    fn name_parents(&mut self) {
        let Some(api) = self.api.clone() else {
            return;
        };
        if !self.location.flat() {
            return;
        }
        let mut wanted = Vec::new();
        for parent in self.entries.iter().filter_map(|f| f.parents.first()) {
            if Some(parent) != self.root_id.as_ref()
                && !self.folder_names.contains_key(parent)
                && self.naming.insert(parent.clone())
            {
                wanted.push(parent.clone());
            }
        }
        if wanted.is_empty() {
            return;
        }
        let drives = self.drives.clone();
        let tx = self.sender();
        std::thread::spawn(move || {
            for id in wanted {
                let name = match drives.iter().find(|d| d.id == id) {
                    Some(drive) => drive.name.clone(),
                    None => api
                        .metadata(&id)
                        .map(|f| f.name)
                        .unwrap_or_else(|_| "—".into()),
                };
                let sent = tx.send(Box::new(move |this: &mut Gdirect, cx| {
                    this.naming.remove(&id);
                    this.folder_names.insert(id, name);
                    cx.notify();
                }));
                if sent.is_err() {
                    break;
                }
            }
        });
    }

    /// Shows the top of a place.
    pub fn go_place(&mut self, place: Place, cx: &mut Context<Self>) {
        // Lists in time order start with the latest. (History keeps the
        // sort of the place being left, so it changes after.)
        let sort = match place {
            Place::Recent | Place::Shared | Place::Trash => (SortKey::Modified, false),
            _ => (SortKey::Name, true),
        };
        let location = Location::new(place);
        self.remember(&location);
        self.sort = sort;
        self.arrive(location, cx);
    }

    /// Shows `location`, which history has been told about.
    fn arrive(&mut self, location: Location, cx: &mut Context<Self>) {
        if !matches!(location.place, Place::Search(_)) {
            self.before_search = None;
            self.clear_search = true;
            self.filters = places::Filters::default();
        }
        self.location = location;
        self.entered(cx);
        self.go(Page::Files, cx);
    }

    /// The location on screen as history keeps it: with its sort, and a
    /// search's chips.
    fn visit(&self) -> Visit {
        Visit {
            location: self.location.clone(),
            sort: self.sort,
            filters: self.filters,
        }
    }

    /// Shows a location Back or Forward returns to, as it was left: its
    /// sort, and a search's text and chips.
    fn revisit(&mut self, visit: Visit, cx: &mut Context<Self>) {
        self.sort = visit.sort;
        if let Place::Search(spec) = &visit.location.place {
            self.filters = visit.filters;
            self.clear_search = false;
            self.restore_search = Some(spec.text.clone());
            self.search_serial += 1;
            // Clearing it goes back to where it started: the step before it.
            if self.location.search().is_none() {
                self.before_search = self
                    .history
                    .last()
                    .filter(|v| v.location.search().is_none())
                    .cloned();
            }
            self.location = visit.location;
            self.entered(cx);
            self.go(Page::Files, cx);
        } else {
            self.restore_search = None;
            self.arrive(visit.location, cx);
        }
    }

    /// Records the location being left for `next`, for Back.
    fn remember(&mut self, next: &Location) {
        if *next == self.location {
            return;
        }
        if self.history.last().map(|v| &v.location) != Some(&self.location) {
            self.history.push(self.visit());
            if self.history.len() > 100 {
                self.history.remove(0);
            }
        }
        self.future.clear();
    }

    /// The location before the last one navigated to.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        let Some(previous) = self.history.pop() else {
            return self.go(Page::Files, cx);
        };
        // Back up to a folder above selects the one it came from, as going up does.
        let from = (previous.location.place == self.location.place
            && self.location.path.starts_with(&previous.location.path))
        .then(|| {
            self.location
                .path
                .get(previous.location.path.len())
                .map(|c| c.id.clone())
        })
        .flatten();
        self.future.push(self.visit());
        self.revisit(previous, cx);
        self.pending_select.extend(from);
    }

    /// The location Back left.
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self.future.pop() else {
            return self.go(Page::Files, cx);
        };
        self.history.push(self.visit());
        self.revisit(next, cx);
    }

    /// A place chosen from the Go menu or its shortcut.
    pub fn go_place_from_menu(&mut self, place: Place, cx: &mut Context<Self>) {
        if !self.accounts.is_empty() {
            self.go_place(place, cx);
        }
    }

    fn open_folder(&mut self, folder: &File, cx: &mut Context<Self>) {
        if self.location.search().is_some() {
            // A search result opens where it lives.
            self.reveal(folder.clone(), cx);
            return;
        }
        let by_keyboard = std::mem::take(&mut self.keyboard_open);
        let mut next = self.location.clone();
        next.enter(folder);
        self.remember(&next);
        self.location = next;
        self.entered(cx);
        // Opened from the keyboard, the keys go on in the folder's first row.
        self.cursor_first = by_keyboard;
    }

    /// Goes up to breadcrumb `depth`, with the folder it came from selected.
    fn back_to(&mut self, depth: usize, cx: &mut Context<Self>) {
        let from = self.location.path.get(depth).map(|c| c.id.clone());
        let mut next = self.location.clone();
        next.back_to(depth);
        self.remember(&next);
        self.location = next;
        self.entered(cx);
        self.pending_select.extend(from);
    }

    /// Opens the folder `id` where it lives, as the finished-upload list does.
    pub fn show_folder(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        if id == "root" || Some(&id) == self.root_id.as_ref() {
            self.go_place(Place::MyDrive, cx);
            return;
        }
        let folder = File {
            id,
            name,
            mime_type: "application/vnd.google-apps.folder".into(),
            ..File::default()
        };
        self.go(Page::Files, cx);
        self.reveal(folder, cx);
    }

    /// Opens `folder` in its real place: My Drive, its Shared drive, or
    /// Shared with me, below its ancestors.
    fn reveal(&mut self, folder: File, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let session = self.session;
        let root = self.root_id.clone();
        self.loading = true;
        self.background(
            move || {
                let folder = api.metadata(&folder.id).unwrap_or(folder);
                let chain = api.ancestors(&folder.id).ok();
                let root = root.or_else(|| api.metadata("root").ok().map(|f| f.id));
                (folder, chain, root)
            },
            move |this, (folder, chain, root), cx| {
                if this.session != session {
                    return;
                }
                if this.root_id.is_none() {
                    this.root_id = root.clone();
                }
                let location = places::located(&folder, chain, root.as_deref(), &this.drives);
                this.remember(&location);
                this.sort = (SortKey::Name, true);
                this.arrive(location, cx);
            },
        );
        cx.notify();
    }

    fn entered(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.next_page = None;
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.pending_select.clear();
        self.cursor_first = false;
        self.list_error = None;
        self.filter.clear();
        self.clear_filter = true;
        self.filter_open = false;
        self.chip_menu = None;
        self.rows.scroll_to_item(0, ScrollStrategy::Top);
        self.grid_scroll.set_offset(Point::default());
        self.refresh(cx);
    }

    /// New files go to My Drive from places that take none, as Drive's New does.
    pub fn writable_here(&mut self, cx: &mut Context<Self>) {
        if !self.location.writable() {
            self.go_place(Place::MyDrive, cx);
        }
    }

    // Search

    /// The search field changed: search once typing pauses, or go back
    /// where the search started when it is emptied.
    pub fn search_typed(&mut self, cx: &mut Context<Self>) {
        self.search_serial += 1;
        let serial = self.search_serial;
        if self.search_input.read(cx).value().trim().is_empty() {
            if !self.filters.any() {
                self.end_search(cx);
            }
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_serial == serial {
                    this.search_now(cx);
                }
            });
        })
        .detach();
    }

    /// Searches all of Drive for the field's text and the chips' filters.
    pub fn search_now(&mut self, cx: &mut Context<Self>) {
        self.search_serial += 1;
        let text = self.search_input.read(cx).value().to_string();
        if text.trim().is_empty() && !self.filters.any() {
            self.end_search(cx);
            return;
        }
        let spec = self.filters.spec(&text, chrono::Utc::now());
        if self.location.search() == Some(&spec) && self.page == Page::Files {
            return;
        }
        if self.before_search.is_none() {
            self.before_search = Some(self.visit());
        }
        let next = Location::new(Place::Search(spec));
        // Refining a search is one step back, not one per keystroke.
        if self.location.search().is_none() {
            self.remember(&next);
            // Full-text results come in relevance order; keep it.
            self.sort = (SortKey::Name, true);
        }
        self.location = next;
        self.entered(cx);
        self.go(Page::Files, cx);
    }

    /// Leaves a search for where it started.
    fn end_search(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.location.place, Place::Search(_)) {
            return;
        }
        // As Back would show it: with the sort it had.
        let back = self.before_search.take().unwrap_or_else(|| Visit {
            location: Location::new(Place::MyDrive),
            sort: (SortKey::Name, true),
            filters: places::Filters::default(),
        });
        // Starting the search put this place on Back's list; returning to it
        // takes it off again, so Back goes further.
        if self.history.last().map(|v| &v.location) == Some(&back.location) {
            self.history.pop();
        }
        self.sort = back.sort;
        self.location = back.location;
        self.entered(cx);
    }

    fn clear_search_now(&mut self, cx: &mut Context<Self>) {
        self.clear_search = true;
        self.search_serial += 1;
        // A new search starts without the last one's filters.
        self.filters = places::Filters::default();
        self.end_search(cx);
        cx.notify();
    }

    fn set_filters(&mut self, change: impl FnOnce(&mut places::Filters), cx: &mut Context<Self>) {
        change(&mut self.filters);
        self.chip_menu = None;
        self.search_now(cx);
        cx.notify();
    }

    /// A click on a row or tile: a plain click selects it alone, Command
    /// (Control) toggles it, Shift selects the range from the anchor, and a
    /// double click opens it.
    fn click(
        &mut self,
        file: &File,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.files_focus, cx);
        self.keyboard_cursor = false;
        self.keyboard_open = false;
        let id = file.id.clone();
        if event.click_count() >= 2 {
            self.selected.retain(|s| *s != id);
            self.activate(file, cx);
            return;
        }
        let modifiers = event.modifiers();
        if modifiers.shift {
            let anchor = self
                .anchor
                .clone()
                .or_else(|| self.cursor.clone())
                .unwrap_or_else(|| id.clone());
            self.select_range(&anchor, &id);
            self.anchor = Some(anchor);
        } else if modifiers.secondary(cx) {
            if let Some(i) = self.selected.iter().position(|s| *s == id) {
                self.selected.remove(i);
            } else {
                self.selected.push(id.clone());
            }
            self.anchor = Some(id.clone());
        } else {
            self.selected = vec![id.clone()];
            self.anchor = Some(id.clone());
        }
        self.cursor = Some(id);
        cx.notify();
    }

    /// Selects the shown rows from `from` to `to`, both included.
    fn select_range(&mut self, from: &str, to: &str) {
        let ids: Vec<String> = self.shown().iter().map(|f| f.id.clone()).collect();
        let (Some(a), Some(b)) = (
            ids.iter().position(|i| i == from),
            ids.iter().position(|i| i == to),
        ) else {
            self.selected = vec![to.to_owned()];
            return;
        };
        self.selected = ids[a.min(b)..=a.max(b)].to_vec();
    }

    /// Moves the cursor row by `delta`; `extend` grows the selection from
    /// the anchor instead of selecting the row alone.
    fn move_cursor(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.shown().iter().map(|f| f.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let current = self
            .cursor
            .as_ref()
            .and_then(|c| ids.iter().position(|i| i == c));
        // A cursor on a row nothing selects (a folder just opened from the
        // keyboard) selects that row first.
        let unselected = current.is_some_and(|i| !self.selected.contains(&ids[i]));
        let next = match current {
            Some(i) if unselected && !extend => i,
            Some(i) => (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize,
            None if delta > 0 => 0,
            None => ids.len() - 1,
        };
        let id = ids[next].clone();
        if extend {
            let anchor = self
                .anchor
                .clone()
                .filter(|a| ids.contains(a))
                .or_else(|| current.map(|i| ids[i].clone()))
                .unwrap_or_else(|| id.clone());
            self.select_range(&anchor, &id);
            self.anchor = Some(anchor);
        } else {
            self.selected = vec![id.clone()];
            self.anchor = Some(id.clone());
        }
        self.cursor = Some(id);
        self.keyboard_cursor = true;
        self.scroll_into_view(next);
        cx.notify();
    }

    /// How far Up and Down move: a row, or a grid's row of tiles.
    fn row_step(&self) -> isize {
        if self.grid {
            self.grid_columns() as isize
        } else {
            1
        }
    }

    /// Scrolls item `ix` into view, in the list or the grid.
    fn scroll_into_view(&mut self, ix: usize) {
        if !self.grid {
            return self.rows.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        // The grid is one child of its scroller, so its tiles' place comes
        // from its height: rows of tiles share it evenly.
        let scroller = &self.grid_scroll;
        let Some(grid) = (0..scroller.children_count())
            .filter_map(|i| scroller.bounds_for_item(i))
            .max_by(|a, b| {
                a.size
                    .height
                    .partial_cmp(&b.size.height)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        else {
            return;
        };
        let columns = self.grid_columns().max(1) as usize;
        let rows = self.shown().len().div_ceil(columns).max(1);
        let gap = px(GRID_GAP);
        let pitch = (grid.size.height + gap) / rows as f32;
        let view = scroller.bounds();
        let offset = scroller.offset();
        // Where the tile sits in the scrolled content, and what is in view.
        let top = grid.origin.y - view.origin.y - offset.y + pitch * (ix / columns) as f32;
        let bottom = top + pitch - gap;
        let (seen_top, seen_bottom) = (-offset.y, -offset.y + view.size.height);
        let y = if top < seen_top {
            top - gap
        } else if bottom > seen_bottom {
            bottom - view.size.height + gap
        } else {
            return;
        };
        let y = y.max(px(0.)).min(scroller.max_offset().y);
        scroller.set_offset(point(offset.x, -y));
    }

    /// Home and End: the cursor to the first or last row, selecting it.
    fn move_cursor_to_end(&mut self, last: bool, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.shown().iter().map(|f| f.id.clone()).collect();
        let Some(ix) = (if last {
            ids.len().checked_sub(1)
        } else {
            (!ids.is_empty()).then_some(0)
        }) else {
            return;
        };
        let id = ids[ix].clone();
        self.selected = vec![id.clone()];
        self.anchor = Some(id.clone());
        self.cursor = Some(id);
        self.keyboard_cursor = true;
        self.scroll_into_view(ix);
        cx.notify();
    }

    fn searching(&self, window: &Window, cx: &App) -> bool {
        self.search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    /// Whether the search or filter field has focus; its editing keeps the keys.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        self.searching(window, cx)
            || self
                .filter_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
    }

    fn clear_filter_now(&mut self, cx: &mut Context<Self>) {
        self.filter.clear();
        self.clear_filter = true;
        self.filter_open = false;
        cx.notify();
    }

    /// Opens the folder filter (a button in narrow windows) and gives it the keys.
    pub fn open_filter(&mut self, cx: &mut Context<Self>) {
        if self.location.search().is_some() {
            return;
        }
        self.filter_open = true;
        self.focus_filter = true;
        cx.notify();
    }

    /// The Files page's shortcuts. Each leaves its key to the search and
    /// filter fields while they have focus.
    fn files_actions(root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        use keymap::*;
        root.on_action(cx.listener(|this, _: &SelectAll, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.selected = this.shown().iter().map(|f| f.id.clone()).collect();
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &Copy, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.yank(false, cx);
        }))
        .on_action(cx.listener(|this, _: &Cut, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.yank(true, cx);
        }))
        .on_action(cx.listener(|this, _: &Paste, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.paste(cx);
        }))
        .on_action(cx.listener(|this, _: &Undo, window, cx| {
            if this.typing(window, cx) || this.undo.is_none() {
                return cx.propagate();
            }
            this.undo(cx);
        }))
        .on_action(cx.listener(|this, _: &Cancel, window, cx| {
            // Escape in a field empties it and hands the keys to the list;
            // emptying the search goes back where it started.
            if this.searching(window, cx) {
                this.clear_search_now(cx);
                window.focus(&this.files_focus, cx);
            } else if this.typing(window, cx) {
                this.clear_filter_now(cx);
                window.focus(&this.files_focus, cx);
            } else if !this.selected.is_empty() {
                this.selected.clear();
                cx.notify();
            } else if !this.filter.is_empty() {
                this.clear_filter_now(cx);
            } else if this.location.search().is_some() {
                this.clear_search_now(cx);
            } else {
                cx.propagate();
            }
        }))
        .on_action(cx.listener(|this, _: &Open, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            let picked = match this.picked() {
                none if none.is_empty() => this
                    .cursor
                    .as_ref()
                    .and_then(|id| this.entries.iter().find(|f| &f.id == id))
                    .cloned()
                    .into_iter()
                    .collect(),
                some => some,
            };
            match picked.as_slice() {
                [] => {}
                [one] => {
                    let one = one.clone();
                    this.selected.clear();
                    this.keyboard_open = true;
                    this.activate(&one, cx);
                }
                _ => {
                    let files = this.picked();
                    if Actions::all(&files, this.location.trash()).download {
                        this.download(files, cx);
                    }
                }
            }
        }))
        .on_action(cx.listener(|this, _: &Trash, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            let files = this.picked();
            let can = Actions::all(&files, this.location.trash());
            if can.delete {
                this.open_dialog(Dialog::DeleteForever(files), window, cx);
            } else if can.trash {
                this.trash(files, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Rename, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            if let [file] = this.picked().as_slice()
                && Actions::of(file, this.location.trash()).rename
            {
                this.open_dialog(Dialog::Rename(Box::new(file.clone())), window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &CopyLink, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            if let [file] = this.picked().as_slice()
                && Actions::of(file, this.location.trash()).link
            {
                let file = file.clone();
                this.copy_link(&file, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Parent, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            let depth = this.location.path.len();
            if depth > 0 {
                // The folder it came from gets the keyboard's cursor.
                this.keyboard_cursor = true;
                this.back_to(depth - 1, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &CursorUp, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(-this.row_step(), false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorDown, window, cx| {
            // Down from a field moves into the list.
            if this.typing(window, cx) {
                window.focus(&this.files_focus, cx);
            }
            this.move_cursor(this.row_step(), false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorFirst, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor_to_end(false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorLast, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor_to_end(true, cx);
        }))
        .on_action(cx.listener(|this, _: &ExtendUp, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(-this.row_step(), true, cx);
        }))
        .on_action(cx.listener(|this, _: &ExtendDown, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(this.row_step(), true, cx);
        }))
        // Left and right move between a grid's tiles.
        .on_action(cx.listener(|this, _: &CursorLeft, window, cx| {
            if this.typing(window, cx) || !this.grid {
                return cx.propagate();
            }
            this.move_cursor(-1, false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorRight, window, cx| {
            if this.typing(window, cx) || !this.grid {
                return cx.propagate();
            }
            this.move_cursor(1, false, cx);
        }))
    }

    /// Opens a file: a folder lists, a Google document opens in the
    /// browser, other files download; a shortcut does so for its target.
    fn activate(&mut self, file: &File, cx: &mut Context<Self>) {
        if self.location.trash() {
            self.say(
                t(
                    "回收站中的项目无法打开，请先恢复。",
                    "Items in the trash can’t be opened. Restore them first.",
                ),
                cx,
            );
            return;
        }
        self.resolve(file.clone(), cx, |this, file, cx| {
            if file.folder() {
                this.open_folder(&file, cx);
            } else if places::native(&file) {
                this.open_in_browser(&file, cx);
            } else if Actions::of(&file, false).download {
                this.download(vec![file], cx);
            }
        });
    }

    /// Runs `then` on a shortcut's target (read from Drive), or on the file itself.
    fn resolve(
        &mut self,
        file: File,
        cx: &mut Context<Self>,
        then: fn(&mut Self, File, &mut Context<Self>),
    ) {
        let (Some((target, _)), Some(api)) = (places::shortcut(&file), self.api.clone()) else {
            then(self, file, cx);
            return;
        };
        let target = target.to_owned();
        let session = self.session;
        self.background(
            move || api.metadata(&target).map_err(|e| format!("{e:#}")),
            move |this, result, cx| {
                if this.session != session {
                    return;
                }
                match result {
                    Ok(target) => then(this, target, cx),
                    Err(e) => {
                        let e = first_line(&e);
                        this.say(
                            tf!(
                                "无法打开快捷方式指向的项目：{e}",
                                "Couldn’t open the shortcut’s target: {e}"
                            ),
                            cx,
                        );
                    }
                }
            },
        );
    }

    fn open_in_browser(&mut self, file: &File, cx: &mut Context<Self>) {
        if let Some(link) = &file.web_view_link {
            cx.open_url(link);
        }
    }

    fn copy_link(&mut self, file: &File, cx: &mut Context<Self>) {
        if let Some(link) = &file.web_view_link {
            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
            self.say(t("已复制链接", "Link copied"), cx);
        }
    }

    fn share(&mut self, file: &File, cx: &mut Context<Self>) {
        if let Some(link) = &file.web_view_link {
            cx.open_url(link);
            self.say(
                tf!(
                    "已在浏览器中打开“{}”：用右上角的“共享”按钮管理权限",
                    "Opened “{}” in your browser: use Share at the top right",
                    file.name
                ),
                cx,
            );
        }
    }

    /// Queues files for download; folders are listed first, shortcuts
    /// download their targets.
    pub fn download(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        let (shortcuts, files): (Vec<File>, Vec<File>) = files
            .into_iter()
            .partition(|f| places::shortcut(f).is_some());
        for shortcut in shortcuts {
            self.resolve(shortcut, cx, |this, target, cx| {
                if Actions::of(&target, false).download {
                    this.download(vec![target], cx);
                }
            });
        }
        let dir = self.settings.download_dir.clone();
        let (folders, files): (Vec<File>, Vec<File>) = files.into_iter().partition(File::folder);
        if !files.is_empty() {
            let account = self.account.clone().unwrap_or_default();
            // One file is named, as the other snackbars name one item.
            let one = match files.as_slice() {
                [one] => Some(one.name.clone()),
                _ => None,
            };
            let (added, skipped) = self.queues.enqueue(
                files.into_iter().map(|f| (f, dir.clone())).collect(),
                self.settings.office,
                &account,
            );
            let message = match (added, skipped, one) {
                (0, _, _) => t(
                    "没有可下载的文件：已在队列中，或无法下载",
                    "Nothing to download: already queued, or can’t be downloaded",
                )
                .to_owned(),
                (1, 0, Some(name)) => {
                    tf!("已将“{name}”加入下载队列", "Queued “{name}” for download")
                }
                (n, 0, _) => tf!(
                    "已加入下载队列 · {}",
                    "Queued for download · {}",
                    file_count(n)
                ),
                (n, s, _) => tf!(
                    "已加入下载队列 · {}，跳过 {s} 个",
                    "Queued for download · {}, skipped {s}",
                    file_count(n)
                ),
            };
            self.say(message, cx);
        }
        for folder in folders {
            self.download_folder(folder, dir.clone(), cx);
        }
        self.selected.clear();
        self.pump(cx);
        cx.notify();
    }

    fn download_folder(&mut self, folder: File, dir: PathBuf, _cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let account = self.account.clone().unwrap_or_default();
        let name = folder.name.clone();
        self.listing.push((name.clone(), 0));
        let tx = self.sender();
        let root = dir.join(gdirect::api::safe_name(&folder.name));
        let progress_name = name.clone();
        self.background(
            move || {
                crate::transfers::walk(&api, &folder.id, folder.drive_id.as_deref(), |count| {
                    let name = progress_name.clone();
                    let _ = tx.send(Box::new(move |this: &mut Gdirect, cx| {
                        if let Some(entry) = this.listing.iter_mut().find(|(n, _)| *n == name) {
                            entry.1 = count;
                            cx.notify();
                        }
                    }));
                })
                .map(|files| {
                    files
                        .into_iter()
                        .map(|(file, sub)| (file, root.join(sub)))
                        .collect::<Vec<_>>()
                })
                .map_err(|e| format!("{e:#}"))
            },
            move |this, result, cx| {
                if let Some(i) = this.listing.iter().position(|(n, _)| *n == name) {
                    this.listing.remove(i);
                }
                let message = match result {
                    Ok(files) if files.is_empty() => {
                        tf!("“{name}”里没有文件", "No files in “{name}”")
                    }
                    Ok(files) => {
                        let (added, _) = this.queues.enqueue(files, this.settings.office, &account);
                        tf!(
                            "“{name}”：已加入 {}",
                            "“{name}”: queued {}",
                            file_count(added)
                        )
                    }
                    Err(e) => {
                        let e = first_line(&e);
                        tf!("无法列出“{name}”：{e}", "Couldn’t list “{name}”: {e}")
                    }
                };
                this.say(message, cx);
                this.pump(cx);
            },
        );
    }

    /// Runs a file operation and reports it with `done`; the location is
    /// listed again when it ends.
    fn operate_with<T: Send + 'static>(
        &mut self,
        pending: String,
        work: impl FnOnce(&gdirect::api::Api) -> T + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.busy += 1;
        self.say(pending, cx);
        self.background(
            move || work(&api),
            move |this, result, cx| {
                this.busy -= 1;
                done(this, result, cx);
                this.refresh(cx);
            },
        );
    }

    /// [`Gdirect::operate_with`] for work that reports one message.
    fn operate(
        &mut self,
        pending: String,
        work: impl FnOnce(&gdirect::api::Api) -> anyhow::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        self.operate_with(
            pending,
            move |api| work(api).map_err(|e| format!("{e:#}")),
            |this, result, cx| match result {
                Ok(message) => this.say(message, cx),
                Err(e) => this.say(first_line(&e), cx),
            },
            cx,
        );
    }

    pub fn create_folder(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(parent) = self.location.folder().map(str::to_owned) else {
            return;
        };
        self.operate_with(
            tf!("正在新建“{name}”…", "Creating “{name}”…"),
            move |api| {
                api.create(&parent, &name, true)
                    .map_err(|e| format!("{e:#}"))
            },
            |this, result, cx| match result {
                Ok(file) => {
                    // The listing that follows shows it selected, scrolled to.
                    this.selected.clear();
                    this.pending_select = vec![file.id.clone()];
                    this.say(
                        tf!("已新建文件夹“{}”", "Created folder “{}”", file.name),
                        cx,
                    );
                }
                Err(e) => this.say(first_line(&e), cx),
            },
            cx,
        );
    }

    pub fn rename(&mut self, file: File, name: String, cx: &mut Context<Self>) {
        self.operate(
            tf!("正在重命名“{}”…", "Renaming “{}”…", file.name),
            move |api| {
                let renamed = api.rename(&file.id, &name)?;
                Ok(tf!("已重命名为“{}”", "Renamed to “{}”", renamed.name))
            },
            cx,
        );
    }

    /// Moves files to the trash at once; the snackbar offers to undo it.
    pub fn trash(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        let gone: HashSet<String> = files.iter().map(|f| f.id.clone()).collect();
        // The row after the first one trashed (before it, at the end) takes
        // the selection, so the keys and the bar carry on from there.
        let ids: Vec<String> = self.shown().iter().map(|f| f.id.clone()).collect();
        let next = ids
            .iter()
            .position(|id| gone.contains(id))
            .and_then(|first| {
                ids[first..]
                    .iter()
                    .find(|id| !gone.contains(*id))
                    .or_else(|| ids[..first].iter().rev().find(|id| !gone.contains(*id)))
                    .cloned()
            });
        let was_selected = self.selected.iter().any(|id| gone.contains(id));
        self.selected.retain(|id| !gone.contains(id));
        if was_selected && self.selected.is_empty() {
            if let Some(next) = next {
                self.selected = vec![next.clone()];
                self.anchor = Some(next.clone());
                self.cursor = Some(next);
            } else {
                self.cursor = None;
                self.anchor = None;
            }
        }
        // The rows leave at once; a listing already on its way, read
        // before the trash, is dropped so it cannot bring them back.
        let before = self.entries.len();
        self.entries.retain(|f| !gone.contains(&f.id));
        if self.entries.len() != before {
            self.generation += 1;
            if let Some(ix) = self
                .cursor
                .as_ref()
                .and_then(|c| self.shown().iter().position(|f| &f.id == c))
            {
                self.scroll_into_view(ix);
            }
        }
        let all = files.clone();
        let location = self.location.clone();
        self.operate_with(
            tf!(
                "正在将 {}移至回收站…",
                "Moving {} to trash…",
                item_count(files.len())
            ),
            move |api| {
                let mut done = Vec::new();
                for file in files {
                    if let Err(e) = api.trash(&file.id) {
                        return (done, Some(format!("{e:#}")));
                    }
                    done.push(file);
                }
                (done, None)
            },
            move |this, (done, error), cx| {
                // What failed comes back at once; the listing that follows
                // has the last word.
                if error.is_some() && this.location == location {
                    let failed = all
                        .into_iter()
                        .filter(|f| !done.iter().any(|d: &File| d.id == f.id))
                        .filter(|f| !this.entries.iter().any(|e| e.id == f.id))
                        .collect::<Vec<_>>();
                    this.entries.extend(failed);
                }
                let message = match (&done[..], &error) {
                    ([], Some(e)) => return this.say(first_line(e), cx),
                    ([one], None) => tf!("已将“{}”移至回收站", "Moved “{}” to trash", one.name),
                    (many, None) => tf!(
                        "已将 {}移至回收站",
                        "Moved {} to trash",
                        item_count(many.len())
                    ),
                    (some, Some(e)) => tf!(
                        "已将 {}移至回收站；其余失败：{}",
                        "Moved {} to trash; the rest failed: {}",
                        item_count(some.len()),
                        first_line(e)
                    ),
                };
                this.say_undo(message, Undoable::Untrash(done), cx);
            },
            cx,
        );
    }

    pub fn restore(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        self.selected.clear();
        // Back where they are listed (an undone trash), they come back selected.
        self.pending_select = files.iter().map(|f| f.id.clone()).collect();
        self.operate(
            tf!("正在恢复 {}…", "Restoring {}…", item_count(files.len())),
            move |api| {
                for file in &files {
                    api.untrash(&file.id)?;
                }
                Ok(match files.as_slice() {
                    [one] => tf!("已恢复“{}”", "Restored “{}”", one.name),
                    many => tf!("已恢复 {}", "Restored {}", item_count(many.len())),
                })
            },
            cx,
        );
    }

    pub fn delete_forever(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        self.selected.clear();
        self.operate(
            tf!(
                "正在永久删除 {}…",
                "Deleting {} forever…",
                item_count(files.len())
            ),
            move |api| {
                for file in &files {
                    api.delete(&file.id)?;
                }
                Ok(tf!(
                    "已永久删除 {}",
                    "Deleted {} forever",
                    item_count(files.len())
                ))
            },
            cx,
        );
    }

    pub fn empty_trash(&mut self, cx: &mut Context<Self>) {
        self.selected.clear();
        self.operate(
            t("正在清空回收站…", "Emptying trash…").into(),
            |api| {
                api.empty_trash()?;
                Ok(t("回收站已清空", "Trash emptied").into())
            },
            cx,
        );
    }

    fn set_starred(&mut self, files: Vec<File>, starred: bool, cx: &mut Context<Self>) {
        self.operate(
            if starred {
                t("正在添加星标…", "Adding to starred…")
            } else {
                t("正在移除星标…", "Removing from starred…")
            }
            .into(),
            move |api| {
                for file in &files {
                    api.set_starred(&file.id, starred)?;
                }
                Ok(match (files.as_slice(), starred) {
                    ([one], true) => tf!("已为“{}”加星标", "Starred “{}”", one.name),
                    ([one], false) => tf!("已移除“{}”的星标", "Unstarred “{}”", one.name),
                    (many, true) => tf!("已为 {}加星标", "Starred {}", item_count(many.len())),
                    (many, false) => {
                        tf!("已移除 {}的星标", "Unstarred {}", item_count(many.len()))
                    }
                })
            },
            cx,
        );
    }

    /// Reverses the last trash or move, which the snackbar offered.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        match undo {
            Undoable::Untrash(files) => self.restore(files, cx),
            Undoable::MoveBack(moves) => self.operate(
                t("正在撤销移动…", "Undoing the move…").into(),
                move |api| {
                    for (id, parent) in &moves {
                        api.move_to(id, parent)?;
                    }
                    Ok(tf!(
                        "已将 {}移回原处",
                        "Moved {} back",
                        item_count(moves.len())
                    ))
                },
                cx,
            ),
        }
    }

    fn yank(&mut self, cut: bool, cx: &mut Context<Self>) {
        let files = self.picked();
        let can = Actions::all(&files, self.location.trash());
        if files.is_empty() || !(if cut { can.cut } else { can.copy }) {
            return;
        }
        let message = if cut {
            tf!(
                "已剪切 {} · 打开目标文件夹后粘贴",
                "Cut {} · open the destination folder to paste",
                item_count(files.len())
            )
        } else {
            tf!(
                "已复制 {} · 打开目标文件夹后粘贴",
                "Copied {} · open the destination folder to paste",
                item_count(files.len())
            )
        };
        self.clipboard = Some((files, cut));
        self.selected.clear();
        self.say(message, cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some((files, cut)) = self.clipboard.clone() else {
            return;
        };
        let Some(folder) = self
            .location
            .folder()
            .filter(|_| self.location.writable())
            .map(str::to_owned)
        else {
            self.say(t("不能粘贴到这里", "Can’t paste here"), cx);
            return;
        };
        if cut {
            if files.iter().any(|f| f.id == folder) {
                self.say(
                    t(
                        "不能把文件夹移到它自己里面",
                        "Can’t move a folder into itself",
                    ),
                    cx,
                );
                return;
            }
            self.clipboard = None;
            self.operate_with(
                tf!("正在移动 {}…", "Moving {}…", item_count(files.len())),
                move |api| {
                    let mut moved = Vec::new();
                    for file in &files {
                        match api.move_to_returning(&file.id, &folder) {
                            Ok(Some(old)) => {
                                moved.extend(old.into_iter().next().map(|p| (file.id.clone(), p)))
                            }
                            Ok(None) => {}
                            Err(e) => return (moved, Some(format!("{e:#}"))),
                        }
                    }
                    (moved, None)
                },
                |this, (moved, error), cx| {
                    let n = moved.len();
                    let message = match &error {
                        None => tf!("已移动 {}到此处", "Moved {} here", item_count(n)),
                        Some(e) if n == 0 => return this.say(first_line(e), cx),
                        Some(e) => tf!(
                            "已移动 {}；其余失败：{}",
                            "Moved {}; the rest failed: {}",
                            item_count(n),
                            first_line(e)
                        ),
                    };
                    if n == 0 {
                        this.say(message, cx);
                    } else {
                        this.say_undo(message, Undoable::MoveBack(moved), cx);
                    }
                },
                cx,
            );
        } else {
            // Drive copies files only; folders would need a recursive copy.
            let (folders, files): (Vec<File>, Vec<File>) =
                files.into_iter().partition(File::folder);
            if files.is_empty() {
                self.say(
                    t(
                        "Google 云端硬盘不能复制文件夹；请用剪切来移动",
                        "Google Drive can’t copy folders; use cut to move them",
                    ),
                    cx,
                );
                return;
            }
            self.operate(
                tf!("正在复制 {}…", "Copying {}…", item_count(files.len())),
                move |api| {
                    for file in &files {
                        api.copy(&file.id, &folder)?;
                    }
                    let mut message = tf!("已复制 {}", "Copied {}", item_count(files.len()));
                    if !folders.is_empty() {
                        message += &tf!(
                            "；{}无法复制",
                            "; couldn’t copy {}",
                            count(folders.len(), "个文件夹", "folder", "folders")
                        );
                    }
                    Ok(message)
                },
                cx,
            );
        }
    }

    /// Asks for local files or a folder and queues them for upload here.
    pub fn choose_upload(&mut self, folders: bool, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: !folders,
            directories: folders,
            multiple: true,
            prompt: Some(t("上传", "Upload").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| this.upload(paths, cx));
            }
        })
        .detach();
    }

    pub fn upload(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        if self.api.is_none() || paths.is_empty() {
            return;
        }
        let Some(folder) = self.location.folder().filter(|_| self.location.writable()) else {
            self.say(
                t(
                    "不能上传到这里，请打开可以添加文件的文件夹。",
                    "Can’t upload here. Open a folder you can add files to.",
                ),
                cx,
            );
            return;
        };
        let parent = (folder.to_owned(), self.location.folder_name());
        let n = paths.len();
        for path in paths {
            self.queues.enqueue_upload(path, parent.clone(), &account);
        }
        self.say(
            tf!(
                "已加入上传队列 · {} → “{}”",
                "Queued for upload · {} → “{}”",
                item_count(n),
                parent.1
            ),
            cx,
        );
        self.pump(cx);
    }

    fn file_state(&self, file: &File) -> Option<(&'static str, ColorRole, &'static str)> {
        Some(match self.queues.file_state(&file.id)? {
            State::Done { .. } => (
                "check_circle",
                ColorRole::SUCCESS,
                t("已下载", "Downloaded"),
            ),
            State::Queued => (
                "schedule",
                ColorRole::ON_SURFACE_VARIANT,
                t("排队中", "Queued"),
            ),
            State::Running => (
                "downloading",
                ColorRole::PRIMARY,
                t("下载中", "Downloading"),
            ),
            State::Paused => (
                "pause_circle",
                ColorRole::ON_SURFACE_VARIANT,
                t("已暂停", "Paused"),
            ),
            State::Failed(_) => ("error", ColorRole::ERROR, t("下载失败", "Download failed")),
        })
    }

    pub fn render_files(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // The fields ignore a new value while they are being drawn.
        if std::mem::take(&mut self.clear_filter) {
            let input = self.filter_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            });
        }
        if std::mem::take(&mut self.clear_search) {
            let input = self.search_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            });
        }
        if let Some(text) = self.restore_search.take() {
            let input = self.search_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            });
        }
        if std::mem::take(&mut self.focus_filter) {
            let input = self.filter_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            });
        }
        if std::mem::take(&mut self.focus_search) {
            let input = self.search_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            });
        }
        // The filter says "this folder" only where the list is one, and the
        // field is wide enough for the words.
        let hint = if self.location.folder().is_some() && self.wide(860.) {
            filter_placeholder()
        } else {
            filter_list_placeholder()
        };
        if self.filter_hint != hint {
            self.filter_hint = hint;
            let input = self.filter_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.set_placeholder(hint, window, cx));
            });
        }
        let filter_focused = self
            .filter_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let list_focused = self.files_focus.is_focused(window);
        Self::files_actions(div().id("files"), cx)
            .track_focus(&self.files_focus)
            .key_context(keymap::FILES)
            .size_full()
            .flex()
            .flex_col()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.upload(paths.paths().to_vec(), cx);
            }))
            .child(self.render_toolbar(cx))
            .child(if self.selected.is_empty() {
                self.render_breadcrumb(filter_focused, cx)
                    .into_any_element()
            } else {
                self.render_selection(cx).into_any_element()
            })
            .when(self.location.search().is_some(), |d| {
                d.child(self.render_chips(cx))
            })
            .when(
                self.location.trash() && self.location.path.is_empty() && !self.entries.is_empty(),
                |d| {
                    d.child(
                        ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            t(
                                "移到回收站的项目会在 30 天后被永久删除。",
                                "Items in the trash are deleted forever after 30 days.",
                            ),
                        )
                        .px(rem(24.))
                        .pb(rem(8.)),
                    )
                },
            )
            .child(self.render_listing(list_focused, cx))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let route = self.current_route();
        let account = self.account.clone().unwrap_or_default();
        let initial: SharedString = account
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
            .into();
        let trigger = MaterialButton::new("account")
            .variant(ButtonVariant::Tonal)
            .tone(ButtonTone::Neutral)
            .icon(Symbol::ARROW_DROP_DOWN)
            .icon_position(ButtonIconPosition::Trailing)
            .accessibility_label(t("切换账号", "Switch account"))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    this.account_menu = !this.account_menu;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem(8.))
                    .child(
                        Avatar::new("me", account.clone())
                            .monogram(initial)
                            .size(28.),
                    )
                    // The account gives way first: narrower windows show
                    // the avatar alone, and the route chip keeps its words.
                    .when(self.wide(1100.), |d| d.child(account.clone())),
            );
        let mut menu = MaterialMenu::new("account-menu")
            .style(MenuStyle::Baseline)
            .width(280.);
        let count = self.accounts.len();
        for (i, email) in self.accounts.clone().into_iter().enumerate() {
            let current = Some(&email) == self.account.as_ref();
            menu = menu.item(
                MaterialMenuItem::new(email.clone())
                    .leading(ui::symbol("account_circle", false))
                    .selected(current)
                    .divider(i + 1 == count)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.account_menu = false;
                        this.switch_account(email.clone(), cx);
                    })),
            );
        }
        menu = menu
            .item(
                MaterialMenuItem::new(t("添加账号", "Add account"))
                    .leading(ui::symbol("person_add", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.account_menu = false;
                        this.add_account(cx);
                    })),
            )
            .item(
                MaterialMenuItem::new(t("账号设置", "Account settings"))
                    .leading(ui::symbol("manage_accounts", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.account_menu = false;
                        this.pane = crate::app::Pane::Account;
                        this.pane_open = true;
                        this.go(Page::Settings, cx);
                    })),
            );
        // The account's storage use, under its accounts.
        if let Some(about) = &self.about {
            let quota = places::quota(about.storage.usage, about.storage.limit);
            menu = menu.item(
                MaterialMenuItem::new(quota)
                    .leading(ui::symbol("cloud", false))
                    .disabled(true),
            );
        }
        let searching =
            self.location.search().is_some() || !self.search_input.read(cx).value().is_empty();

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(12.))
            .pt(rem(16.))
            .pb(rem(8.))
            .pl(rem(24.))
            .pr(rem(16.))
            .child(
                div().flex_1().min_w(rem(160.)).max_w(rem(720.)).child(
                    SearchField::new(&self.search_input)
                        .id("search")
                        .leading(ui::icon("search", 24., ColorRole::ON_SURFACE_VARIANT))
                        .when(searching, |field| {
                            field.trailing(
                                IconButton::new(
                                    "clear-search",
                                    Symbol::CLOSE,
                                    t("清除搜索", "Clear search"),
                                )
                                // The field keeps the keys, as a Mac search
                                // field does: the × leaves with the text.
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        this.clear_search_now(cx);
                                        this.search_input
                                            .update(cx, |input, cx| input.focus(window, cx));
                                    },
                                )),
                            )
                        }),
                ),
            )
            // The search takes the room it can use; the route and account
            // controls keep to the right edge.
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(rem(12.))
                    // The route chip leaves narrow windows; the Routes page has it.
                    .when(self.wide(ROUTE_CHIP), |d| {
                        d.child(match route {
                            Some((ip, ms, _)) => MaterialButton::new("route")
                                .variant(ButtonVariant::Outlined)
                                .tone(ButtonTone::Neutral)
                                .accessibility_label(tf!("当前线路 {ip}", "Current route {ip}"))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.go(Page::Endpoints, cx)
                                }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(rem(8.))
                                        .child(
                                            div()
                                                .size(rem(8.))
                                                .rounded_full()
                                                .bg(color(format::latency_role(ms))),
                                        )
                                        .child(ui::mono(ip))
                                        .when_some(ms, |d, ms| {
                                            d.child(
                                                div()
                                                    .text_color(color(
                                                        ColorRole::ON_SURFACE_VARIANT,
                                                    ))
                                                    .whitespace_nowrap()
                                                    .child(crate::endpoints::latency(ms)),
                                            )
                                        }),
                                )
                                .into_any_element(),
                            None if self.connecting || self.scan.is_some() => {
                                MaterialButton::new("route")
                                    .variant(ButtonVariant::Outlined)
                                    .tone(ButtonTone::Neutral)
                                    .loading(true, t("正在连接", "Connecting"))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.go(Page::Endpoints, cx)
                                    }))
                                    .child(t("正在连接…", "Connecting…"))
                                    .into_any_element()
                            }
                            None => MaterialButton::new("route")
                                .variant(ButtonVariant::Outlined)
                                .tone(ButtonTone::Neutral)
                                .icon(Symbol::ERROR)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.go(Page::Endpoints, cx)
                                }))
                                .child(t("未连接", "Not connected"))
                                .into_any_element(),
                        })
                    })
                    // Narrow windows keep a status dot in its place.
                    .when(!self.wide(ROUTE_CHIP), |d| {
                        d.child(self.render_route_dot(cx))
                    })
                    .child(
                        MenuPopup::new("account-popup", trigger, menu)
                            .anchor(Anchor::TopRight)
                            .open(self.account_menu)
                            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                                this.account_menu = *open;
                                cx.notify();
                            })),
                    ),
            )
    }

    /// The route chip's stand-in in narrow windows: a dot in the route's
    /// colour and its latency in short, the route in a tooltip; a click
    /// opens Routes.
    fn render_route_dot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let latency = self
            .current_route()
            .and_then(|(_, ms, _)| ms)
            .map(crate::endpoints::latency);
        let (ink, tip): (ColorRole, SharedString) = match self.current_route() {
            Some((ip, Some(ms), _)) => (
                format::latency_role(Some(ms)),
                tf!(
                    "当前线路 {ip} · {}",
                    "Route {ip} · {}",
                    crate::endpoints::latency(ms)
                )
                .into(),
            ),
            Some((ip, None, _)) => (
                ColorRole::SUCCESS,
                tf!("当前线路 {ip}", "Route {ip}").into(),
            ),
            None if self.connecting || self.scan.is_some() => {
                (ColorRole::OUTLINE, t("正在连接…", "Connecting…").into())
            }
            None => (ColorRole::ERROR, t("未连接", "Not connected").into()),
        };
        let button = ui::type_role(div(), TypeRole::LabelLarge)
            .id("route-dot")
            .h(rem(40.))
            .min_w(rem(40.))
            .px(rem(if latency.is_some() { 12. } else { 0. }))
            .gap(rem(8.))
            .flex_none()
            .rounded_full()
            // Outlined, as the wide window's route chip is.
            .border_1()
            .border_color(color(ColorRole::OUTLINE))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(color(ColorRole::ON_SURFACE_VARIANT))
            .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
            .child(div().size(rem(12.)).rounded_full().bg(color(ink)))
            .children(latency.map(|text| ui::latin(div()).whitespace_nowrap().child(text)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Endpoints, cx)));
        MaterialTooltip::plain("route-tip", tip).trigger(button)
    }

    /// A search chip that opens a menu of choices; selected when it filters.
    fn chip_popup(
        &self,
        id: &'static str,
        label: SharedString,
        selected: bool,
        menu: MaterialMenu,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let chip = FilterChip::new(id, label)
            .selected(selected)
            .trailing_icon(ui::symbol("arrow_drop_down", false));
        MenuPopup::new((id, 1usize), chip, menu)
            .open(self.chip_menu == Some(id))
            .on_open_change(cx.listener(move |this, open: &bool, _, cx| {
                this.chip_menu = open.then_some(id);
                cx.notify();
            }))
    }

    /// The search's filters: type, owner, modified, and full text.
    fn render_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filters = self.filters;
        let option =
            |label: &'static str,
             selected: bool,
             divider: bool,
             set: Box<dyn Fn(&mut places::Filters)>| {
                MaterialMenuItem::new(label)
                    .selected(selected)
                    .divider(divider)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_filters(|f| set(f), cx)
                    }))
            };
        // Baseline menus mark the choice in the secondary container; the
        // Expressive ones' tertiary reads as an alarm in this palette.
        let mut kinds = choice_menu("kind-menu", 220.).item(option(
            t("任何类型", "Any type"),
            filters.kind.is_none(),
            true,
            Box::new(|f| f.kind = None),
        ));
        for kind in places::KINDS {
            kinds = kinds.item(option(
                places::kind_label(kind),
                filters.kind == Some(kind),
                false,
                Box::new(move |f| f.kind = Some(kind)),
            ));
        }
        let mut owners = choice_menu("owner-menu", 200.);
        for owner in [Owner::Any, Owner::Me, Owner::NotMe] {
            owners = owners.item(option(
                places::owner_label(owner),
                filters.owner == owner,
                false,
                Box::new(move |f| f.owner = owner),
            ));
        }
        let mut modified = choice_menu("modified-menu", 200.);
        for when in Modified::ALL {
            modified = modified.item(option(
                when.label(),
                filters.modified == when,
                false,
                Box::new(move |f| f.modified = when),
            ));
        }
        let kind_label = filters.kind.map_or(t("类型", "Type"), places::kind_label);
        let owner_label = match filters.owner {
            Owner::Any => t("所有者", "Owner").into(),
            owner => tf!("所有者：{}", "Owner: {}", places::owner_label(owner)),
        };
        let modified_label = match filters.modified {
            Modified::Any => t("修改时间", "Modified"),
            when => when.label(),
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(8.))
            .px(rem(24.))
            .pb(rem(8.))
            .child(self.chip_popup(
                "kind-chip",
                kind_label.into(),
                filters.kind.is_some(),
                kinds,
                cx,
            ))
            .child(self.chip_popup(
                "owner-chip",
                owner_label.into(),
                filters.owner != Owner::Any,
                owners,
                cx,
            ))
            .child(self.chip_popup(
                "modified-chip",
                modified_label.into(),
                filters.modified != Modified::Any,
                modified,
                cx,
            ))
            .child(
                FilterChip::new("full-text", t("搜索内容", "Full text"))
                    .selected(filters.full_text)
                    .on_toggle(cx.listener(|this, on: &bool, _, cx| {
                        let on = *on;
                        this.set_filters(|f| f.full_text = on, cx)
                    })),
            )
    }

    /// Sorting for views without the column headers to click: the grid, and
    /// lists too narrow for the date or size column.
    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (key, ascending) = self.sort;
        let trigger = IconButton::new("sort", Symbol::SORT, t("排序", "Sort")).on_click(
            cx.listener(|this, event: &ClickEvent, _, cx| {
                if matches!(event, ClickEvent::Keyboard(_)) {
                    this.sort_menu = !this.sort_menu;
                    cx.notify();
                }
            }),
        );
        let item = |label: &'static str, selected: bool, divider: bool, sort: fn(&mut Gdirect)| {
            MaterialMenuItem::new(label)
                .selected(selected)
                .divider(divider)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.sort_menu = false;
                    sort(this);
                    cx.notify();
                }))
        };
        let menu = choice_menu("sort-menu", 200.)
            .item(item(
                t("名称", "Name"),
                key == SortKey::Name,
                false,
                |this| this.sort.0 = SortKey::Name,
            ))
            .item(item(
                self.date_label(),
                key == SortKey::Modified,
                false,
                |this| this.sort.0 = SortKey::Modified,
            ))
            .item(item(
                t("大小", "Size"),
                key == SortKey::Size,
                true,
                |this| this.sort.0 = SortKey::Size,
            ))
            .item(item(t("升序", "Ascending"), ascending, false, |this| {
                this.sort.1 = true
            }))
            .item(item(t("降序", "Descending"), !ascending, false, |this| {
                this.sort.1 = false
            }));
        MenuPopup::new("sort-popup", trigger, menu)
            .anchor(Anchor::TopRight)
            .open(self.sort_menu)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.sort_menu = *open;
                cx.notify();
            }))
    }

    /// The "…" crumb of a collapsed path: a menu of the ancestors it hides.
    fn render_crumb_menu(
        &self,
        names: &[String],
        hidden: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = ui::type_role(div(), TypeRole::TitleLarge)
            .id("crumb-more")
            .px(rem(8.))
            .py(rem(4.))
            .rounded(RADIUS_SMALL)
            .text_color(color(ColorRole::ON_SURFACE_VARIANT))
            .cursor_pointer()
            .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
            .child("…");
        let mut menu = MaterialMenu::new("crumb-menu").width(240.);
        for depth in hidden {
            menu = menu.item(
                MaterialMenuItem::new(names[depth].clone())
                    .leading(ui::symbol("folder", false))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.crumb_menu = false;
                        this.back_to(depth, cx);
                    })),
            );
        }
        MenuPopup::new("crumb-popup", trigger, menu)
            .open(self.crumb_menu)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.crumb_menu = *open;
                cx.notify();
            }))
            .into_any_element()
    }

    fn render_breadcrumb(&self, filter_focused: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let names = self.location.crumbs();
        let last = names.len() - 1;
        let filter_width = if self.wide(860.) { 220. } else { 168. };
        let searching = self.location.search().is_some();
        // Narrow windows: the filter is a button, and the field it opens
        // takes the crumbs' place while it is in use.
        let filter_button = !searching && !self.wide(FILTER_FIELD);
        let filter_over = filter_button && (self.filter_open || !self.filter.is_empty());
        let collapsed = self.collapsed_crumbs(&names, filter_width);
        let mut crumbs = div()
            .flex_1()
            .min_w(rem(80.))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(rem(4.));
        for (depth, name) in names.iter().enumerate() {
            if let Some(hidden) = collapsed.clone().filter(|h| h.contains(&depth)) {
                if depth == hidden.start {
                    if depth > 0 {
                        crumbs = crumbs.child(div().flex_none().flex().child(ui::icon(
                            "chevron_right",
                            24.,
                            ColorRole::OUTLINE,
                        )));
                    }
                    crumbs = crumbs.child(
                        div()
                            .flex_none()
                            .child(self.render_crumb_menu(&names, hidden, cx)),
                    );
                }
                continue;
            }
            if depth > 0 {
                crumbs = crumbs.child(div().flex_none().flex().child(ui::icon(
                    "chevron_right",
                    24.,
                    ColorRole::OUTLINE,
                )));
            }
            // One line: the last crumb shortens to an ellipsis, the others keep their names.
            // Folder names are user content; the places' names read the same in it.
            let label = ui::latin(ui::type_role(div(), TypeRole::TitleLarge))
                .px(rem(8.))
                .py(rem(4.))
                .rounded(RADIUS_SMALL)
                .whitespace_nowrap()
                .when(depth == last, |d| {
                    d.min_w_0().overflow_hidden().text_ellipsis()
                })
                .when(depth != last, |d| d.flex_none())
                .child(name.clone());
            crumbs = crumbs.child(if depth == last {
                label
                    .text_color(color(ColorRole::ON_SURFACE))
                    .into_any_element()
            } else {
                label
                    .id(("crumb", depth))
                    .text_color(color(ColorRole::ON_SURFACE_VARIANT))
                    .cursor_pointer()
                    .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.back_to(depth, cx)),
                    )
                    .into_any_element()
            });
        }
        // An empty list says "Loading…" itself; one indicator is enough.
        if self.loading && !self.shown().is_empty() {
            crumbs = crumbs.child(
                div()
                    .ml(rem(8.))
                    .flex_none()
                    .child(CircularProgress::new("loading", t("正在加载", "Loading")).size(20.)),
            );
        }
        let wide = self.wide(860.);
        let writable = self.location.writable();
        let paste = self
            .clipboard
            .as_ref()
            .filter(|_| writable)
            .map(|(files, cut)| {
                let n = files.len();
                MaterialButton::new("paste")
                    .variant(ButtonVariant::Tonal)
                    .icon(Symbol::CONTENT_PASTE)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.paste(cx)))
                    .child(match (*cut, wide) {
                        (true, true) => tf!("移动到此处 · {}", "Move here · {}", item_count(n)),
                        (false, true) => tf!("粘贴到此处 · {}", "Paste here · {}", item_count(n)),
                        // Narrow windows keep the count and drop the words around it.
                        (true, false) => tf!("移动 · {n}", "Move · {n}"),
                        (false, false) => tf!("粘贴 · {n}", "Paste · {n}"),
                    })
            });
        let empty_trash = (self.location.trash() && !self.shown().is_empty()).then(|| {
            MaterialButton::new("empty-trash")
                .variant(ButtonVariant::Text)
                .icon(Symbol::DELETE_FOREVER)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_dialog(Dialog::EmptyTrash, window, cx)
                }))
                .child(t("清空回收站", "Empty trash"))
        });
        // The search has chips instead; the local filter would read as a second search.
        let mut filter = (!searching && (!filter_button || filter_over)).then(|| {
            div()
                .when(filter_over, |d| d.flex_1().min_w_0())
                .when(!filter_over, |d| d.flex_none().w(rem(filter_width)))
                .child(
                    SearchField::new(&self.filter_input)
                        .id("filter")
                        .field_style(SearchFieldStyle::Surface)
                        .leading(ui::icon("filter_list", 24., ColorRole::ON_SURFACE_VARIANT))
                        .when(!self.filter.is_empty(), |field| {
                            field.trailing(
                                IconButton::new(
                                    "clear-filter",
                                    Symbol::CLOSE,
                                    t("清除筛选", "Clear filter"),
                                )
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, window, cx| {
                                        this.clear_filter_now(cx);
                                        window.focus(&this.files_focus, cx);
                                    },
                                )),
                            )
                        })
                        // A compact desktop field, with an outlined field's
                        // focus: the primary colour, thicker.
                        .h(rem(44.))
                        .when(filter_focused, |field| {
                            field.border_2().border_color(color(ColorRole::PRIMARY))
                        }),
                )
        });
        let open_filter = (filter_button && !filter_over).then(|| {
            IconButton::new(
                "open-filter",
                Symbol::FILTER_LIST,
                t("筛选此文件夹", "Filter this folder"),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_filter(cx)))
        });
        div()
            .h(rem(BAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(rem(8.))
            .pl(rem(16.))
            .pr(rem(16.))
            .when(!filter_over, |d| d.child(crumbs))
            .when(filter_over, |d| d.children(filter.take()))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(rem(8.))
                    .children(filter)
                    .children(open_filter)
                    .children(empty_trash)
                    .children(paste)
                    .when(
                        self.grid || !self.show_modified() || !self.show_size(),
                        |d| d.child(self.render_sort_menu(cx)),
                    )
                    .when(self.clipboard.is_some(), |d| {
                        d.child(
                            IconButton::new(
                                "clear-clipboard",
                                Symbol::CLOSE,
                                t("清除剪贴板", "Clear clipboard"),
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.clipboard = None;
                                    cx.notify();
                                },
                            )),
                        )
                    })
                    .child(
                        ConnectedButtonGroup::new("view", t("视图", "View"))
                            .options([
                                ("list", t("列表视图", "List view")),
                                ("grid", t("网格视图", "Grid view")),
                            ])
                            .icon("list", Symbol::VIEW_LIST)
                            .icon("grid", Symbol::GRID_VIEW)
                            .labels(GroupLabels::Never)
                            .selected(if self.grid { "grid" } else { "list" })
                            .on_change(cx.listener(|this, value: &SharedString, window, cx| {
                                this.grid = value.as_ref() == "grid";
                                // Keys belong to the listing, not the toggle.
                                window.focus(&this.files_focus, cx);
                                cx.notify();
                            })),
                    ),
            )
    }

    /// The crumbs a breadcrumb hides behind "…" so the rest fit on one line.
    /// The current folder's name comes first: the ancestors after the place
    /// go first, as few as do, then the place too, and only then does the
    /// name itself shorten.
    fn collapsed_crumbs(
        &self,
        names: &[String],
        filter_width: f32,
    ) -> Option<std::ops::Range<usize>> {
        let last = names.len() - 1;
        if last == 0 {
            return None;
        }
        // TitleLarge: 22px, under half that for Latin letters; the label's
        // padding and the chevron (with its gaps) before all but the first.
        let width = |(depth, name): (usize, &String)| -> f32 {
            name.chars()
                .map(|c| if c.is_ascii() { 10.5 } else { 22. })
                .sum::<f32>()
                + 16.
                + if depth > 0 { 32. } else { 0. }
        };
        let mut controls = 112. + 16.;
        if self.location.search().is_none() {
            controls += if self.wide(FILTER_FIELD) {
                filter_width + 8.
            } else {
                40. + 8.
            };
        }
        if self.grid || !self.show_modified() || !self.show_size() {
            controls += 48.;
        }
        if self.clipboard.is_some() {
            controls += if self.wide(860.) { 200. } else { 130. };
        }
        if self.location.trash() {
            controls += 160.;
        }
        if self.loading {
            controls += 28.;
        }
        let room = self.content_width() - 32. - controls;
        let widths: Vec<f32> = names.iter().enumerate().map(width).collect();
        let total: f32 = widths.iter().sum();
        if total <= room {
            return None;
        }
        // The "…" taking its own place, after a chevron unless it is first.
        let more = 40. + 32.;
        // Hide 1..end, keeping the place and as many of the last as fit.
        if last >= 2 {
            let mut end = 2;
            while end < last && widths[0] + more + widths[end..].iter().sum::<f32>() > room {
                end += 1;
            }
            if widths[0] + more + widths[end..].iter().sum::<f32>() <= room {
                return Some(1..end);
            }
        }
        // Still too long: the place goes too, and "…" holds every ancestor.
        // (The last crumb's chevron stays; the "…" first has none.)
        Some(0..last)
    }

    fn render_selection(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picked = self.picked();
        // The count in the interface's face, the name in user content's.
        let label = match picked.as_slice() {
            // Named only where the name has room beside the actions.
            [one] if self.content_width() >= 600. => div()
                .flex()
                .items_center()
                .min_w_0()
                .mr(rem(8.))
                .overflow_hidden()
                .child(
                    ui::text(
                        TypeRole::TitleMedium,
                        ColorRole::ON_SURFACE,
                        t("已选择 1 项 · ", "1 selected · "),
                    )
                    .flex_none()
                    .whitespace_nowrap(),
                )
                // Cut in the middle, as the grid's names are: the
                // extension stays in sight.
                .child(
                    ui::name_line(
                        TypeRole::TitleMedium,
                        ColorRole::ON_SURFACE,
                        one.name.clone(),
                    )
                    .text_ellipsis_middle(),
                ),
            // The count keeps its width; the actions give way first.
            many => div().flex().flex_none().mr(rem(8.)).child(ui::line(
                TypeRole::TitleMedium,
                ColorRole::ON_SURFACE,
                tf!("已选择 {} 项", "{} selected", many.len()),
            )),
        };
        let in_trash = self.location.trash();
        let can = Actions::all(&picked, in_trash);
        let single = picked.len() == 1;
        let starred = picked.iter().all(|f| f.starred);
        let action = |id: &'static str, symbol: Symbol, label: &'static str| {
            IconButton::new(id, symbol, label)
        };
        div()
            .h(rem(BAR_HEIGHT))
            .flex_none()
            .overflow_hidden()
            .mx(rem(8.))
            .px(rem(8.))
            .flex()
            .items_center()
            .gap(rem(4.))
            .rounded(rem(32.))
            .bg(color(ColorRole::SURFACE_CONTAINER_HIGH))
            .child(
                action("clear", Symbol::CLOSE, t("取消选择", "Clear selection")).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.selected.clear();
                        // The bar leaves with the selection; the list keeps the keys.
                        window.focus(&this.files_focus, cx);
                        cx.notify();
                    }),
                ),
            )
            .child(label.flex_1().min_w(rem(48.)))
            // The bar keeps the same actions whatever is selected, those
            // that do not apply disabled, so none moves under the pointer.
            .when(in_trash, |d| {
                d.child(
                    MaterialButton::new("restore")
                        .disabled(!can.restore)
                        .icon(Symbol::RESTORE_FROM_TRASH)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            let files = this.picked();
                            this.restore(files, cx);
                            window.focus(&this.files_focus, cx);
                        }))
                        .child(t("恢复", "Restore")),
                )
            })
            .when(in_trash, |d| {
                d.child(
                    action(
                        "delete-forever",
                        Symbol::DELETE_FOREVER,
                        t("永久删除", "Delete forever"),
                    )
                    .disabled(!can.delete)
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            let files = this.picked();
                            this.open_dialog(Dialog::DeleteForever(files), window, cx);
                        },
                    )),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    MaterialButton::new("download")
                        .disabled(!can.download)
                        .icon(Symbol::DOWNLOAD)
                        .accessibility_label(t("下载所选项", "Download selected"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            let files = this.picked();
                            this.download(files, cx);
                        }))
                        .child(if self.wide(760.) {
                            tf!(
                                "下载 · {}",
                                "Download · {}",
                                count(
                                    self.settings.connections as usize,
                                    "个连接",
                                    "connection",
                                    "connections"
                                )
                            )
                        } else {
                            t("下载", "Download").into()
                        }),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    action("copy", Symbol::CONTENT_COPY, t("复制", "Copy"))
                        .disabled(!can.copy)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(false, cx))),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    action("cut", Symbol::CONTENT_CUT, t("剪切", "Cut"))
                        .disabled(!can.cut)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(true, cx))),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    action("rename", Symbol::EDIT, t("重命名", "Rename"))
                        .disabled(!single || !can.rename)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            if let [file] = this.picked().as_slice() {
                                this.open_dialog(
                                    Dialog::Rename(Box::new(file.clone())),
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    action(
                        "star",
                        Symbol::STAR,
                        if starred {
                            t("移除星标", "Remove from starred")
                        } else {
                            t("添加星标", "Add to starred")
                        },
                    )
                    .toggle(true)
                    .selected(starred)
                    .selected_icon(ui::symbol("star", true))
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            let files = this.picked();
                            this.set_starred(files, !starred, cx);
                        },
                    )),
                )
            })
            .when(!in_trash, |d| {
                d.child(
                    action("trash", Symbol::DELETE, t("移至回收站", "Move to trash"))
                        .disabled(!can.trash)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            let files = this.picked();
                            this.trash(files, cx);
                            // The list keeps the keys, on the row that
                            // took the selection.
                            window.focus(&this.files_focus, cx);
                        })),
                )
            })
    }

    /// The empty listing's icon, title and explanation, by place.
    fn empty_state(&self) -> (&'static str, SharedString, SharedString) {
        if !self.location.path.is_empty() {
            return if self.location.writable() {
                (
                    "folder_open",
                    t("这个文件夹是空的", "This folder is empty").into(),
                    t(
                        "把文件拖到这里即可上传，或点左上角的 + 新建。",
                        "Drag files here to upload them, or click + at the top left to create something.",
                    )
                    .into(),
                )
            } else {
                (
                    "folder_open",
                    t("这个文件夹是空的", "This folder is empty").into(),
                    SharedString::default(),
                )
            };
        }
        match &self.location.place {
            Place::MyDrive | Place::Drive { .. } => (
                "folder_open",
                t("这里还没有文件", "Nothing here yet").into(),
                t(
                    "把文件拖到这里即可上传，或点左上角的 + 新建。",
                    "Drag files here to upload them, or click + at the top left to create something.",
                )
                .into(),
            ),
            Place::Shared => (
                "folder_shared",
                t("还没有与你共享的内容", "Nothing shared with you yet").into(),
                t(
                    "别人与你共享的文件和文件夹会出现在这里。",
                    "Files and folders others share with you appear here.",
                )
                .into(),
            ),
            Place::Starred => (
                "star",
                t("没有加星标的项目", "No starred items").into(),
                t(
                    "给常用的文件加星标，就能在这里快速找到。",
                    "Star the files you use often to find them here.",
                )
                .into(),
            ),
            Place::Recent => (
                "schedule",
                t("没有最近使用的文件", "No recent files").into(),
                SharedString::default(),
            ),
            Place::Trash => (
                "delete",
                t("回收站是空的", "Trash is empty").into(),
                t(
                    "移到回收站的项目会在 30 天后被永久删除。",
                    "Items in the trash are deleted forever after 30 days.",
                )
                .into(),
            ),
            Place::Search(spec) => (
                "search_off",
                t("没有找到结果", "No results").into(),
                if spec.text.is_empty() {
                    t("换个筛选条件试试。", "Try other filters.").into()
                } else if self.filters.any() || self.filters.full_text {
                    tf!(
                        "没有符合筛选条件且名称包含“{}”的项目。",
                        "No items match “{}” with the current filters.",
                        spec.text
                    )
                    .into()
                } else {
                    tf!(
                        "云端硬盘中没有名称包含“{}”的项目。",
                        "Nothing in Drive has “{}” in its name.",
                        spec.text
                    )
                    .into()
                },
            ),
        }
    }

    fn render_listing(&self, list_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        if self.api.is_none() {
            // Saved routes are checked again, which is no first run.
            let saved = !self.endpoints.is_empty();
            let body = if let Some((_, scan)) = &self.scan
                && !saved
            {
                ui::empty(
                    "radar",
                    tf!(
                        "正在扫描 Google 前端 · 已验证 {} / {}",
                        "Scanning Google front ends · {} / {} verified",
                        scan.good,
                        scan.target.max(1)
                    ),
                    t(
                        "首次使用要找出从本机网络可直达的地址，大约需要一分钟。进度和结果见“线路”页。",
                        "The first run looks for addresses reachable from your network, which takes about a minute. See Routes for progress and results.",
                    ),
                )
            } else if self.scan.is_some() {
                ui::empty(
                    "network_ping",
                    t("正在连接…", "Connecting…"),
                    t(
                        "正在检查已保存的线路。进度见“线路”页。",
                        "Checking your saved routes. See Routes for progress.",
                    ),
                )
            } else if self.connecting {
                ui::empty(
                    "network_ping",
                    t("正在连接…", "Connecting…"),
                    t(
                        "正在通过已保存的线路连接 Google 云端硬盘。",
                        "Connecting to Google Drive through your saved routes.",
                    ),
                )
            } else {
                ui::empty(
                    "cloud_off",
                    t("未连接到 Google 云端硬盘", "Not connected to Google Drive"),
                    self.connect_error.clone().unwrap_or_else(|| {
                        t(
                            "请先在“线路”页扫描可直达的地址。",
                            "Scan for reachable addresses in Routes first.",
                        )
                        .into()
                    }),
                )
                .child(
                    MaterialButton::new("reconnect")
                        .icon(Symbol::REFRESH)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.connect(cx)))
                        .child(t("重新连接", "Reconnect")),
                )
            };
            return div().flex_1().child(body).into_any_element();
        }
        let shown = self.shown();
        if shown.is_empty() && self.next_page.is_none() {
            let body = if let Some(error) = &self.list_error {
                ui::empty(
                    "error",
                    t("无法列出此位置", "Couldn’t list this location"),
                    error.clone(),
                )
            } else if self.loading {
                // An indeterminate M3 indicator, where the icon would be.
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(rem(16.))
                    .py(rem(64.))
                    .child(
                        CircularProgress::new("list-loading", t("正在加载", "Loading")).size(48.),
                    )
                    .child(ui::text(
                        TypeRole::TitleMedium,
                        ColorRole::ON_SURFACE,
                        t("正在加载…", "Loading…"),
                    ))
            } else if !self.filter.is_empty() {
                ui::empty(
                    "search_off",
                    t("没有匹配的文件", "No matching files"),
                    tf!(
                        "当前文件夹里没有名称包含“{}”的项目。",
                        "Nothing in this folder has “{}” in its name.",
                        self.filter
                    ),
                )
            } else {
                let (icon, title, detail) = self.empty_state();
                ui::empty(icon, title, detail)
            };
            return div()
                .flex_1()
                .relative()
                .child(self.folder_context_menu(cx))
                .child(body)
                .into_any_element();
        }
        if self.grid {
            return self.render_grid(shown, cx).into_any_element();
        }
        let count = shown.len();
        drop(shown);
        let more = self.next_page.is_some() as usize;
        let background = self.folder_context_menu(cx);
        let focus_list = self.focus_list(cx);
        let header = self.render_header(cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .px(rem(8.))
            .relative()
            .on_mouse_down(MouseButton::Left, focus_list)
            .child(background)
            .child(header)
            .child(
                uniform_list(
                    "rows",
                    count + more + SPACER_ROWS,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        // Nearing the end fetches the next page, unless it just failed.
                        if range.end + PREFETCH >= count && this.list_error.is_none() {
                            this.load_more();
                        }
                        let shown: Vec<File> = this.shown().into_iter().cloned().collect();
                        let selected = |ix: Option<usize>| {
                            ix.and_then(|ix| shown.get(ix))
                                .is_some_and(|f| this.selected.contains(&f.id))
                        };
                        range
                            .map(|ix| match shown.get(ix) {
                                Some(f) => {
                                    let joins =
                                        (selected(ix.checked_sub(1)), selected(Some(ix + 1)));
                                    let ring = list_focused
                                        && this.keyboard_cursor
                                        && this.cursor.as_deref() == Some(f.id.as_str());
                                    this.render_row(ix, f, joins, ring, cx)
                                }
                                None if ix == count && more == 1 => {
                                    this.render_more(cx).into_any_element()
                                }
                                None => div()
                                    .id(("spacer", ix))
                                    .h(rem(ROW_HEIGHT))
                                    .into_any_element(),
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.rows)
                .flex_1(),
            )
            .into_any_element()
    }

    /// The listing's last row while more pages remain: progress, or a
    /// button to try again.
    fn render_more(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("more")
            .h(rem(ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .child(if self.loading_more || self.loading {
                CircularProgress::new("more-progress", t("正在加载更多", "Loading more"))
                    .size(24.)
                    .into_any_element()
            } else {
                MaterialButton::new("load-more")
                    .variant(ButtonVariant::Text)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.list_error = None;
                        this.load_more();
                        // The button gives way to progress; the list keeps the keys.
                        window.focus(&this.files_focus, cx);
                        cx.notify();
                    }))
                    .child(t("加载更多", "Load more"))
                    .into_any_element()
            })
    }

    /// A right click on a file opens the same menu as its ⋮ button.
    fn file_context_menu(
        &self,
        id: impl Into<ElementId>,
        file: &File,
        cx: &mut Context<Self>,
    ) -> ui::ContextMenu {
        let id: ElementId = id.into();
        let (file, view) = (file.clone(), cx.entity().downgrade());
        let how = (self.location.trash(), self.settings.office);
        let menu_id = ElementId::from((id.clone(), "menu"));
        let (target, track) = (file.id.clone(), view.clone());
        ui::ContextMenu::new(id, move |_, _| {
            file_menu(menu_id.clone(), &file, how, view.clone())
        })
        .on_open_change(move |open, _, cx| {
            let _ = track.update(cx, |this, cx| {
                if open {
                    this.context_row = Some(target.clone());
                } else if this.context_row.as_deref() == Some(target.as_str()) {
                    this.context_row = None;
                }
                cx.notify();
            });
        })
    }

    /// A right click on the listing's empty space offers what + does here,
    /// and paste.
    fn folder_context_menu(&self, cx: &mut Context<Self>) -> ui::ContextMenu {
        let here = Here {
            writable: self.location.writable(),
            paste: self.clipboard.is_some(),
            trash: self.location.trash() && !self.entries.is_empty(),
        };
        let view = cx.entity().downgrade();
        ui::ContextMenu::new("folder-context", move |_, _| {
            folder_menu("folder-context-menu", here, view.clone())
        })
    }

    /// A press in the listing gives the list the keys, from the fields too.
    fn focus_list(
        &self,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        cx.listener(|this, _: &MouseDownEvent, window, cx| {
            window.focus(&this.files_focus, cx);
        })
    }

    /// The room under the listing's last row, inside its scrolling content:
    /// a snackbar floats over the list, and the list scrolls the last row
    /// clear of it. Always there, so nothing moves when one appears. The
    /// list makes it of blank rows after its last ([`SPACER_ROWS`]): padding
    /// would shrink the area it draws rows in instead.
    const LISTING_BOTTOM: f32 = SPACER_ROWS as f32 * ROW_HEIGHT;

    /// Whether a file's ⋮ or right-click menu is open.
    fn menu_open(&self, file: &File) -> bool {
        [&self.row_menu, &self.context_row]
            .iter()
            .any(|open| open.as_deref() == Some(file.id.as_str()))
    }

    /// The list's columns give way, last first, as the window narrows.
    fn show_location(&self) -> bool {
        self.location.flat() && self.wide(1000.)
    }

    /// The date column outlasts Size: every row has a date, where folders
    /// have no size.
    fn show_modified(&self) -> bool {
        self.wide(640.)
    }

    fn show_size(&self) -> bool {
        self.wide(820.)
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (key, ascending) = self.sort;
        let column = |id: &'static str, label: &'static str, this_key: SortKey| {
            let arrow = (key == this_key).then(|| {
                ui::icon(
                    if ascending {
                        "arrow_upward"
                    } else {
                        "arrow_downward"
                    },
                    18.,
                    ColorRole::ON_SURFACE_VARIANT,
                )
            });
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(rem(4.))
                .cursor_pointer()
                .child(label)
                .children(arrow)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    // A new column starts the way Drive and Finder do:
                    // names A to Z, dates newest and sizes largest first.
                    this.sort = if this.sort.0 == this_key {
                        (this_key, !this.sort.1)
                    } else {
                        (this_key, this_key == SortKey::Name)
                    };
                    cx.notify();
                }))
        };
        ui::type_role(div(), TypeRole::TitleSmall)
            .flex()
            .items_center()
            .h(rem(48.))
            .px(rem(16.))
            .border_b_1()
            .border_color(color(ColorRole::OUTLINE_VARIANT))
            .text_color(color(ColorRole::ON_SURFACE_VARIANT))
            .child(div().flex_1().min_w_0().pr(rem(16.)).child(column(
                "sort-name",
                t("名称", "Name"),
                SortKey::Name,
            )))
            .when(self.show_location(), |d| {
                d.child(
                    div()
                        .w(rem(COLUMNS[0]))
                        .pr(rem(16.))
                        .child(if self.owner_column() {
                            t("所有者", "Owner")
                        } else {
                            t("位置", "Location")
                        }),
                )
            })
            .when(self.show_modified(), |d| {
                d.child(div().w(rem(COLUMNS[1])).child(column(
                    "sort-modified",
                    self.date_label(),
                    SortKey::Modified,
                )))
            })
            .when(self.show_size(), |d| {
                d.child(div().w(rem(COLUMNS[2])).child(column(
                    "sort-size",
                    t("大小", "Size"),
                    SortKey::Size,
                )))
            })
            .child(div().w(rem(COLUMNS[3])))
    }

    /// Shared with me puts its items' owner where other lists say where
    /// they are, which there is always the same.
    fn owner_column(&self) -> bool {
        matches!(self.location.place, Place::Shared) && self.location.path.is_empty()
    }

    /// The Location column's cell: the folder an item is in, or its owner.
    fn render_location(&self, file: &File) -> impl IntoElement {
        let (icon, name) = if self.owner_column() {
            let owner = file.owners.first().or(file.sharing_user.as_ref());
            (
                "person",
                owner.and_then(|o| o.display_name.clone().or_else(|| o.email_address.clone())),
            )
        } else {
            (
                "folder",
                places::parent_label(file, self.root_id.as_deref(), &self.folder_names),
            )
        };
        div()
            .w(rem(COLUMNS[0]))
            .pr(rem(16.))
            .flex()
            .items_center()
            .gap(rem(8.))
            .map(|d| match name {
                Some(name) => d
                    .child(ui::icon(icon, 18., ColorRole::ON_SURFACE_VARIANT))
                    .child(ui::name_line(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        name,
                    )),
                // Not known yet (its folder's name is being read): a dash,
                // not a blank cell.
                None => d.child(ui::text(
                    TypeRole::BodyMedium,
                    ColorRole::ON_SURFACE_VARIANT,
                    "—",
                )),
            })
    }

    /// One row. A selected row meets the selected rows next to it (`joins`:
    /// above, below) as one block; `ring` marks the keyboard's row.
    fn render_row(
        &self,
        ix: usize,
        file: &File,
        (above, below): (bool, bool),
        ring: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected.contains(&file.id);
        let (above, below) = (selected && above, selected && below);
        let corner = |joined: bool| rem(if joined { 4. } else { 28. });
        // A row whose menu is open stays highlighted while it is.
        let menu_open = self.menu_open(file);
        let state = self.file_state(file);
        let activate = file.clone();
        div()
            .id(("row", ix))
            .h(rem(ROW_HEIGHT))
            .when(!above, |d| d.pt(rem(2.)))
            .relative()
            .child(self.file_context_menu(("row-context", ix), file, cx))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    // The header's padding, so the columns line up under it
                    // (less the ring's width, which takes from it).
                    .px(rem(if ring { 14. } else { 16. }))
                    .rounded_t(corner(above))
                    .rounded_b(corner(below))
                    .when(ring, |d| {
                        d.border_2().border_color(color(ColorRole::PRIMARY))
                    })
                    .when(selected, |d| d.bg(color(ColorRole::SECONDARY_CONTAINER)))
                    .when(!selected && menu_open, |d| {
                        d.bg(color(ColorRole::ON_SURFACE).opacity(0.08))
                    })
                    .when(!selected && !menu_open, |d| {
                        d.hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.06)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pr(rem(16.))
                            .flex()
                            .items_center()
                            .gap(rem(16.))
                            .child(kind_tile(file, 32., selected))
                            .child(ui::name_line(
                                TypeRole::BodyLarge,
                                ColorRole::ON_SURFACE,
                                file.name.clone(),
                            ))
                            .when(file.starred && !self.location.trash(), |d| {
                                d.child(starred_mark(18.))
                            })
                            .when_some(state, |d, (icon, ink, _)| {
                                d.child(ui::icon(icon, 18., ink))
                            }),
                    )
                    .when(self.show_location(), |d| {
                        d.child(self.render_location(file))
                    })
                    .when(self.show_modified(), |d| {
                        d.child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                format::modified(self.date_of(file)),
                            )
                            .w(rem(COLUMNS[1])),
                        )
                    })
                    .when(self.show_size(), |d| {
                        d.child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                format::size(file),
                            )
                            .w(rem(COLUMNS[2])),
                        )
                    })
                    .child(
                        div()
                            .w(rem(COLUMNS[3]))
                            .child(self.render_row_menu(ix, file, cx)),
                    ),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click(&activate, event, window, cx)
            }))
            .into_any_element()
    }

    /// The ⋮ button. A press opens the file's menu from the pointer, kept
    /// whole inside the window; the keyboard opens it from the button.
    fn render_row_menu(&self, ix: usize, file: &File, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .child(self.render_row_popup(ix, file, cx))
            .child(
                self.file_context_menu(("more-press", ix), file, cx)
                    .on_left_press(),
            )
    }

    fn render_row_popup(&self, ix: usize, file: &File, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.row_menu.as_deref() == Some(file.id.as_str());
        let trigger_id = file.id.clone();
        let trigger = IconButton::new(
            ("more", ix),
            Symbol::MORE_VERT,
            t("更多操作", "More actions"),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            cx.stop_propagation();
            if matches!(event, ClickEvent::Keyboard(_)) {
                this.row_menu = if this.row_menu.as_deref() == Some(trigger_id.as_str()) {
                    None
                } else {
                    Some(trigger_id.clone())
                };
                cx.notify();
            }
        }));
        let menu = file_menu(
            ("row-menu", ix),
            file,
            (self.location.trash(), self.settings.office),
            cx.entity().downgrade(),
        );
        let menu_id = file.id.clone();
        MenuPopup::new(("row-popup", ix), trigger, menu)
            .anchor(Anchor::TopRight)
            .open(open)
            // A press opens the pointer's menu over the button instead (see
            // `render_row_menu`); this one opens from the keyboard only.
            .on_open_change(cx.listener(move |this, open: &bool, _, cx| {
                if !*open && this.row_menu.as_deref() == Some(menu_id.as_str()) {
                    this.row_menu = None;
                    cx.notify();
                }
            }))
    }

    /// The grid's columns: as many tiles of at least [`TILE_MIN`] as the
    /// pane's width holds, each stretched to share the rest.
    fn grid_columns(&self) -> u16 {
        let room = self.content_width() - 2. * GRID_PAD;
        ((room + GRID_GAP) / (TILE_MIN + GRID_GAP)).floor().max(1.) as u16
    }

    fn render_grid(&self, shown: Vec<&File>, cx: &mut Context<Self>) -> impl IntoElement {
        // A divider under the bar above, once tiles scroll beneath it.
        let scrolled = self.grid_scroll.offset().y < px(0.);
        div()
            .id("grid")
            .flex_1()
            .min_h_0()
            .track_scroll(&self.grid_scroll)
            .border_t_1()
            .border_color(if scrolled {
                color(ColorRole::OUTLINE_VARIANT)
            } else {
                gpui_kit::transparent_black().into()
            })
            .overflow_y_scroll()
            .px(rem(GRID_PAD))
            .relative()
            .on_mouse_down(MouseButton::Left, self.focus_list(cx))
            .child(self.folder_context_menu(cx))
            .child(
                div()
                    .grid()
                    .grid_cols(self.grid_columns())
                    .gap(rem(GRID_GAP))
                    .children(shown.into_iter().enumerate().map(|(ix, file)| {
                        let selected = self.selected.contains(&file.id);
                        let menu_open = self.menu_open(file);
                        let activate = file.clone();
                        let state = self.file_state(file);
                        div()
                            .id(("tile", ix))
                            .relative()
                            .child(self.file_context_menu(("tile-context", ix), file, cx))
                            .min_w_0()
                            .p(rem(12.))
                            .flex()
                            .flex_col()
                            .gap(rem(12.))
                            .rounded(rem(16.))
                            .bg(color(if selected {
                                ColorRole::SECONDARY_CONTAINER
                            } else if menu_open {
                                ColorRole::SURFACE_CONTAINER_HIGH
                            } else {
                                ColorRole::SURFACE_CONTAINER_LOW
                            }))
                            .when(!selected, |d| {
                                d.hover(|s| s.bg(color(ColorRole::SURFACE_CONTAINER_HIGH)))
                            })
                            .child(
                                div()
                                    .h(rem(96.))
                                    .rounded(rem(12.))
                                    .bg(color(ColorRole::SURFACE_CONTAINER_LOWEST))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .relative()
                                    .child(kind_tile(file, 48., false))
                                    // A selected tile says so in its corner too.
                                    .when(selected, |d| {
                                        d.child(
                                            div()
                                                .absolute()
                                                .top(rem(8.))
                                                .left(rem(8.))
                                                .size(rem(24.))
                                                .rounded_full()
                                                .bg(color(ColorRole::PRIMARY))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .child(ui::icon(
                                                    "check",
                                                    18.,
                                                    ColorRole::ON_PRIMARY,
                                                )),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(rem(4.))
                                    .right(rem(4.))
                                    .child(self.render_row_menu(ix, file, cx)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rem(6.))
                                    // Cut in the middle: a name's end (a date,
                                    // an extension) tells similar ones apart.
                                    .child(
                                        ui::name_line(
                                            TypeRole::BodyMedium,
                                            ColorRole::ON_SURFACE,
                                            file.name.clone(),
                                        )
                                        .text_ellipsis_middle()
                                        .flex_1(),
                                    )
                                    .when(file.starred && !self.location.trash(), |d| {
                                        d.child(starred_mark(16.))
                                    })
                                    .when_some(state, |d, (icon, ink, _)| {
                                        d.child(ui::icon(icon, 16., ink))
                                    }),
                            )
                            .child(ui::text(
                                TypeRole::BodySmall,
                                ColorRole::ON_SURFACE_VARIANT,
                                if file.folder() {
                                    format::modified(self.date_of(file))
                                } else {
                                    format!(
                                        "{} · {}",
                                        format::size(file),
                                        format::modified(self.date_of(file))
                                    )
                                },
                            ))
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.click(&activate, event, window, cx)
                            }))
                    })),
            )
            .when(self.next_page.is_some(), |d| {
                d.child(self.render_more(cx).pt(rem(8.)))
            })
            // The room under the last tiles, a blank block as the list's is.
            .child(div().h(rem(Self::LISTING_BOTTOM)))
    }
}

/// A menu of choices, the chosen one marked.
fn choice_menu(id: &'static str, width: f32) -> MaterialMenu {
    MaterialMenu::new(id)
        .style(MenuStyle::Baseline)
        .density(MenuDensity::Dense)
        .width(width)
}

/// A file's kind tile, with a shortcut's arrow in its corner. On a selected
/// row, whose container is the tile's own role for folders, it sits on the
/// lowest surface instead.
fn kind_tile(file: &File, size: f32, selected: bool) -> Div {
    ui::kind_tile(Kind::of(file), size)
        .relative()
        .when(selected, |d| {
            d.bg(color(ColorRole::SURFACE_CONTAINER_LOWEST))
        })
        .when(places::shortcut(file).is_some(), |d| {
            d.child(
                div()
                    .absolute()
                    .bottom(rem(-2.))
                    .right(rem(-2.))
                    .rounded_full()
                    .bg(color(ColorRole::SURFACE))
                    .child(ui::icon(
                        "shortcut",
                        size * 0.4,
                        ColorRole::ON_SURFACE_VARIANT,
                    )),
            )
        })
}

/// The filled star after a starred item's name.
fn starred_mark(size: f32) -> gpui_kit_material::icons::Icon {
    ui::symbol("star", true)
        .size(rem(size))
        .color(color(ColorRole::ON_SURFACE_VARIANT))
}

type FileAct = fn(&mut Gdirect, File, &mut Window, &mut Context<Gdirect>);

/// A file's actions, for its ⋮ button and its right-click menu: those
/// Drive allows, in groups.
fn file_menu(
    id: impl Into<ElementId>,
    file: &File,
    (trash, office): (bool, bool),
    view: WeakEntity<Gdirect>,
) -> MaterialMenu {
    let item = |label: SharedString, icon: &'static str, act: FileAct| {
        let (view, file) = (view.clone(), file.clone());
        MaterialMenuItem::new(label)
            .leading(ui::symbol(icon, false))
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |this, cx| {
                    this.row_menu = None;
                    this.context_row = None;
                    act(this, file.clone(), window, cx);
                });
            })
    };
    let can = Actions::of(file, trash);
    let target_folder = file.folder() || places::shortcut(file).is_some_and(|(_, f)| f);
    let native = places::native(file);
    let mut groups: Vec<Vec<MaterialMenuItem>> = Vec::new();
    if trash {
        let mut group = Vec::new();
        if can.restore {
            group.push(item(
                t("恢复", "Restore").into(),
                "restore_from_trash",
                |this, f, _, cx| this.restore(vec![f], cx),
            ));
        }
        if can.delete {
            group.push(item(
                t("永久删除", "Delete forever").into(),
                "delete_forever",
                |this, f, window, cx| this.open_dialog(Dialog::DeleteForever(vec![f]), window, cx),
            ));
        }
        groups.push(group);
    } else {
        let mut open = Vec::new();
        if target_folder {
            open.push(item(
                t("打开", "Open").into(),
                "folder_open",
                |this, f, _, cx| this.activate(&f, cx),
            ));
        }
        if can.link {
            open.push(item(
                t("在浏览器中打开", "Open in browser").into(),
                "open_in_new",
                |this, f, _, cx| this.open_in_browser(&f, cx),
            ));
        }
        if can.download {
            let label: SharedString = if file.folder() {
                t("下载", "Download").into()
            } else if let Some((_, ext)) = native.then(|| format::export(file, office)).flatten() {
                let ext = ext.to_uppercase();
                tf!("下载为 {ext}", "Download as {ext}").into()
            } else {
                t("下载", "Download").into()
            };
            open.push(item(label, "download", |this, f, _, cx| {
                this.download(vec![f], cx)
            }));
        }
        groups.push(open);

        let mut edit = Vec::new();
        if can.rename {
            edit.push(item(
                t("重命名…", "Rename…").into(),
                "edit",
                |this, f, window, cx| this.open_dialog(Dialog::Rename(Box::new(f)), window, cx),
            ));
        }
        if can.copy {
            edit.push(item(
                t("复制", "Copy").into(),
                "content_copy",
                |this, f, _, cx| {
                    this.selected = vec![f.id];
                    this.yank(false, cx)
                },
            ));
        }
        if can.cut {
            edit.push(item(
                t("剪切", "Cut").into(),
                "content_cut",
                |this, f, _, cx| {
                    this.selected = vec![f.id];
                    this.yank(true, cx)
                },
            ));
        }
        groups.push(edit);

        let mut share = Vec::new();
        if can.star {
            share.push(if file.starred {
                item(
                    t("移除星标", "Remove from starred").into(),
                    "star_border",
                    |this, f, _, cx| this.set_starred(vec![f], false, cx),
                )
            } else {
                item(
                    t("添加星标", "Add to starred").into(),
                    "star",
                    |this, f, _, cx| this.set_starred(vec![f], true, cx),
                )
            });
        }
        if can.link {
            share.push(item(
                t("复制链接", "Copy link").into(),
                "link",
                |this, f, _, cx| this.copy_link(&f, cx),
            ));
        }
        if can.share {
            share.push(item(
                t("共享…", "Share…").into(),
                "person_add",
                |this, f, _, cx| this.share(&f, cx),
            ));
        }
        groups.push(share);

        if can.trash {
            groups.push(vec![item(
                t("移至回收站", "Move to trash").into(),
                "delete",
                |this, f, _, cx| this.trash(vec![f], cx),
            )]);
        }
    }
    menu_of(dense_menu(id), groups)
}

/// A file or folder menu: dense, as desktop menus are, so a file's dozen
/// actions fit a short window.
fn dense_menu(id: impl Into<ElementId>) -> MaterialMenu {
    MaterialMenu::new(id)
        .width(220.)
        .density(MenuDensity::Dense)
}

/// Adds groups of items to a menu, a divider between groups.
fn menu_of(mut menu: MaterialMenu, groups: Vec<Vec<MaterialMenuItem>>) -> MaterialMenu {
    let groups: Vec<_> = groups.into_iter().filter(|g| !g.is_empty()).collect();
    let n = groups.len();
    for (i, group) in groups.into_iter().enumerate() {
        let last = group.len() - 1;
        for (j, item) in group.into_iter().enumerate() {
            menu = menu.item(item.divider(j == last && i + 1 < n));
        }
    }
    menu
}

/// What the listing's empty space offers.
#[derive(Clone, Copy)]
struct Here {
    writable: bool,
    paste: bool,
    trash: bool,
}

/// The location's actions, for a right click on its empty space.
fn folder_menu(id: impl Into<ElementId>, here: Here, view: WeakEntity<Gdirect>) -> MaterialMenu {
    let item = |label: &'static str,
                icon: &'static str,
                act: fn(&mut Gdirect, &mut Window, &mut Context<Gdirect>)| {
        let view = view.clone();
        MaterialMenuItem::new(label)
            .leading(ui::symbol(icon, false))
            .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |this, cx| act(this, window, cx));
            })
    };
    let mut groups = Vec::new();
    if here.writable {
        groups.push(vec![
            item(
                t("新建文件夹", "New folder"),
                "create_new_folder",
                |this, window, cx| this.open_dialog(Dialog::NewFolder, window, cx),
            ),
            item(
                t("上传文件…", "Upload files…"),
                "upload_file",
                |this, _, cx| this.choose_upload(false, cx),
            ),
            item(
                t("上传文件夹…", "Upload folder…"),
                "drive_folder_upload",
                |this, _, cx| this.choose_upload(true, cx),
            ),
        ]);
    }
    let mut rest = Vec::new();
    if here.writable && here.paste {
        rest.push(item(
            t("粘贴到此处", "Paste here"),
            "content_paste",
            |this, _, cx| this.paste(cx),
        ));
    }
    if here.trash {
        rest.push(item(
            t("清空回收站", "Empty trash"),
            "delete_forever",
            |this, window, cx| this.open_dialog(Dialog::EmptyTrash, window, cx),
        ));
    }
    rest.push(item(t("刷新", "Refresh"), "refresh", |this, _, cx| {
        this.refresh(cx)
    }));
    groups.push(rest);
    menu_of(dense_menu(id), groups)
}
