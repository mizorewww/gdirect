//! 设置: a category list beside the chosen category's settings.
use crate::{
    app::{ACCENTS, Dialog, Gdirect, Pane},
    format,
    i18n::{Language, t},
    settings::{Identity, Theme},
    tf, ui,
};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;

impl Gdirect {
    fn choose_dir(&mut self, mount: bool, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t("选择", "Choose").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await
                && let Some(dir) = paths.pop()
            {
                let _ = this.update(cx, |this, cx| {
                    if mount {
                        this.settings.mount_path = dir;
                    } else {
                        this.settings.download_dir = dir;
                    }
                    this.settings.save(&this.db);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn choose_ca(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("选择 CA 证书（PEM）", "Choose CA certificate (PEM)").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update(cx, |this, cx| {
                    this.settings.ca_cert = Some(path);
                    this.settings.save(&this.db);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.save(&self.db);
        cx.notify();
    }

    pub fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account_sub = match &self.account {
            Some(a) => {
                let n = self.accounts.len();
                if n == 1 {
                    tf!("{a} · 共 {n} 个账号", "{a} · {n} account")
                } else {
                    tf!("{a} · 共 {n} 个账号", "{a} · {n} accounts")
                }
            }
            None => t("未登录", "Not signed in").into(),
        };
        // Pane, title, supporting text, icon and tint of each category.
        type Category = (Pane, &'static str, String, &'static str, Tint);
        let groups: [Vec<Category>; 3] = [
            vec![(
                Pane::Account,
                t("账号", "Account"),
                account_sub,
                "account_circle",
                Tint::Blue,
            )],
            vec![
                (
                    Pane::Transfer,
                    t("下载与上传", "Downloads & uploads"),
                    {
                        let dir = format::home(&self.settings.download_dir);
                        let n = self.settings.connections;
                        if n == 1 {
                            tf!("{dir} · {n} 个连接", "{dir} · {n} connection")
                        } else {
                            tf!("{dir} · {n} 个连接", "{dir} · {n} connections")
                        }
                    },
                    "download",
                    Tint::Green,
                ),
                (
                    Pane::Network,
                    t("网络", "Network"),
                    match self.settings.pinned {
                        Some(ip) => tf!(
                            "固定 {ip} · TLS 身份 · 证书",
                            "Pinned to {ip} · TLS · certificates"
                        ),
                        None => t(
                            "自动选择线路 · TLS 身份 · 证书",
                            "Auto route · TLS · certificates",
                        )
                        .into(),
                    },
                    "network_ping",
                    Tint::Teal,
                ),
                (
                    Pane::Mount,
                    t("挂载", "Mount"),
                    match &self.mount {
                        Some(m) => {
                            let path = format::home(&m.path);
                            tf!("已挂载到 {path}", "Mounted at {path}")
                        }
                        None => t("未挂载", "Not mounted").into(),
                    },
                    "hard_drive",
                    Tint::Purple,
                ),
            ],
            vec![
                (
                    Pane::Appearance,
                    t("外观", "Appearance"),
                    t("主题、强调色、语言", "Theme, accent color, language").into(),
                    "palette",
                    Tint::Orange,
                ),
                (
                    Pane::About,
                    t("关于", "About"),
                    format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                    "info",
                    Tint::Amber,
                ),
            ],
        ];
        // Wide windows show the list beside the pane; narrow ones show one at a
        // time, with a way back to the list.
        let two_panes = self.wide(960.);
        let list = div()
            .id("settings-list")
            .when(two_panes, |d| d.w(rem(360.)).flex_none())
            .when(!two_panes, |d| d.flex_1())
            .h_full()
            .overflow_y_scroll();
        let list = list.child(
            div()
                .p(rem(16.))
                .flex()
                .flex_col()
                .gap(rem(16.))
                .child(
                    ui::text(
                        TypeRole::HeadlineSmall,
                        ColorRole::ON_SURFACE,
                        t("设置", "Settings"),
                    )
                    .px(rem(8.))
                    .pt(rem(8.)),
                )
                .children(groups.into_iter().map(|group| {
                    segmented_list(group.into_iter().map(|(pane, title, sub, icon, tint)| {
                        ListItem::new(title, title)
                            .supporting(sub)
                            .leading(ListLeading::Tinted(ui::symbol(icon, true), tint, 40.))
                            .selected(two_panes && self.pane == pane)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.pane = pane;
                                this.pane_open = true;
                                this.reload_accounts();
                                cx.notify();
                            }))
                    }))
                })),
        );
        let detail = match self.pane {
            Pane::Account => self.pane_account(cx).into_any_element(),
            Pane::Transfer => self.pane_transfer(cx).into_any_element(),
            Pane::Network => self.pane_network(cx).into_any_element(),
            Pane::Mount => self.pane_mount(cx).into_any_element(),
            Pane::Appearance => self.pane_appearance(cx).into_any_element(),
            Pane::About => self.pane_about(cx).into_any_element(),
        };
        let back = (!two_panes).then(|| {
            div()
                .flex()
                .items_center()
                .gap(rem(4.))
                .ml(rem(-12.))
                .child(
                    IconButton::new(
                        "settings-back",
                        Symbol::ARROW_BACK,
                        t("返回设置", "Back to settings"),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.pane_open = false;
                        cx.notify();
                    })),
                )
                .child(ui::text(
                    TypeRole::TitleMedium,
                    ColorRole::ON_SURFACE_VARIANT,
                    t("设置", "Settings"),
                ))
        });
        let detail = div()
            .id("settings-detail")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .bg(color(ColorRole::SURFACE_CONTAINER_LOW))
            .child(
                div()
                    .max_w(rem(720.))
                    .p(rem(if two_panes { 32. } else { 24. }))
                    .flex()
                    .flex_col()
                    .gap(rem(24.))
                    .children(back)
                    .child(detail),
            );
        div()
            .size_full()
            .flex()
            .when(two_panes || !self.pane_open, |d| d.child(list))
            .when(two_panes || self.pane_open, |d| d.child(detail))
    }

    fn pane_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .accounts
            .iter()
            .enumerate()
            .map(|(ix, email)| {
                let is_default = self.account.as_ref() == Some(email);
                let mounted = self
                    .db
                    .setting(&gdirect::store::Store::mount_key(email))
                    .ok()
                    .flatten()
                    .is_some();
                let switch = email.clone();
                let leave = email.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(rem(16.))
                    .px(rem(20.))
                    .py(rem(12.))
                    .child(
                        Avatar::new(("avatar", ix), email.clone()).monogram(
                            email
                                .chars()
                                .next()
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_default(),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(ui::line(
                                TypeRole::BodyLarge,
                                ColorRole::ON_SURFACE,
                                email.clone(),
                            ))
                            .child(ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                if mounted {
                                    t("文件夹缓存 · 已挂载", "Folder cache · Mounted")
                                } else {
                                    t("文件夹缓存", "Folder cache")
                                },
                            )),
                    )
                    .child(if is_default {
                        status_chip(StatusKind::Tonal)
                            .child(t("默认", "Default"))
                            .into_any_element()
                    } else {
                        material_button(("make-default", ix), ButtonVariant::Text)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.switch_account(switch.clone(), cx)
                            }))
                            .child(t("设为默认", "Make default"))
                            .into_any_element()
                    })
                    .child(
                        IconButton::new(
                            ("logout", ix),
                            Symbol::LOGOUT,
                            t("退出此账号", "Sign out of this account"),
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.open_dialog(Dialog::Logout(leave.clone()), window, cx)
                            },
                        )),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(
                TypeRole::HeadlineSmall,
                ColorRole::ON_SURFACE,
                t("账号", "Account"),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("Google 账号", "Google accounts")))
                    .child(ui::group(rows))
                    .child(
                        div().pt(rem(4.)).child(
                            MaterialButton::new("add-account")
                                .variant(ButtonVariant::Tonal)
                                .icon(Symbol::PERSON_ADD)
                                .loading(self.signing_in, t("正在等待浏览器登录", "Waiting for sign-in in the browser"))
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.add_account(cx)),
                                )
                                .child(t("添加账号", "Add account")),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("OAuth 客户端", "OAuth client")))
                    .child(ui::group(vec![ui::setting_row(
                        "key",
                        if self.oauth_client {
                            t("桌面应用客户端 · 已保存", "Desktop app client · Saved")
                        } else {
                            t("未保存客户端", "No client saved")
                        },
                        Some(
                            t(
                                "应用处于测试阶段时，每个账号都要加为测试用户，登录 7 天后过期",
                                "While the app is in testing, add each account as a test user; sign-ins expire after 7 days",
                            )
                            .into(),
                        ),
                        Some(
                            material_button("client-json", ButtonVariant::Text)
                                .disabled(self.signing_in)
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.choose_client(cx)
                                    }),
                                )
                                .child(t("更换 JSON", "Replace JSON"))
                                .into_any_element(),
                        ),
                    )])),
            )
    }

    fn pane_transfer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let connections = div()
            .flex()
            .flex_col()
            .gap(rem(4.))
            .px(rem(20.))
            .py(rem(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem(16.))
                    .child(ui::icon("stacks", 24., ColorRole::ON_SURFACE_VARIANT))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(rem(2.))
                            .child(ui::text(
                                TypeRole::BodyLarge,
                                ColorRole::ON_SURFACE,
                                t("默认连接数", "Connections per download"),
                            ))
                            .child(ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                t(
                                    "每个文件用多少条连接并行下载，1–16",
                                    "Parallel connections used for each file, 1–16",
                                ),
                            )),
                    )
                    .child(ui::text(
                        TypeRole::HeadlineSmall,
                        ColorRole::PRIMARY,
                        s.connections.to_string(),
                    )),
            )
            .child(
                div().pl(rem(40.)).child(
                    MaterialSlider::new(
                        &self.connections,
                        t("默认连接数", "Connections per download"),
                    )
                    .id("connections")
                    .stops(true)
                    .value_indicator(true)
                    .format(|v| format!("{}", v.round() as u8).into()),
                ),
            );
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, t("下载与上传", "Downloads & uploads")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("下载", "Downloads")))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "folder_open",
                            t("下载目录", "Download folder"),
                            Some(format::home(&s.download_dir).into()),
                            Some(
                                material_button("download-dir", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_dir(false, cx)))
                                    .child(t("更改", "Change"))
                                    .into_any_element(),
                            ),
                        ),
                        connections.into_any_element(),
                        ui::setting_row(
                            "article",
                            t("Google 文档导出格式", "Google Docs export format"),
                            Some(t("文档、表格、幻灯片下载时转换成的格式", "Docs, Sheets and Slides are converted to this format when downloaded").into()),
                            Some(
                                ConnectedButtonGroup::new("export", t("导出格式", "Export format"))
                                    .options([("pdf", "PDF"), ("office", "Office")])
                                    .selected(if s.office { "office" } else { "pdf" })
                                    .on_change(cx.listener(|this, v: &SharedString, _, cx| {
                                        this.settings.office = v.as_ref() == "office";
                                        this.save_settings(cx);
                                    }))
                                    .into_any_element(),
                            ),
                        ),
                        ui::setting_row(
                            "verified",
                            t("下载后校验 MD5", "Verify MD5 after download"),
                            Some(t("与 Drive 记录的校验和比对；已存在且一致的文件会跳过", "Compares against the checksum Drive records; matching existing files are skipped").into()),
                            Some(
                                MaterialSwitch::new("md5", t("下载后校验 MD5", "Verify MD5 after download"))
                                    .checked(s.md5)
                                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                                        this.settings.md5 = *on;
                                        this.save_settings(cx);
                                    }))
                                    .into_any_element(),
                            ),
                        ),
                    ])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("上传", "Uploads")))
                    .child(notice_banner(
                        "upload-info",
                        NoticeKind::Info,
                        t(
                            "上传始终新建文件，不会覆盖同名文件。上传文件夹时会保留目录结构，跳过符号链接；中断后重新上传同一文件夹即可续传。",
                            "Uploads always create new files and never overwrite files with the same name. Folder uploads keep their structure and skip symbolic links; to resume an interrupted upload, upload the same folder again.",
                        ),
                    )),
            )
    }

    fn pane_network(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, t("网络", "Network")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("线路选择", "Route")))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "keep",
                            t("固定地址", "Pinned address"),
                            Some(match s.pinned {
                                Some(ip) => tf!("所有请求只走 {ip}", "All requests go through {ip}").into(),
                                None => t(
                                    "按延迟自动选择，失效时切换到下一个",
                                    "Chosen automatically by latency; switches to the next one if it fails",
                                )
                                .into(),
                            }),
                            s.pinned.map(|_| {
                                material_button("unpin", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.pinned = None;
                                        this.settings.save(&this.db);
                                        this.connect(cx);
                                    }))
                                    .child(t("取消固定", "Unpin"))
                                    .into_any_element()
                            }),
                        ),
                        div()
                            .flex()
                            .flex_col()
                            .gap(rem(12.))
                            .child(ui::setting_row(
                                "badge",
                                t("TLS 身份", "TLS identity"),
                                Some(t(
                                    "自动：先用真实 SNI，被重置时改用伪装身份并按 IP 记住。“不发送”只用于下载",
                                    "Automatic: tries the real SNI first, switches to a disguised identity when reset and remembers it per IP. “Don’t send” applies to downloads only",
                                ).into()),
                                None,
                            ))
                            .child(
                                div().pl(rem(60.)).pb(rem(16.)).child(
                                    ConnectedButtonGroup::new("identity", t("TLS 身份", "TLS identity"))
                                        .options([
                                            ("auto", t("自动", "Automatic")),
                                            ("forged", t("始终伪装", "Always disguise")),
                                            ("none", t("不发送", "Don’t send")),
                                        ])
                                        .selected(match s.identity {
                                            Identity::Auto => "auto",
                                            Identity::Forged => "forged",
                                            Identity::None => "none",
                                        })
                                        .on_change(cx.listener(|this, v: &SharedString, _, cx| {
                                            this.settings.identity = match v.as_ref() {
                                                "forged" => Identity::Forged,
                                                "none" => Identity::None,
                                                _ => Identity::Auto,
                                            };
                                            this.settings.save(&this.db);
                                            this.connect(cx);
                                        })),
                                ),
                            )
                            .into_any_element(),
                    ])),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label(t("登录与证书", "Sign-in & certificates")))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "vpn_lock",
                            t("登录时使用的代理", "Sign-in proxy"),
                            Some(t(
                                "跟随系统代理（HTTPS_PROXY 等）。只用于登录和账号验证；传输始终直连已扫描的地址",
                                "Follows the system proxy (HTTPS_PROXY and similar). Used only for sign-in and account checks; transfers always connect directly to scanned addresses",
                            ).into()),
                            None,
                        ),
                        ui::setting_row(
                            "verified_user",
                            t("证书", "Certificate"),
                            Some(match &s.ca_cert {
                                Some(path) => {
                                    let path = format::home(path);
                                    tf!("下载使用自定义 CA：{path}", "Downloads use a custom CA: {path}").into()
                                }
                                None => t(
                                    "内置 Google GTS 根证书，不使用系统 CA",
                                    "Built-in Google GTS root certificates; the system CA isn’t used",
                                )
                                .into(),
                            }),
                            Some(if s.ca_cert.is_some() {
                                material_button("ca", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.ca_cert = None;
                                        this.save_settings(cx);
                                    }))
                                    .child(t("恢复内置", "Use built-in"))
                                    .into_any_element()
                            } else {
                                material_button("ca", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_ca(cx)))
                                    .child(t("使用自定义 CA…", "Use custom CA…"))
                                    .into_any_element()
                            }),
                        ),
                    ])),
            )
            .child(notice_banner(
                "tun-tip",
                NoticeKind::Info,
                t(
                    "开着 Clash/Mihomo TUN？把 Google IP 段设为 DIRECT 并关闭 fake-ip，或添加规则 PROCESS-NAME,gdirect-desktop,DIRECT。",
                    "Using Clash/Mihomo TUN? Set Google’s IP ranges to DIRECT and turn off fake-ip, or add the rule PROCESS-NAME,gdirect-desktop,DIRECT.",
                ),
            ))
    }

    fn pane_mount(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let mounted = self.mount.as_ref();
        let (title, sub) = match (mounted, &self.account) {
            (Some(m), Some(account)) => {
                let path = format::home(&m.path);
                (
                    tf!("已挂载 · {account}", "Mounted · {account}"),
                    tf!(
                        "位置 {path} · 写入先进缓存再后台上传",
                        "At {path} · Writes go to the cache first, then upload in the background"
                    ),
                )
            }
            _ => (
                t("未挂载", "Not mounted").to_owned(),
                t(
                    "挂载后可以在访达中像本地磁盘一样使用云盘",
                    "Once mounted, use Google Drive in Finder like a local disk",
                )
                .to_owned(),
            ),
        };
        let status_bg = if mounted.is_some() {
            ColorRole::TERTIARY_CONTAINER
        } else {
            ColorRole::SURFACE_CONTAINER_LOWEST
        };
        let status_fg = if mounted.is_some() {
            ColorRole::ON_TERTIARY_CONTAINER
        } else {
            ColorRole::ON_SURFACE_VARIANT
        };
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, t("挂载", "Mount")))
            .child(
                div()
                    .rounded(rem(24.))
                    .bg(color(status_bg))
                    .p(rem(20.))
                    .flex()
                    .items_center()
                    .gap(rem(16.))
                    .child(ui::symbol("hard_drive", true).size(rem(32.)).color(color(status_fg)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(rem(2.))
                            .child(ui::text(TypeRole::TitleLarge, status_fg, title))
                            .child(ui::text(TypeRole::BodyMedium, status_fg, sub)),
                    )
                    .when(mounted.is_some(), |d| {
                        let path = mounted.map(|m| m.path.clone()).unwrap();
                        d.child(
                            material_button("reveal-mount", ButtonVariant::Text)
                                .on_click(move |_, _, cx| cx.open_with_system(&path))
                                .child(t("在访达中打开", "Open in Finder")),
                        )
                    })
                    .child(
                        MaterialButton::new("mount")
                            .variant(if mounted.is_some() { ButtonVariant::Outlined } else { ButtonVariant::Filled })
                            .disabled(self.account.is_none())
                            .loading(self.mount_busy, if mounted.is_some() { t("正在卸载", "Unmounting") } else { t("正在挂载", "Mounting") })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_mount(cx)))
                            .child(if mounted.is_some() { t("卸载", "Unmount") } else { t("挂载", "Mount") }),
                    ),
            )
            .child(ui::group(vec![
                ui::setting_row(
                    "folder_open",
                    t("挂载位置", "Mount location"),
                    Some(format::home(&s.mount_path).into()),
                    Some(
                        material_button("mount-dir", ButtonVariant::Text)
                            .disabled(mounted.is_some())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_dir(true, cx)))
                            .child(t("更改", "Change"))
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "lock",
                    t("只读", "Read-only"),
                    Some(t("下次挂载时生效", "Takes effect the next time you mount").into()),
                    Some(
                        MaterialSwitch::new("read-only", t("只读挂载", "Read-only mount"))
                            .checked(s.read_only)
                            .on_change(cx.listener(|this, on: &bool, _, cx| {
                                this.settings.read_only = *on;
                                this.save_settings(cx);
                            }))
                            .into_any_element(),
                    ),
                ),
            ]))
            .child(ui::text(
                TypeRole::BodySmall,
                ColorRole::ON_SURFACE_VARIANT,
                t(
                    "挂载通过 rclone 1.75.2+ 进行，经由扫描到的线路访问 Google。macOS 使用内置 NFS，无需 FUSE。",
                    "Mounting uses rclone 1.75.2+ and reaches Google over the scanned routes. macOS uses its built-in NFS, so no FUSE is needed.",
                ),
            ))
    }

    fn pane_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let swatches = div()
            .flex()
            .gap(rem(12.))
            .children(ACCENTS.iter().enumerate().map(|(ix, (seed, name))| {
                let selected = s.accent == ix;
                let rgb = gpui_kit::rgb(*seed);
                let swatch = div()
                    .id(("accent", ix))
                    .size(rem(44.))
                    .rounded_full()
                    .bg(rgb)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.border_3().border_color(color(ColorRole::ON_SURFACE))
                    })
                    .when(selected, |d| {
                        d.child(
                            ui::symbol("check", false)
                                .size(rem(20.))
                                .color(gpui_kit::white()),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.settings.accent = ix;
                        this.settings.save(&this.db);
                        this.apply_theme(window, cx);
                        cx.notify();
                    }));
                MaterialTooltip::plain(("accent-tip", ix), *name).trigger(swatch)
            }));
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(
                TypeRole::HeadlineSmall,
                ColorRole::ON_SURFACE,
                t("外观", "Appearance"),
            ))
            .child(ui::group(vec![
                ui::setting_row(
                    "dark_mode",
                    t("主题", "Theme"),
                    None,
                    Some(
                        ConnectedButtonGroup::new("theme", t("主题", "Theme"))
                            .options([
                                ("system", t("跟随系统", "System")),
                                ("light", t("浅色", "Light")),
                                ("dark", t("深色", "Dark")),
                            ])
                            .icon("system", Symbol::BRIGHTNESS_AUTO)
                            .icon("light", Symbol::LIGHT_MODE)
                            .icon("dark", Symbol::DARK_MODE)
                            .selected(match s.theme {
                                Theme::System => "system",
                                Theme::Light => "light",
                                Theme::Dark => "dark",
                            })
                            .on_change(cx.listener(|this, v: &SharedString, window, cx| {
                                this.settings.theme = match v.as_ref() {
                                    "light" => Theme::Light,
                                    "dark" => Theme::Dark,
                                    _ => Theme::System,
                                };
                                this.settings.save(&this.db);
                                this.apply_theme(window, cx);
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "translate",
                    t("语言", "Language"),
                    None,
                    Some(
                        ConnectedButtonGroup::new("language", t("语言", "Language"))
                            .options([
                                ("system", t("跟随系统", "System")),
                                ("zh", "中文"),
                                ("en", "English"),
                            ])
                            .selected(match s.language {
                                Language::System => "system",
                                Language::Chinese => "zh",
                                Language::English => "en",
                            })
                            .on_change(cx.listener(|this, v: &SharedString, _, cx| {
                                this.settings.language = match v.as_ref() {
                                    "zh" => Language::Chinese,
                                    "en" => Language::English,
                                    _ => Language::System,
                                };
                                this.settings.save(&this.db);
                                crate::i18n::apply(this.settings.language);
                                cx.refresh_windows();
                            }))
                            .into_any_element(),
                    ),
                ),
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(12.))
                    .child(ui::setting_row(
                        "palette",
                        t("强调色", "Accent color"),
                        None,
                        None,
                    ))
                    .child(div().pl(rem(60.)).pb(rem(20.)).child(swatches))
                    .into_any_element(),
            ]))
    }

    fn pane_about(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let db_path = self.db_path.clone();
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(
                TypeRole::HeadlineSmall,
                ColorRole::ON_SURFACE,
                t("关于", "About"),
            ))
            .child(ui::group(vec![
                ui::setting_row(
                    "system_update",
                    format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                    Some(t("桌面客户端 · Material 3", "Desktop client · Material 3").into()),
                    Some(
                        material_button("releases", ButtonVariant::Text)
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/mizorewww/gdirect/releases")
                            })
                            .child(t("检查更新", "Check for updates"))
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "database",
                    t("数据文件", "Data file"),
                    Some(format::home(&self.db_path).into()),
                    Some(
                        material_button("reveal-db", ButtonVariant::Text)
                            .on_click(move |_, _, cx| cx.reveal_path(&db_path))
                            .child(t("在访达中显示", "Show in Finder"))
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "code",
                    t("源代码与更新日志", "Source code & changelog"),
                    Some("github.com/mizorewww/gdirect".into()),
                    Some(
                        IconButton::new(
                            "source",
                            Symbol::OPEN_IN_NEW,
                            t("在浏览器中打开", "Open in browser"),
                        )
                        .on_click(|_, _, cx| cx.open_url("https://github.com/mizorewww/gdirect"))
                        .into_any_element(),
                    ),
                ),
            ]))
    }
}
