//! 文件: browsing a Drive folder and acting on its files.
use crate::i18n::t;
use crate::tf;
use crate::{
    app::{Dialog, Gdirect, Page, SortKey},
    format::{self, Kind},
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
                            this.say(
                                tf!("无法刷新此文件夹：{e}", "Couldn’t refresh this folder: {e}"),
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
        self.list_error = None;
        self.filter.clear();
        self.clear_filter = true;
        self.rows.scroll_to_item(0, ScrollStrategy::Top);
        self.refresh(cx);
    }

    fn toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.selected.iter().position(|s| s == id) {
            self.selected.remove(i);
        } else {
            self.selected.push(id.to_owned());
        }
        cx.notify();
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
                    Err(e) => tf!("无法列出 {name}：{e}", "Couldn’t list {name}: {e}"),
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
                    Err(e) => this.say(e, cx),
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
        div()
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

    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let last = self.path.len() - 1;
        let mut crumbs = div()
            .flex_1()
            .min_w(rem(120.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(4.));
        for (depth, (_, name)) in self.path.iter().enumerate() {
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
            return div().flex_1().child(body).into_any_element();
        }
        if self.grid {
            return self.render_grid(shown, cx).into_any_element();
        }
        let count = shown.len();
        drop(shown);
        let header = self.render_header(cx);
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .px(rem(8.))
            .pb(rem(16.))
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
        let state = self.file_state(file);
        let id = file.id.clone();
        let activate = file.clone();
        div()
            .id(("row", ix))
            .h(rem(58.))
            .pt(rem(2.))
            .child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .pl(rem(16.))
                    .rounded(rem(28.))
                    .when(selected, |d| d.bg(color(ColorRole::SECONDARY_CONTAINER)))
                    .when(!selected, |d| {
                        d.hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.06)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(rem(16.))
                            .child(ui::kind_tile(Kind::of(file), 32.))
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
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.selected.retain(|s| *s != activate.id);
                    this.activate(&activate, cx);
                } else {
                    this.toggle(&id, cx);
                }
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
        let item =
            |label: &'static str,
             icon: &'static str,
             f: File,
             act: fn(&mut Gdirect, File, &mut Window, &mut Context<Gdirect>)| {
                MaterialMenuItem::new(label)
                    .leading(ui::symbol(icon, false))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.row_menu = None;
                        act(this, f.clone(), window, cx);
                    }))
            };
        let menu = MaterialMenu::new(("row-menu", ix))
            .width(200.)
            .item(item(
                if file.folder() {
                    t("打开", "Open")
                } else {
                    t("下载", "Download")
                },
                if file.folder() {
                    "folder_open"
                } else {
                    "download"
                },
                file.clone(),
                |this, f, _, cx| this.activate(&f, cx),
            ))
            .when(file.folder(), |m| {
                m.item(item(
                    t("下载整个文件夹", "Download folder"),
                    "download",
                    file.clone(),
                    |this, f, _, cx| this.download(vec![f], cx),
                ))
            })
            .item(item(
                t("重命名", "Rename"),
                "edit",
                file.clone(),
                |this, f, window, cx| this.open_dialog(Dialog::Rename(f), window, cx),
            ))
            .item(item(
                t("复制", "Copy"),
                "content_copy",
                file.clone(),
                |this, f, _, cx| {
                    this.selected = vec![f.id];
                    this.yank(false, cx)
                },
            ))
            .item(
                item(
                    t("剪切", "Cut"),
                    "content_cut",
                    file.clone(),
                    |this, f, _, cx| {
                        this.selected = vec![f.id];
                        this.yank(true, cx)
                    },
                )
                .divider(true),
            )
            .item(item(
                t("移至回收站", "Move to trash"),
                "delete",
                file.clone(),
                |this, f, window, cx| this.open_dialog(Dialog::Trash(vec![f]), window, cx),
            ));
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
            .pb(rem(16.))
            .child(div().flex().flex_wrap().gap(rem(12.)).children(
                shown.into_iter().enumerate().map(|(ix, file)| {
                    let selected = self.selected.contains(&file.id);
                    let id = file.id.clone();
                    let activate = file.clone();
                    let state = self.file_state(file);
                    div()
                        .id(("tile", ix))
                        .w(rem(176.))
                        .p(rem(12.))
                        .flex()
                        .flex_col()
                        .gap(rem(12.))
                        .rounded(rem(16.))
                        .bg(color(if selected {
                            ColorRole::SECONDARY_CONTAINER
                        } else {
                            ColorRole::SURFACE_CONTAINER_LOW
                        }))
                        .hover(|s| s.bg(color(ColorRole::SURFACE_CONTAINER_HIGH)))
                        .child(
                            div()
                                .h(rem(96.))
                                .rounded(rem(12.))
                                .bg(color(ColorRole::SURFACE_CONTAINER_LOWEST))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(ui::kind_tile(Kind::of(file), 48.)),
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
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.click_count() >= 2 {
                                this.selected.retain(|s| *s != activate.id);
                                this.activate(&activate, cx);
                            } else {
                                this.toggle(&id, cx);
                            }
                        }))
                }),
            ))
    }
}
