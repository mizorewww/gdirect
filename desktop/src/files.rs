//! 文件: browsing a Drive folder and acting on its files.
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
                            this.say(format!("无法刷新此文件夹：{e}"), cx);
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
                (0, _) => "没有可下载的文件：已在队列中，或无法下载".to_owned(),
                (n, 0) => format!("已加入下载队列 · {n} 个文件"),
                (n, s) => format!("已加入下载队列 · {n} 个文件，跳过 {s} 个"),
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
                    Ok(files) if files.is_empty() => format!("{name} 里没有文件"),
                    Ok(files) => {
                        let (added, _) = this.queues.enqueue(files, this.settings.office, &account);
                        format!("{name}：已加入 {added} 个文件")
                    }
                    Err(e) => format!("无法列出 {name}：{e}"),
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
            format!("正在新建 {name}…"),
            move |api| {
                let file = api.create(&parent, &name, true)?;
                Ok(format!("已新建文件夹 {}", file.name))
            },
            cx,
        );
    }

    pub fn rename(&mut self, file: File, name: String, cx: &mut Context<Self>) {
        self.operate(
            format!("正在重命名 {}…", file.name),
            move |api| {
                let renamed = api.rename(&file.id, &name)?;
                Ok(format!("已重命名为 {}", renamed.name))
            },
            cx,
        );
    }

    pub fn trash(&mut self, files: Vec<File>, cx: &mut Context<Self>) {
        self.selected
            .retain(|id| !files.iter().any(|f| &f.id == id));
        self.operate(
            format!("正在将 {} 项移至回收站…", files.len()),
            move |api| {
                for file in &files {
                    api.trash(&file.id)?;
                }
                Ok(format!("已将 {} 项移至回收站", files.len()))
            },
            cx,
        );
    }

    fn yank(&mut self, cut: bool, cx: &mut Context<Self>) {
        let files = self.picked();
        if files.is_empty() {
            return;
        }
        let message = format!(
            "已{} {} 项 · 打开目标文件夹后粘贴",
            if cut { "剪切" } else { "复制" },
            files.len()
        );
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
                self.say("不能把文件夹移到它自己里面", cx);
                return;
            }
            self.clipboard = None;
            self.operate(
                format!("正在移动 {} 项…", files.len()),
                move |api| {
                    let mut moved = 0;
                    for file in &files {
                        if api.move_to(&file.id, &folder)? {
                            moved += 1;
                        }
                    }
                    Ok(format!("已移动 {moved} 项到此处"))
                },
                cx,
            );
        } else {
            // Drive copies files only; folders would need a recursive copy.
            let (folders, files): (Vec<File>, Vec<File>) =
                files.into_iter().partition(File::folder);
            if files.is_empty() {
                self.say("Google 云盘不能复制文件夹；请用剪切来移动", cx);
                return;
            }
            self.operate(
                format!("正在复制 {} 项…", files.len()),
                move |api| {
                    for file in &files {
                        api.copy(&file.id, &folder)?;
                    }
                    let mut message = format!("已复制 {} 项", files.len());
                    if !folders.is_empty() {
                        message += &format!("；{} 个文件夹无法复制", folders.len());
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
            prompt: Some("上传".into()),
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
        self.say(format!("已加入上传队列 · {n} 项 → {}", parent.1), cx);
        self.pump(cx);
    }

    fn file_state(&self, file: &File) -> Option<(&'static str, ColorRole, &'static str)> {
        Some(match self.queues.file_state(&file.id)? {
            State::Done { .. } => ("check_circle", ColorRole::SUCCESS, "已下载"),
            State::Queued => ("schedule", ColorRole::ON_SURFACE_VARIANT, "排队中"),
            State::Running => ("downloading", ColorRole::PRIMARY, "下载中"),
            State::Paused => ("pause_circle", ColorRole::ON_SURFACE_VARIANT, "已暂停"),
            State::Failed(_) => ("error", ColorRole::ERROR, "下载失败"),
        })
    }

    pub fn render_files(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .accessibility_label("切换账号")
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
                    .child(account.clone()),
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
                MaterialMenuItem::new("添加账号")
                    .leading(ui::symbol("person_add", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.account_menu = false;
                        this.add_account(cx);
                    })),
            )
            .item(
                MaterialMenuItem::new("账号设置")
                    .leading(ui::symbol("manage_accounts", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.account_menu = false;
                        this.pane = crate::app::Pane::Account;
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
                div().flex_1().min_w(rem(240.)).max_w(rem(640.)).child(
                    SearchField::new(&self.filter_input)
                        .id("filter")
                        .leading(ui::icon("filter_list", 24., ColorRole::ON_SURFACE_VARIANT)),
                ),
            )
            .child(div().flex_1())
            .child(match route {
                Some((ip, ms, _)) => MaterialButton::new("route")
                    .variant(ButtonVariant::Outlined)
                    .tone(ButtonTone::Neutral)
                    .accessibility_label(format!("当前线路 {ip}"))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Endpoints, cx)),
                    )
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
                                        .text_color(color(ColorRole::ON_SURFACE_VARIANT))
                                        .child(format!("{ms} ms")),
                                )
                            }),
                    )
                    .into_any_element(),
                None if self.connecting => MaterialButton::new("route")
                    .variant(ButtonVariant::Outlined)
                    .tone(ButtonTone::Neutral)
                    .loading(true, "正在选择线路")
                    .child("正在选择线路…")
                    .into_any_element(),
                None => MaterialButton::new("route")
                    .variant(ButtonVariant::Outlined)
                    .tone(ButtonTone::Neutral)
                    .icon(Symbol::ERROR)
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Endpoints, cx)),
                    )
                    .child("未连接")
                    .into_any_element(),
            })
            .child(
                MenuPopup::new("account-popup", trigger, menu)
                    .anchor(Anchor::TopRight)
                    .open(self.account_menu)
                    .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                        this.account_menu = *open;
                        cx.notify();
                    })),
            )
    }

    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let last = self.path.len() - 1;
        let mut crumbs = div()
            .flex_1()
            .min_w(rem(200.))
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
                    .child(CircularProgress::new("loading", "正在加载").size(20.)),
            );
        }
        let paste = self.clipboard.as_ref().map(|(files, cut)| {
            MaterialButton::new("paste")
                .variant(ButtonVariant::Tonal)
                .icon(Symbol::CONTENT_PASTE)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.paste(cx)))
                .child(format!(
                    "{}到此处 · {} 项",
                    if *cut { "移动" } else { "粘贴" },
                    files.len()
                ))
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
            .when_some(paste, |d, b| d.child(b))
            .when(self.clipboard.is_some(), |d| {
                d.child(
                    IconButton::new("clear-clipboard", Symbol::CLOSE, "清除剪贴板").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.clipboard = None;
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(
                ConnectedButtonGroup::new("view", "视图")
                    .options([("list", "列表视图"), ("grid", "网格视图")])
                    .icon("list", Symbol::VIEW_LIST)
                    .icon("grid", Symbol::GRID_VIEW)
                    .labels(GroupLabels::Never)
                    .selected(if self.grid { "grid" } else { "list" })
                    .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                        this.grid = value.as_ref() == "grid";
                        cx.notify();
                    })),
            )
    }

    fn render_selection(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let picked = self.picked();
        let label: SharedString = match picked.as_slice() {
            [one] => format!("已选择 1 项 · {}", one.name).into(),
            many => format!("已选择 {} 项", many.len()).into(),
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
                action("clear", Symbol::CLOSE, "取消选择").on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.selected.clear();
                        cx.notify();
                    },
                )),
            )
            .child(
                ui::line(TypeRole::TitleMedium, ColorRole::ON_SURFACE, label)
                    .flex_1()
                    .min_w(rem(120.)),
            )
            .child(
                MaterialButton::new("download")
                    .icon(Symbol::DOWNLOAD)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let files = this.picked();
                        this.download(files, cx);
                    }))
                    .child(format!("下载 · {} 连接", self.settings.connections)),
            )
            .child(
                action("copy", Symbol::CONTENT_COPY, "复制")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(false, cx))),
            )
            .child(
                action("cut", Symbol::CONTENT_CUT, "剪切")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.yank(true, cx))),
            )
            .child(
                action("rename", Symbol::EDIT, "重命名")
                    .disabled(!single)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        if let [file] = this.picked().as_slice() {
                            this.open_dialog(Dialog::Rename(file.clone()), window, cx);
                        }
                    })),
            )
            .child(
                action("trash", Symbol::DELETE, "移至回收站").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        let files = this.picked();
                        this.open_dialog(Dialog::Trash(files), window, cx);
                    },
                )),
            )
    }

    fn render_listing(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.api.is_none() {
            let body = if let Some((_, scan)) = &self.scan {
                ui::empty(
                    "radar",
                    format!(
                        "正在扫描 Google 前端 · 已验证 {} / {}",
                        scan.good,
                        scan.target.max(1)
                    ),
                    "首次使用要找出从本机网络可直达的地址，大约需要一分钟。进度和结果见“线路”页。",
                )
            } else if self.connecting {
                ui::empty(
                    "network_ping",
                    "正在选择线路",
                    "首次使用会扫描可直达的 Google 前端，大约需要半分钟。",
                )
            } else {
                ui::empty(
                    "cloud_off",
                    "未连接到 Google 云盘",
                    self.connect_error
                        .clone()
                        .unwrap_or_else(|| "请先在“线路”页扫描可用地址。".into()),
                )
                .child(
                    MaterialButton::new("reconnect")
                        .icon(Symbol::REFRESH)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.connect(cx)))
                        .child("重新连接"),
                )
            };
            return div().flex_1().child(body).into_any_element();
        }
        let shown = self.shown();
        if shown.is_empty() {
            let body = if let Some(error) = &self.list_error {
                ui::empty("error", "无法列出此文件夹", error.clone())
            } else if self.loading {
                ui::empty("hourglass_empty", "正在加载…", "")
            } else if !self.filter.is_empty() {
                ui::empty(
                    "search_off",
                    "没有匹配的文件",
                    format!("当前文件夹里没有名称包含“{}”的项目", self.filter),
                )
            } else {
                ui::empty(
                    "folder_open",
                    "这个文件夹是空的",
                    "把文件拖到这里即可上传，或点左上角的 + 新建。",
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(column("sort-name", "名称", SortKey::Name)),
            )
            .child(div().w(rem(COLUMNS[0])).child(column(
                "sort-modified",
                "修改时间",
                SortKey::Modified,
            )))
            .child(
                div()
                    .w(rem(COLUMNS[1]))
                    .child(column("sort-size", "大小", SortKey::Size)),
            )
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
                    .child(
                        ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            format::modified(file.modified_time.as_deref()),
                        )
                        .w(rem(COLUMNS[0])),
                    )
                    .child(
                        ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            format::size(file),
                        )
                        .w(rem(COLUMNS[1])),
                    )
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
        let trigger = IconButton::new(("more", ix), Symbol::MORE_VERT, "更多操作").on_click(
            cx.listener(move |this, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if matches!(event, ClickEvent::Keyboard(_)) {
                    this.row_menu = if this.row_menu.as_deref() == Some(trigger_id.as_str()) {
                        None
                    } else {
                        Some(trigger_id.clone())
                    };
                    cx.notify();
                }
            }),
        );
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
                if file.folder() { "打开" } else { "下载" },
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
                    "下载整个文件夹",
                    "download",
                    file.clone(),
                    |this, f, _, cx| this.download(vec![f], cx),
                ))
            })
            .item(item(
                "重命名",
                "edit",
                file.clone(),
                |this, f, window, cx| this.open_dialog(Dialog::Rename(f), window, cx),
            ))
            .item(item(
                "复制",
                "content_copy",
                file.clone(),
                |this, f, _, cx| {
                    this.selected = vec![f.id];
                    this.yank(false, cx)
                },
            ))
            .item(
                item("剪切", "content_cut", file.clone(), |this, f, _, cx| {
                    this.selected = vec![f.id];
                    this.yank(true, cx)
                })
                .divider(true),
            )
            .item(item(
                "移至回收站",
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
