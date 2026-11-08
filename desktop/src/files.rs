//! 文件: browsing a Drive folder and acting on its files.
use crate::i18n::t;
use crate::tf;
use crate::{
    app::{Dialog, Gdirect, Page, SortKey, first_line},
    format::{self, Kind},
    keymap,
    transfers::State,
    ui,
};
use gdirect::api::File;
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::path::PathBuf;

const COLUMNS: [f32; 3] = [160., 112., 56.];

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

impl Gdirect {
    pub fn folder(&self) -> &str {
        &self.path.last().unwrap().0
    }

    /// The current folder's entries, filtered and sorted as shown.
    pub fn shown(&self) -> Vec<&File> {
        let needle = self.filter.to_lowercase();
        let mut files: Vec<&File> = self
            .entries
            .iter()
            .filter(|f| needle.is_empty() || f.name.to_lowercase().contains(&needle))
            .collect();
        let (key, ascending) = self.sort;
        files.sort_by(|a, b| {
            let order = match key {
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Modified => a.modified_time.cmp(&b.modified_time),
                SortKey::Size => {
                    let size = |f: &File| f.size.as_deref().and_then(|s| s.parse::<u64>().ok());
                    size(a).cmp(&size(b))
                }
            };
            let order = if ascending { order } else { order.reverse() };
            // Folders stay first whichever way the list is sorted.
            b.folder().cmp(&a.folder()).then(order)
        });
        files
    }

    fn picked(&self) -> Vec<File> {
        self.entries
            .iter()
            .filter(|f| self.selected.contains(&f.id))
            .cloned()
            .collect()
    }

    /// Lists the current folder, showing the cached listing meanwhile.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let folder = self.folder().to_owned();
        let session = self.session;
        if self.entries.is_empty()
            && let Ok(cached) = api.cached(&folder)
        {
            self.entries = cached;
        }
        self.loading = true;
        let listed = folder.clone();
        self.background(
            move || api.list(&listed).map_err(|e| format!("{e:#}")),
            move |this, result, cx| {
                if this.session != session || this.folder() != folder {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(files) => {
                        this.entries = files;
                        this.list_error = None;
                        this.selected
                            .retain(|id| this.entries.iter().any(|f| &f.id == id));
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

    pub fn open_folder(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        self.path.push((id, name));
        self.entered(cx);
    }

    fn back_to(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.path.truncate(depth + 1);
        self.entered(cx);
    }

    /// Opens the folder `id` from the root, as the finished-upload list does.
    pub fn show_folder(&mut self, id: String, name: String, cx: &mut Context<Self>) {
        self.path.truncate(1);
        if id != "root" {
            self.path.push((id, name));
        }
        self.entered(cx);
        self.go(Page::Files, cx);
    }

    fn entered(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.list_error = None;
        self.filter.clear();
        self.clear_filter = true;
        self.rows.scroll_to_item(0, ScrollStrategy::Top);
        self.refresh(cx);
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

    /// Whether the filter field has focus; its text editing keeps the keys.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        self.filter_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    fn clear_filter_now(&mut self, cx: &mut Context<Self>) {
        self.filter.clear();
        self.clear_filter = true;
        cx.notify();
    }

    /// The Files page's shortcuts. Each leaves its key to the filter field
    /// while that has focus.
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
        .on_action(cx.listener(|this, _: &Cancel, window, cx| {
            // Escape in the filter empties it and hands the keys to the list.
            if this.typing(window, cx) {
                this.clear_filter_now(cx);
                window.focus(&this.files_focus, cx);
            } else if !this.selected.is_empty() {
                this.selected.clear();
                cx.notify();
            } else if !this.filter.is_empty() {
                this.clear_filter_now(cx);
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
                    this.download(files, cx);
                }
            }
        }))
        .on_action(cx.listener(|this, _: &Trash, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            let files = this.picked();
            if !files.is_empty() {
                this.open_dialog(Dialog::Trash(files), window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Rename, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            if let [file] = this.picked().as_slice() {
                this.open_dialog(Dialog::Rename(file.clone()), window, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &Parent, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            if this.path.len() > 1 {
                this.back_to(this.path.len() - 2, cx);
            }
        }))
        .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
            this.filter_input
                .update(cx, |input, cx| input.focus(window, cx));
        }))
        .on_action(cx.listener(|this, _: &CursorUp, window, cx| {
            if this.typing(window, cx) {
                return cx.propagate();
            }
            this.move_cursor(-1, false, cx);
        }))
        .on_action(cx.listener(|this, _: &CursorDown, window, cx| {
            // Down from the filter moves into the list.
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

    fn activate(&mut self, file: &File, cx: &mut Context<Self>) {
        if file.folder() {
            self.open_folder(file.id.clone(), file.name.clone(), cx);
        } else {
            self.download(vec![file.clone()], cx);
        }
    }

    /// Queues files for download; folders are listed first.
    pub fn download(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
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
                crate::transfers::walk(&api, &folder.id, |count| {
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

    /// Runs a file operation; the folder is listed again when it ends.
    fn operate(
        &mut self,
        pending: String,
        work: impl FnOnce(&gdirect::api::Api) -> anyhow::Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        self.busy += 1;
        self.say(pending, cx);
        self.background(
            move || work(&api).map_err(|e| format!("{e:#}")),
            |this, result, cx| {
                this.busy -= 1;
                match result {
                    Ok(message) => this.say(message, cx),
                    Err(e) => this.say(first_line(&e), cx),
                }
                this.refresh(cx);
            },
        );
    }

    pub fn create_folder(&mut self, name: String, cx: &mut Context<Self>) {
        let parent = self.folder().to_owned();
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

    pub fn trash(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        self.selected
            .retain(|id| !files.iter().any(|f| &f.id == id));
        self.operate(
            tf!(
                "正在将 {}移至回收站…",
                "Moving {} to trash…",
                item_count(files.len())
            ),
            move |api| {
                for file in &files {
                    api.trash(&file.id)?;
                }
                Ok(tf!(
                    "已将 {}移至回收站",
                    "Moved {} to trash",
                    item_count(files.len())
                ))
            },
            cx,
        );
    }

    fn yank(&mut self, cut: bool, cx: &mut Context<Self>) {
        let files = self.picked();
        if files.is_empty() {
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
        let folder = self.folder().to_owned();
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
            self.operate(
                tf!("正在移动 {}…", "Moving {}…", item_count(files.len())),
                move |api| {
                    let mut moved = 0;
                    for file in &files {
                        if api.move_to(&file.id, &folder)? {
                            moved += 1;
                        }
                    }
                    Ok(tf!("已移动 {}到此处", "Moved {} here", item_count(moved)))
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
                        "Google 云盘不能复制文件夹；请用剪切来移动",
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
        let parent = self.path.last().unwrap().clone();
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
        // The field ignores a new value while it is being drawn.
        if std::mem::take(&mut self.clear_filter) {
            let input = self.filter_input.clone();
            window.on_next_frame(move |window, cx| {
                input.update(cx, |input, cx| input.set_value("", window, cx));
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
                div().flex_1().min_w(rem(160.)).max_w(rem(640.)).child(
                    SearchField::new(&self.filter_input)
                        .id("filter")
                        .leading(ui::icon("filter_list", 24., ColorRole::ON_SURFACE_VARIANT)),
                ),
            )
            // The filter takes the room it can use; the route and account
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

    /// Sorting for views without the column headers to click: the grid, and
    /// lists too narrow for the Modified or Size column.
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
                t("修改时间", "Modified"),
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
            let name = self.path[depth].1.clone();
            menu = menu.item(
                MaterialMenuItem::new(name)
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
        let last = self.path.len() - 1;
        // Deep paths in narrow windows show the root, "…" and the last two.
        let collapsed = (!self.wide(760.) && self.path.len() >= 4).then(|| 1..last - 1);
        let mut crumbs = div()
            .flex_1()
            .min_w(rem(120.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(4.));
        for (depth, (_, name)) in self.path.iter().enumerate() {
            if let Some(hidden) = collapsed.clone().filter(|h| h.contains(&depth)) {
                if depth == hidden.start {
                    crumbs = crumbs
                        .child(ui::icon("chevron_right", 24., ColorRole::OUTLINE))
                        .child(self.render_crumb_menu(hidden, cx));
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
                // The root's name follows a language switch.
                .child(if depth == 0 {
                    t("我的云盘", "My Drive").into()
                } else {
                    name.clone()
                });
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
        let paste = self.clipboard.as_ref().map(|(files, cut)| {
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
                    .when_some(paste, |d, b| d.child(b))
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
        let single = picked.len() == 1;
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
            .child(
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
            .child(
                action("copy", Symbol::CONTENT_COPY, t("复制", "Copy"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(false, cx))),
            )
            .child(
                action("cut", Symbol::CONTENT_CUT, t("剪切", "Cut"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(true, cx))),
            )
            .child(
                action("rename", Symbol::EDIT, t("重命名", "Rename"))
                    .disabled(!single)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        if let [file] = this.picked().as_slice() {
                            this.open_dialog(Dialog::Rename(file.clone()), window, cx);
                        }
                    })),
            )
            .child(
                action("trash", Symbol::DELETE, t("移至回收站", "Move to trash")).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        let files = this.picked();
                        this.open_dialog(Dialog::Trash(files), window, cx);
                    }),
                ),
            )
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
                    t("未连接到 Google 云盘", "Not connected to Google Drive"),
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
        if shown.is_empty() {
            let body = if let Some(error) = &self.list_error {
                ui::empty(
                    "error",
                    t("无法列出此文件夹", "Couldn’t list this folder"),
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
                ui::empty(
                    "folder_open",
                    t("这个文件夹是空的", "This folder is empty"),
                    t(
                        "把文件拖到这里即可上传，或点左上角的 + 新建。",
                        "Drag files here to upload them, or click + at the top left to create something.",
                    ),
                )
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
                    count,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let shown: Vec<File> = this.shown().into_iter().cloned().collect();
                        range
                            .filter_map(|ix| shown.get(ix).map(|f| this.render_row(ix, f, cx)))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.rows)
                .flex_1(),
            )
            .into_any_element()
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
        let menu_id = ElementId::from((id.clone(), "menu"));
        MaterialContextMenu::new(id, move |_, _| {
            file_menu(menu_id.clone(), &file, view.clone())
        })
    }

    /// A right click on a folder's empty space offers what + does, and paste.
    fn folder_context_menu(&self, cx: &mut Context<Self>) -> MaterialContextMenu {
        let paste = self.clipboard.is_some();
        let view = cx.entity().downgrade();
        MaterialContextMenu::new("folder-context", move |_, _| {
            folder_menu("folder-context-menu", paste, view.clone())
        })
    }

    /// A press in the listing gives the list the keys, from the filter too.
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
            .when(self.show_modified(), |d| {
                d.child(div().w(rem(COLUMNS[0])).child(column(
                    "sort-modified",
                    t("修改时间", "Modified"),
                    SortKey::Modified,
                )))
            })
            .when(self.show_size(), |d| {
                d.child(div().w(rem(COLUMNS[1])).child(column(
                    "sort-size",
                    t("大小", "Size"),
                    SortKey::Size,
                )))
            })
            .child(div().w(rem(COLUMNS[2])))
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
                            .when_some(state, |d, (icon, ink, _)| {
                                d.child(ui::icon(icon, 18., ink))
                            }),
                    )
                    .when(self.show_modified(), |d| {
                        d.child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                format::modified(file.modified_time.as_deref()),
                            )
                            .w(rem(COLUMNS[0])),
                        )
                    })
                    .when(self.show_size(), |d| {
                        d.child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                format::size(file),
                            )
                            .w(rem(COLUMNS[1])),
                        )
                    })
                    .child(
                        div()
                            .w(rem(COLUMNS[2]))
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
        let menu = file_menu(("row-menu", ix), file, cx.entity().downgrade());
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
                                .child(ui::kind_tile(Kind::of(file), 48.))
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
                                .when_some(state, |d, (icon, ink, _)| {
                                    d.child(ui::icon(icon, 16., ink))
                                }),
                        )
                        .child(ui::text(
                            TypeRole::BodySmall,
                            ColorRole::ON_SURFACE_VARIANT,
                            if file.folder() {
                                format::modified(file.modified_time.as_deref())
                            } else {
                                format!(
                                    "{} · {}",
                                    format::size(file),
                                    format::modified(file.modified_time.as_deref())
                                )
                            },
                        ))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            this.click(&activate, event, window, cx)
                        }))
                }),
            ))
    }
}

/// A file's kind tile; on a selected row, whose container is the tile's own
/// role for folders, it sits on the lowest surface instead.
fn kind_tile(file: &File, size: f32, selected: bool) -> Div {
    ui::kind_tile(Kind::of(file), size).when(selected, |d| {
        d.bg(color(ColorRole::SURFACE_CONTAINER_LOWEST))
    })
}

/// A file's actions, for its ⋮ button and its right-click menu.
fn file_menu(id: impl Into<ElementId>, file: &File, view: WeakEntity<Gdirect>) -> MaterialMenu {
    let item = |label: &'static str,
                icon: &'static str,
                act: fn(&mut Gdirect, File, &mut Window, &mut Context<Gdirect>)| {
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
    let folder = file.folder();
    MaterialMenu::new(id)
        .width(200.)
        .item(if folder {
            item(t("打开", "Open"), "folder_open", |this, f, _, cx| {
                this.activate(&f, cx)
            })
        } else {
            item(t("下载", "Download"), "download", |this, f, _, cx| {
                this.activate(&f, cx)
            })
        })
        .when(folder, |m| {
            m.item(item(
                t("下载整个文件夹", "Download folder"),
                "download",
                |this, f, _, cx| this.download(vec![f], cx),
            ))
        })
        .item(item(
            t("重命名", "Rename"),
            "edit",
            |this, f, window, cx| this.open_dialog(Dialog::Rename(f), window, cx),
        ))
        .item(item(
            t("复制", "Copy"),
            "content_copy",
            |this, f, _, cx| {
                this.selected = vec![f.id];
                this.yank(false, cx)
            },
        ))
        .item(
            item(t("剪切", "Cut"), "content_cut", |this, f, _, cx| {
                this.selected = vec![f.id];
                this.yank(true, cx)
            })
            .divider(true),
        )
        .item(item(
            t("移至回收站", "Move to trash"),
            "delete",
            |this, f, window, cx| this.open_dialog(Dialog::Trash(vec![f]), window, cx),
        ))
}

/// The current folder's actions, for a right click on its empty space.
fn folder_menu(id: impl Into<ElementId>, paste: bool, view: WeakEntity<Gdirect>) -> MaterialMenu {
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
    MaterialMenu::new(id)
        .width(220.)
        .item(item(
            t("新建文件夹", "New folder"),
            "create_new_folder",
            |this, window, cx| this.open_dialog(Dialog::NewFolder, window, cx),
        ))
        .item(item(
            t("上传文件…", "Upload files…"),
            "upload_file",
            |this, _, cx| this.choose_upload(false, cx),
        ))
        .item(
            item(
                t("上传文件夹…", "Upload folder…"),
                "drive_folder_upload",
                |this, _, cx| this.choose_upload(true, cx),
            )
            .divider(true),
        )
        .when(paste, |m| {
            m.item(item(
                t("粘贴到此处", "Paste here"),
                "content_paste",
                |this, _, cx| this.paste(cx),
            ))
        })
        .item(item(t("刷新", "Refresh"), "refresh", |this, _, cx| {
            this.refresh(cx)
        }))
}
