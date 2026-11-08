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

/// Name, Location, date and size; then the ⋮ button.
const COLUMNS: [f32; 4] = [180., 160., 112., 56.];
/// The first page of a listing, small so it shows quickly.
const FIRST_PAGE: u32 = 200;
/// Rows from the end at which the next page is fetched.
const PREFETCH: usize = 40;

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

pub fn search_placeholder() -> &'static str {
    t("在云端硬盘中搜索", "Search in Drive")
}

/// What the snackbar's Undo reverses.
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
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
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
            (Place::Trash, true) => &file.trashed_time,
            (Place::Recent, true) => &file.viewed_by_me_time,
            (Place::Shared, true) => &file.shared_with_me_time,
            _ => &file.modified_time,
        };
        time.as_deref().or(file.modified_time.as_deref())
    }

    fn date_label(&self) -> &'static str {
        match (&self.location.place, self.location.path.is_empty()) {
            (Place::Trash, true) => t("删除日期", "Trashed"),
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
        // Lists in time order start with the latest.
        self.sort = match place {
            Place::Recent | Place::Shared | Place::Trash => (SortKey::Modified, false),
            _ => (SortKey::Name, true),
        };
        self.set_location(Location::new(place), cx);
    }

    fn set_location(&mut self, location: Location, cx: &mut Context<Self>) {
        if !matches!(location.place, Place::Search(_)) {
            self.before_search = None;
            self.clear_search = true;
        }
        self.location = location;
        self.entered(cx);
        self.go(Page::Files, cx);
    }

    fn open_folder(&mut self, folder: &File, cx: &mut Context<Self>) {
        if self.location.search().is_some() {
            // A search result opens where it lives.
            self.reveal(folder.clone(), cx);
            return;
        }
        self.location.enter(folder);
        self.entered(cx);
    }

    fn back_to(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.location.back_to(depth);
        self.entered(cx);
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
                this.sort = (SortKey::Name, true);
                let location = places::located(&folder, chain, root.as_deref(), &this.drives);
                this.set_location(location, cx);
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
        self.list_error = None;
        self.filter.clear();
        self.clear_filter = true;
        self.chip_menu = None;
        self.rows.scroll_to_item(0, ScrollStrategy::Top);
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
            self.before_search = Some(self.location.clone());
        }
        // Full-text results come in relevance order; keep it.
        if self.location.search().is_none() {
            self.sort = (SortKey::Name, true);
        }
        self.location = Location::new(Place::Search(spec));
        self.entered(cx);
        self.go(Page::Files, cx);
    }

    /// Leaves a search for where it started.
    fn end_search(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.location.place, Place::Search(_)) {
            return;
        }
        let back = self
            .before_search
            .take()
            .unwrap_or_else(|| Location::new(Place::MyDrive));
        self.location = back;
        self.entered(cx);
    }

    fn clear_search_now(&mut self, cx: &mut Context<Self>) {
        self.clear_search = true;
        self.search_serial += 1;
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
        let next = match current {
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
        self.rows.scroll_to_item(next, ScrollStrategy::Nearest);
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
            match this.picked().as_slice() {
                [] => {}
                [one] => {
                    let one = one.clone();
                    this.selected.clear();
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
        .on_action(cx.listener(|this, _: &Parent, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            let depth = this.location.path.len();
            if depth > 0 {
                this.back_to(depth - 1, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &CursorUp, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(-1, false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorDown, window, cx| {
            // Down from a field moves into the list.
            if this.typing(window, cx) {
                window.focus(&this.files_focus, cx);
            }
            this.move_cursor(1, false, cx);
        }))
        .on_action(cx.listener(|this, _: &ExtendUp, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(-1, true, cx);
        }))
        .on_action(cx.listener(|this, _: &ExtendDown, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(1, true, cx);
        }))
    }

    /// Opens a file: a folder lists, a Google document opens in the
    /// browser, other files download; a shortcut does so for its target.
    fn activate(&mut self, file: &File, cx: &mut Context<Self>) {
        if self.location.trash() {
            self.say(
                t(
                    "回收站中的项目无法打开；请先恢复",
                    "Items in the trash can’t be opened. Restore them first",
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
            let (added, skipped) = self.queues.enqueue(
                files.into_iter().map(|f| (f, dir.clone())).collect(),
                self.settings.office,
                &account,
            );
            let message = match (added, skipped) {
                (0, _) => t(
                    "没有可下载的文件：已在队列中，或无法下载",
                    "Nothing to download: already queued, or can’t be downloaded",
                )
                .to_owned(),
                (n, 0) => tf!(
                    "已加入下载队列 · {}",
                    "Queued for download · {}",
                    file_count(n)
                ),
                (n, s) => tf!(
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
                    Ok(files) if files.is_empty() => tf!("{name} 里没有文件", "No files in {name}"),
                    Ok(files) => {
                        let (added, _) = this.queues.enqueue(files, this.settings.office, &account);
                        tf!("{name}：已加入 {}", "{name}: queued {}", file_count(added))
                    }
                    Err(e) => {
                        let e = first_line(&e);
                        tf!("无法列出 {name}：{e}", "Couldn’t list {name}: {e}")
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
        self.operate(
            tf!("正在新建 {name}…", "Creating {name}…"),
            move |api| {
                let file = api.create(&parent, &name, true)?;
                Ok(tf!("已新建文件夹 {}", "Created folder {}", file.name))
            },
            cx,
        );
    }

    pub fn rename(&mut self, file: File, name: String, cx: &mut Context<Self>) {
        self.operate(
            tf!("正在重命名 {}…", "Renaming {}…", file.name),
            move |api| {
                let renamed = api.rename(&file.id, &name)?;
                Ok(tf!("已重命名为 {}", "Renamed to {}", renamed.name))
            },
            cx,
        );
    }

    /// Moves files to the trash at once; the snackbar offers to undo it.
    pub fn trash(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        self.selected
            .retain(|id| !files.iter().any(|f| &f.id == id));
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
            |this, (done, error), cx| {
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
        self.operate(
            tf!("正在恢复 {}…", "Restoring {}…", item_count(files.len())),
            move |api| {
                for file in &files {
                    api.untrash(&file.id)?;
                }
                Ok(tf!("已恢复 {}", "Restored {}", item_count(files.len())))
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
                t("正在添加星标…", "Adding to Starred…")
            } else {
                t("正在移除星标…", "Removing from Starred…")
            }
            .into(),
            move |api| {
                for file in &files {
                    api.set_starred(&file.id, starred)?;
                }
                Ok(if starred {
                    tf!("已为 {}加星标", "Starred {}", item_count(files.len()))
                } else {
                    tf!("已移除 {}的星标", "Unstarred {}", item_count(files.len()))
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
                    "不能上传到这里；请打开可以添加文件的文件夹",
                    "Can’t upload here. Open a folder you can add files to",
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
                "已加入上传队列 · {} → {}",
                "Queued for upload · {} → {}",
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
        if std::mem::take(&mut self.focus_search) {
            let input = self.search_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            });
        }
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
                self.render_breadcrumb(cx).into_any_element()
            } else {
                self.render_selection(cx).into_any_element()
            })
            .when(self.location.search().is_some(), |d| {
                d.child(self.render_chips(cx))
            })
            .child(self.render_listing(cx))
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
                    // Narrow windows show the avatar alone.
                    .when(self.wide(760.), |d| d.child(account.clone())),
            );
        let mut menu = MaterialMenu::new("account-menu").width(280.);
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
                                .on_click(cx.listener(
                                    |this, _: &ClickEvent, _, cx| this.clear_search_now(cx),
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
                    .when(self.wide(960.), |d| {
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
                                                .bg(color(ColorRole::SUCCESS)),
                                        )
                                        .child(ui::mono(ip))
                                        .when_some(ms, |d, ms| {
                                            d.child(
                                                div()
                                                    .text_color(color(
                                                        ColorRole::ON_SURFACE_VARIANT,
                                                    ))
                                                    .child(format!("{ms} ms")),
                                            )
                                        }),
                                )
                                .into_any_element(),
                            None if self.connecting => MaterialButton::new("route")
                                .variant(ButtonVariant::Outlined)
                                .tone(ButtonTone::Neutral)
                                .loading(true, t("正在选择线路", "Choosing a route"))
                                .child(t("正在选择线路…", "Choosing a route…"))
                                .into_any_element(),
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
                    .when(!self.wide(960.), |d| d.child(self.render_route_dot(cx)))
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
    /// colour, its route and latency in a tooltip; a click opens Routes.
    fn render_route_dot(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (ink, tip): (ColorRole, SharedString) = match self.current_route() {
            Some((ip, Some(ms), _)) => (
                ColorRole::SUCCESS,
                tf!("当前线路 {ip} · {ms} ms", "Route {ip} · {ms} ms").into(),
            ),
            Some((ip, None, _)) => (
                ColorRole::SUCCESS,
                tf!("当前线路 {ip}", "Route {ip}").into(),
            ),
            None if self.connecting => (
                ColorRole::OUTLINE,
                t("正在选择线路…", "Choosing a route…").into(),
            ),
            None => (ColorRole::ERROR, t("未连接", "Not connected").into()),
        };
        let button = div()
            .id("route-dot")
            .size(rem(40.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
            .child(div().size(rem(12.)).rounded_full().bg(color(ink)))
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
        let mut kinds = MaterialMenu::new("kind-menu").width(220.).item(option(
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
        let mut owners = MaterialMenu::new("owner-menu").width(200.);
        for owner in [Owner::Any, Owner::Me, Owner::NotMe] {
            owners = owners.item(option(
                places::owner_label(owner),
                filters.owner == owner,
                false,
                Box::new(move |f| f.owner = owner),
            ));
        }
        let mut modified = MaterialMenu::new("modified-menu").width(200.);
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
        let menu = MaterialMenu::new("sort-menu")
            .width(200.)
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

    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let names = self.location.crumbs();
        let last = names.len() - 1;
        // Deep paths in narrow windows show the place, "…" and the last two.
        let collapsed = (!self.wide(760.) && names.len() >= 4).then(|| 1..last - 1);
        let mut crumbs = div()
            .flex_1()
            .min_w(rem(120.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(4.));
        for (depth, name) in names.iter().enumerate() {
            if let Some(hidden) = collapsed.clone().filter(|h| h.contains(&depth)) {
                if depth == hidden.start {
                    crumbs = crumbs
                        .child(ui::icon("chevron_right", 24., ColorRole::OUTLINE))
                        .child(self.render_crumb_menu(&names, hidden, cx));
                }
                continue;
            }
            if depth > 0 {
                crumbs = crumbs.child(ui::icon("chevron_right", 24., ColorRole::OUTLINE));
            }
            let label = ui::type_role(div(), TypeRole::TitleLarge)
                .px(rem(8.))
                .py(rem(4.))
                .rounded(RADIUS_SMALL)
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
        if self.loading {
            crumbs = crumbs.child(
                div()
                    .ml(rem(8.))
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
        let filter = self.location.search().is_none().then(|| {
            div().w(rem(220.)).child(
                SearchField::new(&self.filter_input)
                    .id("filter")
                    .field_style(SearchFieldStyle::Surface)
                    .leading(ui::icon("filter_list", 24., ColorRole::ON_SURFACE_VARIANT)),
            )
        });
        div()
            .min_h(rem(64.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(8.))
            .py(rem(8.))
            .pl(rem(16.))
            .pr(rem(16.))
            .child(crumbs)
            // The controls wrap as one group, at the right.
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(rem(8.))
                    .children(filter)
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
                            .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                                this.grid = value.as_ref() == "grid";
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_selection(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picked = self.picked();
        let label: SharedString = match picked.as_slice() {
            [one] => tf!("已选择 1 项 · {}", "1 selected · {}", one.name).into(),
            many => tf!("已选择 {} 项", "{} selected", many.len()).into(),
        };
        let can = Actions::all(&picked, self.location.trash());
        let single = picked.len() == 1;
        let starred = picked.iter().all(|f| f.starred);
        let action = |id: &'static str, symbol: Symbol, label: &'static str| {
            IconButton::new(id, symbol, label)
        };
        div()
            .min_h(rem(64.))
            .mx(rem(8.))
            .px(rem(8.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(4.))
            .rounded(rem(32.))
            .bg(color(ColorRole::SURFACE_CONTAINER_HIGH))
            .child(
                action("clear", Symbol::CLOSE, t("取消选择", "Clear selection")).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.selected.clear();
                        cx.notify();
                    }),
                ),
            )
            .child(
                ui::line(TypeRole::TitleMedium, ColorRole::ON_SURFACE, label)
                    .flex_1()
                    .min_w(rem(48.)),
            )
            .when(can.restore, |d| {
                d.child(
                    MaterialButton::new("restore")
                        .icon(Symbol::RESTORE_FROM_TRASH)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            let files = this.picked();
                            this.restore(files, cx);
                        }))
                        .child(t("恢复", "Restore")),
                )
            })
            .when(can.delete, |d| {
                d.child(
                    action(
                        "delete-forever",
                        Symbol::DELETE_FOREVER,
                        t("永久删除", "Delete forever"),
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            let files = this.picked();
                            this.open_dialog(Dialog::DeleteForever(files), window, cx);
                        },
                    )),
                )
            })
            .when(can.download, |d| {
                d.child(
                    MaterialButton::new("download")
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
                                    "连接",
                                    "connection",
                                    "connections"
                                )
                            )
                        } else {
                            t("下载", "Download").into()
                        }),
                )
            })
            .when(can.copy, |d| {
                d.child(
                    action("copy", Symbol::CONTENT_COPY, t("复制", "Copy"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(false, cx))),
                )
            })
            .when(can.cut, |d| {
                d.child(
                    action("cut", Symbol::CONTENT_CUT, t("剪切", "Cut"))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(true, cx))),
                )
            })
            .when(can.rename, |d| {
                d.child(
                    action("rename", Symbol::EDIT, t("重命名", "Rename"))
                        .disabled(!single)
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
            .when(can.star, |d| {
                d.child(
                    action(
                        "star",
                        Symbol::STAR,
                        if starred {
                            t("移除星标", "Remove from Starred")
                        } else {
                            t("添加星标", "Add to Starred")
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
            .when(can.trash, |d| {
                d.child(
                    action("trash", Symbol::DELETE, t("移至回收站", "Move to trash")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let files = this.picked();
                            this.trash(files, cx);
                        }),
                    ),
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
                } else {
                    tf!(
                        "云端硬盘中没有名称包含“{}”的项目",
                        "Nothing in Drive has “{}” in its name",
                        spec.text
                    )
                    .into()
                },
            ),
        }
    }

    fn render_listing(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.api.is_none() {
            let body = if let Some((_, scan)) = &self.scan {
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
            } else if self.connecting {
                ui::empty(
                    "network_ping",
                    t("正在选择线路", "Choosing a route"),
                    t(
                        "首次使用会扫描可直达的 Google 前端，大约需要半分钟。",
                        "The first run scans for reachable Google front ends, which takes about 30 seconds.",
                    ),
                )
            } else {
                ui::empty(
                    "cloud_off",
                    t("未连接到 Google 云端硬盘", "Not connected to Google Drive"),
                    self.connect_error.clone().unwrap_or_else(|| {
                        t(
                            "请先在“线路”页扫描可用地址。",
                            "Scan for usable addresses in Routes first.",
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
                ui::empty("hourglass_empty", t("正在加载…", "Loading…"), "")
            } else if !self.filter.is_empty() {
                ui::empty(
                    "search_off",
                    t("没有匹配的文件", "No matching files"),
                    tf!(
                        "当前文件夹里没有名称包含“{}”的项目",
                        "Nothing in this folder has “{}” in its name",
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
            .pb(rem(self.listing_bottom()))
            .relative()
            .on_mouse_down(MouseButton::Left, focus_list)
            .child(background)
            .child(header)
            .child(
                uniform_list(
                    "rows",
                    count + more,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        // Nearing the end fetches the next page, unless it just failed.
                        if range.end + PREFETCH >= count && this.list_error.is_none() {
                            this.load_more();
                        }
                        let shown: Vec<File> = this.shown().into_iter().cloned().collect();
                        range
                            .map(|ix| match shown.get(ix) {
                                Some(f) => this.render_row(ix, f, cx),
                                None => this.render_more(cx).into_any_element(),
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
            .h(rem(58.))
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
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.list_error = None;
                        this.load_more();
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
    ) -> MaterialContextMenu {
        let id: ElementId = id.into();
        let (file, view) = (file.clone(), cx.entity().downgrade());
        let how = (self.location.trash(), self.settings.office);
        let menu_id = ElementId::from((id.clone(), "menu"));
        MaterialContextMenu::new(id, move |_, _| {
            file_menu(menu_id.clone(), &file, how, view.clone())
        })
    }

    /// A right click on the listing's empty space offers what + does here,
    /// and paste.
    fn folder_context_menu(&self, cx: &mut Context<Self>) -> MaterialContextMenu {
        let here = Here {
            writable: self.location.writable(),
            paste: self.clipboard.is_some(),
            trash: self.location.trash() && !self.entries.is_empty(),
        };
        let view = cx.entity().downgrade();
        MaterialContextMenu::new("folder-context", move |_, _| {
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

    /// The listing's bottom padding: a snackbar on screen takes the room
    /// above the edge, so it never covers the last row.
    fn listing_bottom(&self) -> f32 {
        if self.toast.is_some() { 80. } else { 16. }
    }

    /// The list's columns give way, last first, as the window narrows.
    fn show_location(&self) -> bool {
        self.location.flat() && self.wide(1000.)
    }

    fn show_modified(&self) -> bool {
        self.wide(820.)
    }

    fn show_size(&self) -> bool {
        self.wide(640.)
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
                    this.sort = if this.sort.0 == this_key {
                        (this_key, !this.sort.1)
                    } else {
                        (this_key, true)
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
            .child(div().flex_1().min_w_0().child(column(
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
            .when_some(name, |d, name| {
                d.child(ui::icon(icon, 18., ColorRole::ON_SURFACE_VARIANT))
                    .child(ui::line(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        name,
                    ))
            })
    }

    fn render_row(&self, ix: usize, file: &File, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.contains(&file.id);
        // A row whose ⋮ menu is open stays highlighted while it is.
        let menu_open = self.row_menu.as_deref() == Some(file.id.as_str());
        let state = self.file_state(file);
        let activate = file.clone();
        div()
            .id(("row", ix))
            .h(rem(58.))
            .pt(rem(2.))
            .relative()
            .child(self.file_context_menu(("row-context", ix), file, cx))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    // The header's padding, so the columns line up under it.
                    .px(rem(16.))
                    .rounded(rem(28.))
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
                            .flex()
                            .items_center()
                            .gap(rem(16.))
                            .child(kind_tile(file, 32., selected))
                            .child(ui::line(
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

    fn render_row_menu(&self, ix: usize, file: &File, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_open_change(cx.listener(move |this, open: &bool, _, cx| {
                this.row_menu = open.then(|| menu_id.clone());
                cx.notify();
            }))
    }

    fn render_grid(&self, shown: Vec<&File>, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("grid")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(rem(16.))
            .pb(rem(self.listing_bottom()))
            .relative()
            .on_mouse_down(MouseButton::Left, self.focus_list(cx))
            .child(self.folder_context_menu(cx))
            .child(div().flex().flex_wrap().gap(rem(12.)).children(
                shown.into_iter().enumerate().map(|(ix, file)| {
                    let selected = self.selected.contains(&file.id);
                    let menu_open = self.row_menu.as_deref() == Some(file.id.as_str());
                    let activate = file.clone();
                    let state = self.file_state(file);
                    div()
                        .id(("tile", ix))
                        .relative()
                        .child(self.file_context_menu(("tile-context", ix), file, cx))
                        .w(rem(176.))
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
                                            .child(ui::icon("check", 18., ColorRole::ON_PRIMARY)),
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
                                .child(
                                    ui::line(
                                        TypeRole::BodyMedium,
                                        ColorRole::ON_SURFACE,
                                        file.name.clone(),
                                    )
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
                }),
            ))
            .when(self.next_page.is_some(), |d| {
                d.child(self.render_more(cx).pt(rem(8.)))
            })
    }
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
                t("下载整个文件夹", "Download folder").into()
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
                t("重命名", "Rename").into(),
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
                    t("移除星标", "Remove from Starred").into(),
                    "star_border",
                    |this, f, _, cx| this.set_starred(vec![f], false, cx),
                )
            } else {
                item(
                    t("添加星标", "Add to Starred").into(),
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
    menu_of(MaterialMenu::new(id).width(220.), groups)
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
    menu_of(MaterialMenu::new(id).width(220.), groups)
}
