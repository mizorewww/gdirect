//! 传输: the download and upload queues.
use crate::{
    app::{Gdirect, Page},
    format::{self, Kind},
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
            Action::RemoveDownload(id) => self.queues.downloads.retain(|d| d.id != id),
            Action::RemoveUpload(id) => self.queues.uploads.retain(|u| u.id != id),
        }
        cx.notify();
    }

    /// A queued item's status: it runs only while its account is active.
    fn waiting_for(&self, account: &str) -> String {
        if self.account.as_deref() == Some(account) {
            "排队中".into()
        } else {
            format!("等待切换到 {account}")
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
                status: "正在列出".into(),
                status_ink: ColorRole::PRIMARY,
                fraction: None,
                muted: false,
                detail: format!(
                    "已找到 {count} 个文件 · 保存到 {}",
                    format::home(&self.settings.download_dir.join(name))
                ),
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
                        format!(
                            " · 剩余 {}",
                            format::duration((total - job.stats.done) / speed)
                        )
                    } else {
                        String::new()
                    };
                    (
                        "下载中".to_owned(),
                        ColorRole::PRIMARY,
                        if total > 0 {
                            format!(
                                "{} / {} · {}{left} · {} 个连接",
                                format::bytes(job.stats.done),
                                format::bytes(total),
                                format::speed(speed),
                                job.stats.connections.max(0)
                            )
                        } else {
                            "正在连接…".into()
                        },
                        vec![(
                            "stop_circle",
                            "取消（保留已下载部分）",
                            Action::CancelDownload(job.id),
                        )],
                    )
                }
                State::Queued => (
                    self.waiting_for(&job.account),
                    ColorRole::ON_SURFACE_VARIANT,
                    format!(
                        "{}保存到 {}",
                        job.size
                            .map(|s| format!("{} · ", format::bytes(s)))
                            .unwrap_or_default(),
                        format::home(&job.output)
                    ),
                    vec![("close", "从队列移除", Action::RemoveDownload(job.id))],
                ),
                State::Paused => (
                    "已取消 · 可续传".to_owned(),
                    ColorRole::ON_SURFACE_VARIANT,
                    format!(
                        "已保留 {} · 继续时从断点开始",
                        format::bytes(job.stats.done)
                    ),
                    vec![
                        ("play_arrow", "继续", Action::ResumeDownload(job.id)),
                        (
                            "close",
                            "从队列移除（已下载部分留在磁盘上）",
                            Action::RemoveDownload(job.id),
                        ),
                    ],
                ),
                State::Failed(error) => (
                    "失败".to_owned(),
                    ColorRole::ERROR,
                    error.clone(),
                    vec![
                        ("refresh", "重试", Action::ResumeDownload(job.id)),
                        ("close", "从队列移除", Action::RemoveDownload(job.id)),
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
                        "上传中".to_owned(),
                        ColorRole::PRIMARY,
                        format!(
                            "{} / {} 个文件 · {} / {} · {} · 中断后重新上传即可续传",
                            p.files_done,
                            p.files_total,
                            format::bytes(p.bytes_done),
                            format::bytes(p.bytes_total),
                            format::speed(job.speed)
                        ),
                        Vec::new(),
                    ),
                    State::Running => (
                        "上传中".to_owned(),
                        ColorRole::PRIMARY,
                        format!(
                            "{} / {} · {}",
                            format::bytes(p.bytes_done),
                            format::bytes(p.bytes_total),
                            format::speed(job.speed)
                        ),
                        Vec::new(),
                    ),
                    State::Queued => (
                        self.waiting_for(&job.account),
                        ColorRole::ON_SURFACE_VARIANT,
                        if job.folder {
                            "递归上传，保留目录结构".to_owned()
                        } else {
                            job.path
                                .metadata()
                                .map(|m| {
                                    format!("{} · 始终新建文件，不覆盖同名", format::bytes(m.len()))
                                })
                                .unwrap_or_default()
                        },
                        vec![("close", "从队列移除", Action::RemoveUpload(job.id))],
                    ),
                    State::Failed(error) => (
                        "失败".to_owned(),
                        ColorRole::ERROR,
                        error.clone(),
                        vec![("close", "移除", Action::RemoveUpload(job.id))],
                    ),
                    State::Paused | State::Done { .. } => unreachable!(),
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
        let dl_count = self.listing.len() + self.queues.downloads.len();
        let up_count = self.queues.uploads.len();
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
                        ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, "传输")
                            .flex_1()
                            .min_w(rem(120.)),
                    )
                    .child(self.render_stepper(cx))
                    .child(
                        ConnectedButtonGroup::new("transfer-kind", "传输类型")
                            .options([
                                ("dl", format!("下载 · {dl_count}")),
                                ("up", format!("上传 · {up_count}")),
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
                    "连接数",
                )
                .mr(rem(4.)),
            )
            .child(
                IconButton::new("conn-down", Symbol::REMOVE, "减少连接数")
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
                IconButton::new("conn-up", Symbol::ADD, "增加连接数")
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
                    format!("正在上传 · {}", format::speed(job.speed)),
                    format!(
                        "队列中还有 {} 项 · 上传始终新建文件",
                        queued.saturating_sub(1)
                    ),
                ),
                None => (
                    "没有进行中的上传".into(),
                    "把文件拖到“文件”页，或用左上角的 + 上传".into(),
                ),
            }
        } else {
            match self.queues.active_download() {
                Some(job) => (
                    format!("正在下载 · {}", format::speed(job.stats.speed)),
                    format!(
                        "队列中还有 {} 项 · 连接数对下一个下载生效并保存为默认值",
                        queued.saturating_sub(1)
                    ),
                ),
                None => (
                    "没有进行中的下载".into(),
                    format!("保存到 {}", format::home(&self.settings.download_dir)),
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
                        "真实 SNI".to_owned()
                    } else {
                        format!("伪装 · {sni}")
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
                    .child(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_PRIMARY_CONTAINER,
                        sub,
                    )),
            )
            .when(failed, |d| {
                d.child(
                    MaterialButton::new("retry-failed")
                        .variant(ButtonVariant::Outlined)
                        .icon(Symbol::REFRESH)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            let n = this.queues.retry_failed();
                            this.say(format!("已重新排队 {n} 项"), cx);
                            this.pump(cx);
                        }))
                        .child("重试失败项"),
                )
            })
            .child(if uploads {
                MaterialButton::new("open-target")
                    .icon(Symbol::FOLDER_OPEN)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Page::Files, cx)))
                    .child("在云盘中查看")
            } else {
                MaterialButton::new("open-target")
                    .icon(Symbol::FOLDER_OPEN)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let dir = this.settings.download_dir.clone();
                        let _ = std::fs::create_dir_all(&dir);
                        cx.open_with_system(&dir);
                    }))
                    .child("打开下载目录")
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
                            "队列是空的"
                        } else {
                            "队列是空的 · 在“文件”页选择文件后点“下载”"
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
                    .child(ui::section_label("队列 · 依次进行"))
                    .when(has_queued, |d| {
                        d.child(
                            material_button("clear-queued", ButtonVariant::Text)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.queues.clear_queued(uploads);
                                    cx.notify();
                                }))
                                .child("移除未开始的项"),
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
                                ui::line(
                                    TypeRole::TitleMedium,
                                    ColorRole::ON_SURFACE,
                                    row.name.clone(),
                                )
                                .flex_1(),
                            )
                            .child(ui::text(TypeRole::LabelLarge, row.status_ink, row.status)),
                    )
                    .child(
                        LinearProgress::new(id.clone(), format!("{} 进度", row.name))
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
                        .child("在云盘中查看")
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
                        .child("打开")
                        .into_any_element(),
                    Some(
                        IconButton::new(
                            ("reveal-dl", job.id as usize),
                            Symbol::FOLDER_OPEN,
                            "在访达中显示",
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
                    .child(ui::section_label("已完成"))
                    .child(
                        material_button("clear-done", ButtonVariant::Text)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.queues.clear_finished(uploads);
                                cx.notify();
                            }))
                            .child("清除"),
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
                .child(ui::line(TypeRole::TitleMedium, ColorRole::ON_SURFACE, name))
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
                        .child(ui::line(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            detail,
                        )),
                ),
        )
        .child(primary)
        .children(secondary)
        .into_any_element()
}
