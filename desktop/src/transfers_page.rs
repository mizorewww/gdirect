//! 传输: the download and upload queues.
use crate::{
    app::{Gdirect, Page},
    format::{self, Kind},
    i18n::t,
    tf,
    transfers::State,
    ui,
};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::sync::atomic::Ordering;

/// One queue row as drawn, built from a download, an upload or a listing.
struct Row {
    key: (&'static str, u64),
    kind: Kind,
    name: String,
    status: String,
    status_ink: ColorRole,
    fraction: Option<f32>,
    muted: bool,
    detail: String,
    actions: Vec<(&'static str, &'static str, Action)>,
}

#[derive(Clone, Copy)]
enum Action {
    CancelDownload(u64),
    ResumeDownload(u64),
    RemoveDownload(u64),
    CancelUpload(u64),
    ResumeUpload(u64),
    RemoveUpload(u64),
}

impl Gdirect {
    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::CancelDownload(id) => {
                if let Some(job) = self.queues.download(id)
                    && let Some(cancel) = &job.cancel
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Action::ResumeDownload(id) => {
                if let Some(job) = self.queues.download(id) {
                    job.state = State::Queued;
                }
                self.pump(cx);
            }
            // A running job is cancelled first; its row must stay until it ends.
            Action::RemoveDownload(id) => self
                .queues
                .downloads
                .retain(|d| d.id != id || d.state == State::Running),
            Action::CancelUpload(id) => {
                if let Some(job) = self.queues.upload(id)
                    && let Some(cancel) = &job.cancel
                {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Action::ResumeUpload(id) => {
                if let Some(job) = self.queues.upload(id) {
                    job.state = State::Queued;
                }
                self.pump(cx);
            }
            Action::RemoveUpload(id) => self
                .queues
                .uploads
                .retain(|u| u.id != id || u.state == State::Running),
        }
        cx.notify();
    }

    /// A queued item's status: it runs only while its account is active.
    fn waiting_for(&self, account: &str) -> String {
        if self.account.as_deref() == Some(account) {
            t("排队中", "Queued").into()
        } else {
            tf!("等待切换到 {account}", "Waiting for {account}")
        }
    }

    fn download_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .listing
            .iter()
            .enumerate()
            .map(|(i, (name, count))| Row {
                key: ("listing", i as u64),
                kind: Kind::Folder,
                name: format!("{name}/"),
                status: t("正在列出", "Listing").into(),
                status_ink: ColorRole::PRIMARY,
                fraction: None,
                muted: false,
                detail: {
                    let s = if *count == 1 { "" } else { "s" };
                    tf!(
                        "已找到 {count} 个文件 · 保存到 {}",
                        "Found {count} file{s} · saving to {}",
                        format::home(&self.settings.download_dir.join(name))
                    )
                },
                actions: Vec::new(),
            })
            .collect();
        for job in self.queues.downloads.iter().filter(|d| d.state.pending()) {
            let total = if job.stats.total > 0 {
                job.stats.total
            } else {
                job.size.unwrap_or(0)
            };
            let (status, ink, detail, actions) = match &job.state {
                State::Running => {
                    let speed = job.stats.speed;
                    let left = if speed > 0 && total > job.stats.done {
                        tf!(
                            " · 剩余 {}",
                            " · {} left",
                            format::duration((total - job.stats.done) / speed)
                        )
                    } else {
                        String::new()
                    };
                    (
                        t("下载中", "Downloading").to_owned(),
                        ColorRole::PRIMARY,
                        if total > 0 {
                            let connections = job.stats.connections.max(0);
                            let s = if connections == 1 { "" } else { "s" };
                            tf!(
                                "{} / {} · {}{left} · {connections} 个连接",
                                "{} / {} · {}{left} · {connections} connection{s}",
                                format::bytes(job.stats.done),
                                format::bytes(total),
                                format::speed(speed)
                            )
                        } else {
                            t("正在连接…", "Connecting…").into()
                        },
                        vec![(
                            "stop_circle",
                            t("取消（保留已下载部分）", "Cancel (keep downloaded data)"),
                            Action::CancelDownload(job.id),
                        )],
                    )
                }
                State::Queued => (
                    self.waiting_for(&job.account),
                    ColorRole::ON_SURFACE_VARIANT,
                    tf!(
                        "{}保存到 {}",
                        "{}saving to {}",
                        job.size
                            .map(|s| format!("{} · ", format::bytes(s)))
                            .unwrap_or_default(),
                        format::home(&job.output)
                    ),
                    vec![(
                        "close",
                        t("从队列移除", "Remove from queue"),
                        Action::RemoveDownload(job.id),
                    )],
                ),
                State::Paused => (
                    t("已取消 · 可续传", "Cancelled · can resume").to_owned(),
                    ColorRole::ON_SURFACE_VARIANT,
                    tf!(
                        "已保留 {} · 继续时从断点开始",
                        "{} kept · resumes where it stopped",
                        format::bytes(job.stats.done)
                    ),
                    vec![
                        (
                            "play_arrow",
                            t("继续", "Resume"),
                            Action::ResumeDownload(job.id),
                        ),
                        (
                            "close",
                            t(
                                "从队列移除（已下载部分留在磁盘上）",
                                "Remove from queue (downloaded data stays on disk)",
                            ),
                            Action::RemoveDownload(job.id),
                        ),
                    ],
                ),
                State::Failed(error) => (
                    t("失败", "Failed").to_owned(),
                    ColorRole::ERROR,
                    error.clone(),
                    vec![
                        (
                            "refresh",
                            t("重试", "Retry"),
                            Action::ResumeDownload(job.id),
                        ),
                        (
                            "close",
                            t("从队列移除", "Remove from queue"),
                            Action::RemoveDownload(job.id),
                        ),
                    ],
                ),
                State::Done { .. } => unreachable!(),
            };
            rows.push(Row {
                key: ("download", job.id),
                kind: Kind::of(&job.file),
                name: job
                    .output
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| job.file.name.clone()),
                status,
                status_ink: ink,
                fraction: Some(job.fraction()),
                muted: job.state != State::Running,
                detail,
                actions,
            });
        }
        rows
    }

    fn upload_rows(&self) -> Vec<Row> {
        self.queues
            .uploads
            .iter()
            .filter(|u| u.state.pending())
            .map(|job| {
                let p = &job.progress;
                let (status, ink, detail, actions) = match &job.state {
                    State::Running if job.folder => (
                        t("上传中", "Uploading").to_owned(),
                        ColorRole::PRIMARY,
                        tf!(
                            "{} / {} 个文件 · {} / {} · {} · 中断后重新上传即可续传",
                            "{} / {} files · {} / {} · {} · upload again to resume if interrupted",
                            p.files_done,
                            p.files_total,
                            format::bytes(p.bytes_done),
                            format::bytes(p.bytes_total),
                            format::speed(job.speed)
                        ),
                        vec![(
                            "stop_circle",
                            t(
                                "取消（已上传的部分可续传）",
                                "Cancel (uploaded data can resume)",
                            ),
                            Action::CancelUpload(job.id),
                        )],
                    ),
                    State::Running => (
                        t("上传中", "Uploading").to_owned(),
                        ColorRole::PRIMARY,
                        format!(
                            "{} / {} · {}",
                            format::bytes(p.bytes_done),
                            format::bytes(p.bytes_total),
                            format::speed(job.speed)
                        ),
                        vec![(
                            "stop_circle",
                            t(
                                "取消（已上传的部分可续传）",
                                "Cancel (uploaded data can resume)",
                            ),
                            Action::CancelUpload(job.id),
                        )],
                    ),
                    State::Queued => (
                        self.waiting_for(&job.account),
                        ColorRole::ON_SURFACE_VARIANT,
                        if job.folder {
                            t(
                                "递归上传，保留目录结构",
                                "Uploads recursively, keeping the folder structure",
                            )
                            .to_owned()
                        } else {
                            job.path
                                .metadata()
                                .map(|m| {
                                    tf!(
                                        "{} · 始终新建文件，不覆盖同名",
                                        "{} · always creates a new file, never overwrites",
                                        format::bytes(m.len())
                                    )
                                })
                                .unwrap_or_default()
                        },
                        vec![(
                            "close",
                            t("从队列移除", "Remove from queue"),
                            Action::RemoveUpload(job.id),
                        )],
                    ),
                    State::Failed(error) => (
                        t("失败", "Failed").to_owned(),
                        ColorRole::ERROR,
                        error.clone(),
                        vec![
                            ("refresh", t("重试", "Retry"), Action::ResumeUpload(job.id)),
                            (
                                "close",
                                t("从队列移除", "Remove from queue"),
                                Action::RemoveUpload(job.id),
                            ),
                        ],
                    ),
                    State::Paused => (
                        t("已取消 · 可续传", "Cancelled · can resume").to_owned(),
                        ColorRole::ON_SURFACE_VARIANT,
                        tf!(
                            "已上传 {} / {} · 继续时从中断处开始",
                            "{} / {} uploaded · resumes where it stopped",
                            format::bytes(p.bytes_done),
                            format::bytes(p.bytes_total)
                        ),
                        vec![
                            (
                                "play_arrow",
                                t("继续", "Resume"),
                                Action::ResumeUpload(job.id),
                            ),
                            (
                                "close",
                                t("从队列移除", "Remove from queue"),
                                Action::RemoveUpload(job.id),
                            ),
                        ],
                    ),
                    State::Done { .. } => unreachable!(),
                };
                Row {
                    key: ("upload", job.id),
                    kind: Kind::from_path(&job.path),
                    name: format!(
                        "{}{} → {}",
                        job.name(),
                        if job.folder { "/" } else { "" },
                        job.parent.1
                    ),
                    status,
                    status_ink: ink,
                    fraction: Some(job.fraction()),
                    muted: job.state != State::Running,
                    detail,
                    actions,
                }
            })
            .collect()
    }

    pub fn render_transfers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let uploads = self.show_uploads;
        // The tabs count what is still to do; finished items are listed below.
        let dl_count = self.listing.len()
            + self
                .queues
                .downloads
                .iter()
                .filter(|d| d.state.pending())
                .count();
        let up_count = self
            .queues
            .uploads
            .iter()
            .filter(|u| u.state.pending())
            .count();
        let rows = if uploads {
            self.upload_rows()
        } else {
            self.download_rows()
        };
        let failed = self
            .queues
            .downloads
            .iter()
            .any(|d| matches!(d.state, State::Failed(_)))
            || self
                .queues
                .uploads
                .iter()
                .any(|u| matches!(u.state, State::Failed(_)));

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(rem(16.))
                    .pt(rem(24.))
                    .px(rem(24.))
                    .pb(rem(16.))
                    .child(
                        ui::text(
                            TypeRole::HeadlineSmall,
                            ColorRole::ON_SURFACE,
                            t("传输", "Transfers"),
                        )
                        .flex_1()
                        .min_w(rem(120.)),
                    )
                    .child(self.render_stepper(cx))
                    .child(
                        ConnectedButtonGroup::new("transfer-kind", t("传输类型", "Transfer type"))
                            .options([
                                ("dl", tf!("下载 · {dl_count}", "Downloads · {dl_count}")),
                                ("up", tf!("上传 · {up_count}", "Uploads · {up_count}")),
                            ])
                            .icon("dl", Symbol::DOWNLOAD)
                            .icon("up", Symbol::UPLOAD)
                            .selected(if uploads { "up" } else { "dl" })
                            .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                                this.show_uploads = value.as_ref() == "up";
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("transfers-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .px(rem(24.))
                            .pb(rem(24.))
                            .flex()
                            .flex_col()
                            .gap(rem(24.))
                            .child(self.render_summary(failed, rows.len(), cx))
                            .child(self.render_queue(rows, cx))
                            .child(self.render_done(cx)),
                    ),
            )
    }

    fn render_stepper(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.settings.connections;
        div()
            .flex()
            .items_center()
            .gap(rem(4.))
            .h(rem(40.))
            .pl(rem(16.))
            .pr(rem(4.))
            .rounded_full()
            .bg(color(ColorRole::SURFACE_CONTAINER_HIGH))
            .child(
                ui::text(
                    TypeRole::LabelLarge,
                    ColorRole::ON_SURFACE_VARIANT,
                    t("连接数", "Connections"),
                )
                .mr(rem(4.)),
            )
            .child(
                IconButton::new(
                    "conn-down",
                    Symbol::REMOVE,
                    t("减少连接数", "Fewer connections"),
                )
                .button_size(ButtonSize::ExtraSmall)
                .disabled(n <= 1)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.set_connections(this.settings.connections - 1, window, cx)
                })),
            )
            .child(
                ui::text(TypeRole::TitleMedium, ColorRole::ON_SURFACE, n.to_string())
                    .min_w(rem(24.))
                    .text_center(),
            )
            .child(
                IconButton::new("conn-up", Symbol::ADD, t("增加连接数", "More connections"))
                    .button_size(ButtonSize::ExtraSmall)
                    .disabled(n >= 16)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.set_connections(this.settings.connections + 1, window, cx)
                    })),
            )
    }

    pub fn set_connections(&mut self, n: u8, window: &mut Window, cx: &mut Context<Self>) {
        let n = n.clamp(1, 16);
        self.settings.connections = n;
        self.settings.save(&self.db);
        self.connections
            .update(cx, |slider, cx| slider.set_value(n as f32, window, cx));
        cx.notify();
    }

    fn render_summary(
        &self,
        failed: bool,
        queued: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let uploads = self.show_uploads;
        let (title, sub) = if uploads {
            match self.queues.active_upload() {
                Some(job) => (
                    tf!("正在上传 · {}", "Uploading · {}", format::speed(job.speed)),
                    {
                        let left = queued.saturating_sub(1);
                        let s = if left == 1 { "" } else { "s" };
                        tf!(
                            "队列中还有 {left} 项 · 上传始终新建文件",
                            "{left} more item{s} in the queue · uploads always create new files"
                        )
                    },
                ),
                None => (
                    t("没有进行中的上传", "No uploads in progress").into(),
                    t(
                        "把文件拖到“文件”页，或用左上角的 + 上传",
                        "Drop files on the Files page, or use + at the top left to upload",
                    )
                    .into(),
                ),
            }
        } else {
            match self.queues.active_download() {
                Some(job) => (
                    tf!(
                        "正在下载 · {}",
                        "Downloading · {}",
                        format::speed(job.stats.speed)
                    ),
                    {
                        let left = queued.saturating_sub(1);
                        let s = if left == 1 { "" } else { "s" };
                        tf!(
                            "队列中还有 {left} 项 · 连接数对下一个下载生效并保存为默认值",
                            "{left} more item{s} in the queue · connection count applies to the next download and is saved as the default"
                        )
                    },
                ),
                None => (
                    t("没有进行中的下载", "No downloads in progress").into(),
                    tf!(
                        "保存到 {}",
                        "Saving to {}",
                        format::home(&self.settings.download_dir)
                    ),
                ),
            }
        };
        let route = self.current_route().map(|(ip, ms, sni)| {
            div()
                .flex()
                .items_center()
                .gap(rem(8.))
                .px(rem(10.))
                .py(rem(4.))
                .rounded(RADIUS_SMALL)
                .bg(color(ColorRole::SURFACE_CONTAINER_LOWEST).opacity(0.55))
                .child(ui::icon(
                    "network_ping",
                    18.,
                    ColorRole::ON_PRIMARY_CONTAINER,
                ))
                .child(ui::mono(ip))
                .child(format!(
                    "{}· {}",
                    ms.map(|ms| format!("· {ms} ms ")).unwrap_or_default(),
                    if sni.is_empty() {
                        t("真实 SNI", "Real SNI").to_owned()
                    } else {
                        tf!("伪装 · {sni}", "Disguised · {sni}")
                    }
                ))
        });
        div()
            .rounded(rem(24.))
            .bg(color(ColorRole::PRIMARY_CONTAINER))
            .text_color(color(ColorRole::ON_PRIMARY_CONTAINER))
            .py(rem(20.))
            .px(rem(24.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(16.))
            .child(
                div()
                    .flex_1()
                    .min_w(rem(260.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(rem(8.))
                    .child(ui::text(
                        TypeRole::TitleLarge,
                        ColorRole::ON_PRIMARY_CONTAINER,
                        title,
                    ))
                    .children(route)
                    // Names and paths: their hyphens in Roboto.
                    .child(ui::latin(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_PRIMARY_CONTAINER,
                        sub,
                    ))),
            )
            .when(failed, |d| {
                d.child(
                    MaterialButton::new("retry-failed")
                        .variant(ButtonVariant::Outlined)
                        .icon(Symbol::REFRESH)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            let n = this.queues.retry_failed();
                            let s = if n == 1 { "" } else { "s" };
                            this.say(tf!("已重新排队 {n} 项", "Requeued {n} item{s}"), cx);
                            this.pump(cx);
                        }))
                        .child(t("重试失败项", "Retry failed")),
                )
            })
            .child(if uploads {
                MaterialButton::new("open-target")
                    .icon(Symbol::FOLDER_OPEN)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Files, cx)))
                    .child(t("在云端硬盘中查看", "Show in Google Drive"))
            } else {
                MaterialButton::new("open-target")
                    .icon(Symbol::FOLDER_OPEN)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let dir = this.settings.download_dir.clone();
                        let _ = std::fs::create_dir_all(&dir);
                        cx.open_with_system(&dir);
                    }))
                    .child(t("打开下载目录", "Open download folder"))
            })
    }

    fn render_queue(&self, rows: Vec<Row>, cx: &mut Context<Self>) -> impl IntoElement {
        let uploads = self.show_uploads;
        let has_queued = if uploads {
            self.queues.uploads.iter().any(|u| u.state == State::Queued)
        } else {
            self.queues
                .downloads
                .iter()
                .any(|d| d.state == State::Queued)
        };
        let body = if rows.is_empty() {
            ui::group(vec![
                div()
                    .py(rem(20.))
                    .px(rem(20.))
                    .child(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        if uploads {
                            t("队列是空的", "The queue is empty")
                        } else {
                            t(
                                "队列是空的。在“文件”页选择文件后点“下载”。",
                                "The queue is empty. Select files on the Files page and choose Download.",
                            )
                        },
                    ))
                    .into_any_element(),
            ])
        } else {
            ui::group(
                rows.into_iter()
                    .map(|row| self.render_queue_row(row, cx))
                    .collect(),
            )
        };
        div()
            .flex()
            .flex_col()
            .gap(rem(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .min_h(rem(40.))
                    .child(ui::section_label(t(
                        "队列 · 依次进行",
                        "Queue · one at a time",
                    )))
                    .when(has_queued, |d| {
                        d.child(
                            material_button("clear-queued", ButtonVariant::Text)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.queues.clear_queued(uploads);
                                    cx.notify();
                                }))
                                .child(t("移除未开始的项", "Remove queued items")),
                        )
                    }),
            )
            .child(body)
    }

    fn render_queue_row(&self, row: Row, cx: &mut Context<Self>) -> AnyElement {
        let id: ElementId = (row.key.0, row.key.1 as usize).into();
        div()
            .flex()
            .items_center()
            .gap(rem(16.))
            .py(rem(14.))
            .pl(rem(16.))
            .pr(rem(8.))
            .child(ui::kind_tile(row.kind, 40.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rem(12.))
                            .child(
                                ui::name_line(
                                    TypeRole::TitleMedium,
                                    ColorRole::ON_SURFACE,
                                    row.name.clone(),
                                )
                                .flex_1(),
                            )
                            .child(ui::text(TypeRole::LabelLarge, row.status_ink, row.status)),
                    )
                    .child(
                        LinearProgress::new(id.clone(), tf!("{} 进度", "{} progress", row.name))
                            .value(row.fraction)
                            .muted(row.muted),
                    )
                    .child(ui::line(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        row.detail,
                    )),
            )
            .children(
                row.actions
                    .into_iter()
                    .enumerate()
                    .map(|(i, (icon, label, action))| {
                        let symbol = Symbol::named(icon).unwrap_or(Symbol::CLOSE);
                        let upload = if row.key.0 == "upload" { 1 << 32 } else { 0 };
                        IconButton::new(
                            ("act", (row.key.1 as usize + upload) * 2 + i),
                            symbol,
                            label,
                        )
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.act(action, cx)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_done(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let uploads = self.show_uploads;
        let mut rows: Vec<AnyElement> = Vec::new();
        if uploads {
            for job in self.queues.uploads.iter().rev() {
                let State::Done { at, note } = &job.state else {
                    continue;
                };
                let parent = job.parent.clone();
                rows.push(done_row(
                    ("up-done", job.id as usize),
                    Kind::from_path(&job.path),
                    format!("{} → {}", job.name(), job.parent.1),
                    format!("{note} · {}", format::clock(*at)),
                    MaterialButton::new(("show-up", job.id as usize))
                        .variant(ButtonVariant::Tonal)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.show_folder(parent.0.clone(), parent.1.clone(), cx)
                        }))
                        .child(t("在云端硬盘中查看", "Show in Google Drive"))
                        .into_any_element(),
                    None,
                ));
            }
        } else {
            for job in self.queues.downloads.iter().rev() {
                let State::Done { at, note } = &job.state else {
                    continue;
                };
                let open = job.output.clone();
                let reveal = job.output.clone();
                rows.push(done_row(
                    ("dl-done", job.id as usize),
                    Kind::of(&job.file),
                    job.output
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    format!("{note} · {}", format::clock(*at)),
                    MaterialButton::new(("open-dl", job.id as usize))
                        .variant(ButtonVariant::Tonal)
                        .on_click(move |_, _, cx| cx.open_with_system(&open))
                        .child(t("打开", "Open"))
                        .into_any_element(),
                    Some(
                        IconButton::new(
                            ("reveal-dl", job.id as usize),
                            Symbol::FOLDER_OPEN,
                            t("在访达中显示", "Show in Finder"),
                        )
                        .on_click(move |_, _, cx| cx.reveal_path(&reveal))
                        .into_any_element(),
                    ),
                ));
            }
        }
        if rows.is_empty() {
            return div().into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap(rem(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .min_h(rem(40.))
                    .child(ui::section_label(t("已完成", "Completed")))
                    .child(
                        material_button("clear-done", ButtonVariant::Text)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.queues.clear_finished(uploads);
                                cx.notify();
                            }))
                            .child(t("清除", "Clear")),
                    ),
            )
            .child(ui::group(rows))
            .into_any_element()
    }
}

fn done_row(
    id: impl Into<ElementId>,
    kind: Kind,
    name: String,
    detail: String,
    primary: AnyElement,
    secondary: Option<AnyElement>,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rem(16.))
        .py(rem(8.))
        .pl(rem(16.))
        .pr(rem(8.))
        .child(ui::kind_tile(kind, 40.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(rem(2.))
                .child(ui::name_line(
                    TypeRole::TitleMedium,
                    ColorRole::ON_SURFACE,
                    name,
                ))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rem(6.))
                        .child(
                            ui::symbol("verified", true)
                                .size(rem(16.))
                                .color(color(ColorRole::SUCCESS)),
                        )
                        // The note can be long (a skipped download says why); it wraps.
                        .child(
                            ui::text(TypeRole::BodyMedium, ColorRole::ON_SURFACE_VARIANT, detail)
                                .flex_1()
                                .min_w_0(),
                        ),
                ),
        )
        .child(primary)
        .children(secondary)
        .into_any_element()
}
