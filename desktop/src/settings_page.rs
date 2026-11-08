//! 设置: a category list beside the chosen category's settings.
use crate::{
    app::{ACCENTS, Dialog, Gdirect, Pane},
    format,
    settings::{Identity, Theme},
    ui,
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
            prompt: Some("选择".into()),
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
            prompt: Some("选择 CA 证书（PEM）".into()),
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
            Some(a) => format!("{a} · 共 {} 个账号", self.accounts.len()),
            None => "未登录".into(),
        };
        // Pane, title, supporting text, icon and tint of each category.
        type Category = (Pane, &'static str, String, &'static str, Tint);
        let groups: [Vec<Category>; 3] = [
            vec![(
                Pane::Account,
                "账号",
                account_sub,
                "account_circle",
                Tint::Blue,
            )],
            vec![
                (
                    Pane::Transfer,
                    "下载与上传",
                    format!(
                        "{} · {} 个连接",
                        format::home(&self.settings.download_dir),
                        self.settings.connections
                    ),
                    "download",
                    Tint::Green,
                ),
                (
                    Pane::Network,
                    "网络",
                    match self.settings.pinned {
                        Some(ip) => format!("固定 {ip} · TLS 身份 · 证书"),
                        None => "自动选择线路 · TLS 身份 · 证书".into(),
                    },
                    "network_ping",
                    Tint::Teal,
                ),
                (
                    Pane::Mount,
                    "挂载",
                    match &self.mount {
                        Some(m) => format!("已挂载到 {}", format::home(&m.path)),
                        None => "未挂载".into(),
                    },
                    "hard_drive",
                    Tint::Purple,
                ),
            ],
            vec![
                (
                    Pane::Appearance,
                    "外观",
                    "主题、强调色".into(),
                    "palette",
                    Tint::Orange,
                ),
                (
                    Pane::About,
                    "关于",
                    format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                    "info",
                    Tint::Amber,
                ),
            ],
        ];
        let list = div()
            .id("settings-list")
            .w(rem(360.))
            .flex_none()
            .h_full()
            .overflow_y_scroll();
        let list = list.child(
            div()
                .p(rem(16.))
                .flex()
                .flex_col()
                .gap(rem(16.))
                .child(
                    ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, "设置")
                        .px(rem(8.))
                        .pt(rem(8.)),
                )
                .children(groups.into_iter().map(|group| {
                    segmented_list(group.into_iter().map(|(pane, title, sub, icon, tint)| {
                        ListItem::new(title, title)
                            .supporting(sub)
                            .leading(ListLeading::Tinted(ui::symbol(icon, true), tint, 40.))
                            .selected(self.pane == pane)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.pane = pane;
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
        div().size_full().flex().child(list).child(
            div()
                .id("settings-detail")
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_y_scroll()
                .bg(color(ColorRole::SURFACE_CONTAINER_LOW))
                .child(
                    div()
                        .max_w(rem(720.))
                        .p(rem(32.))
                        .flex()
                        .flex_col()
                        .gap(rem(24.))
                        .child(detail),
                ),
        )
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
                                    "文件夹缓存 · 已挂载"
                                } else {
                                    "文件夹缓存"
                                },
                            )),
                    )
                    .child(if is_default {
                        status_chip(StatusKind::Tonal)
                            .child("默认")
                            .into_any_element()
                    } else {
                        material_button(("make-default", ix), ButtonVariant::Text)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.switch_account(switch.clone(), cx)
                            }))
                            .child("设为默认")
                            .into_any_element()
                    })
                    .child(
                        IconButton::new(("logout", ix), Symbol::LOGOUT, "退出此账号").on_click(
                            cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_dialog(Dialog::Logout(leave.clone()), window, cx)
                            }),
                        ),
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
                "账号",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label("Google 账号"))
                    .child(ui::group(rows))
                    .child(
                        div().pt(rem(4.)).child(
                            MaterialButton::new("add-account")
                                .variant(ButtonVariant::Tonal)
                                .icon(Symbol::PERSON_ADD)
                                .loading(self.signing_in, "正在等待浏览器登录")
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.add_account(cx)),
                                )
                                .child("添加账号"),
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label("OAuth 客户端"))
                    .child(ui::group(vec![ui::setting_row(
                        "key",
                        if self.oauth_client {
                            "桌面应用客户端 · 已保存"
                        } else {
                            "未保存客户端"
                        },
                        Some(
                            "应用处于测试阶段时，每个账号都要加为测试用户，登录 7 天后过期".into(),
                        ),
                        Some(
                            material_button("client-json", ButtonVariant::Text)
                                .disabled(self.signing_in)
                                .on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.choose_client(cx)
                                    }),
                                )
                                .child("更换 JSON")
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
                                "默认连接数",
                            ))
                            .child(ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                "每个文件用多少条连接并行下载，1–16",
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
                    MaterialSlider::new(&self.connections, "默认连接数")
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
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, "下载与上传"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label("下载"))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "folder_open",
                            "下载目录",
                            Some(format::home(&s.download_dir).into()),
                            Some(
                                material_button("download-dir", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_dir(false, cx)))
                                    .child("更改")
                                    .into_any_element(),
                            ),
                        ),
                        connections.into_any_element(),
                        ui::setting_row(
                            "article",
                            "Google 文档导出格式",
                            Some("文档、表格、幻灯片下载时转换成的格式".into()),
                            Some(
                                ConnectedButtonGroup::new("export", "导出格式")
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
                            "下载后校验 MD5",
                            Some("与 Drive 记录的校验和比对；已存在且一致的文件会跳过".into()),
                            Some(
                                MaterialSwitch::new("md5", "下载后校验 MD5")
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
                    .child(ui::section_label("上传"))
                    .child(notice_banner(
                        "upload-info",
                        NoticeKind::Info,
                        "上传始终新建文件，不会覆盖同名文件。上传文件夹时会保留目录结构，跳过符号链接；中断后重新上传同一文件夹即可续传。",
                    )),
            )
    }

    fn pane_network(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, "网络"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::section_label("线路选择"))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "keep",
                            "固定地址",
                            Some(match s.pinned {
                                Some(ip) => format!("所有请求只走 {ip}").into(),
                                None => "按延迟自动选择，失效时切换到下一个".into(),
                            }),
                            s.pinned.map(|_| {
                                material_button("unpin", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.pinned = None;
                                        this.settings.save(&this.db);
                                        this.connect(cx);
                                    }))
                                    .child("取消固定")
                                    .into_any_element()
                            }),
                        ),
                        div()
                            .flex()
                            .flex_col()
                            .gap(rem(12.))
                            .child(ui::setting_row(
                                "badge",
                                "TLS 身份",
                                Some("自动：先用真实 SNI，被重置时改用伪装身份并按 IP 记住。“不发送”只用于下载".into()),
                                None,
                            ))
                            .child(
                                div().pl(rem(60.)).pb(rem(16.)).child(
                                    ConnectedButtonGroup::new("identity", "TLS 身份")
                                        .options([("auto", "自动"), ("forged", "始终伪装"), ("none", "不发送")])
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
                    .child(ui::section_label("登录与证书"))
                    .child(ui::group(vec![
                        ui::setting_row(
                            "vpn_lock",
                            "登录时使用的代理",
                            Some("跟随系统代理（HTTPS_PROXY 等）。只用于登录和账号验证；传输始终直连已扫描的地址".into()),
                            None,
                        ),
                        ui::setting_row(
                            "verified_user",
                            "证书",
                            Some(match &s.ca_cert {
                                Some(path) => format!("下载使用自定义 CA：{}", format::home(path)).into(),
                                None => "内置 Google GTS 根证书，不使用系统 CA".into(),
                            }),
                            Some(if s.ca_cert.is_some() {
                                material_button("ca", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.settings.ca_cert = None;
                                        this.save_settings(cx);
                                    }))
                                    .child("恢复内置")
                                    .into_any_element()
                            } else {
                                material_button("ca", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_ca(cx)))
                                    .child("使用自定义 CA…")
                                    .into_any_element()
                            }),
                        ),
                    ])),
            )
            .child(notice_banner(
                "tun-tip",
                NoticeKind::Info,
                "开着 Clash/Mihomo TUN？把 Google IP 段设为 DIRECT 并关闭 fake-ip，或添加规则 PROCESS-NAME,gdirect-desktop,DIRECT。",
            ))
    }

    fn pane_mount(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let mounted = self.mount.as_ref();
        let (title, sub) = match (mounted, &self.account) {
            (Some(m), Some(account)) => (
                format!("已挂载 · {account}"),
                format!("位置 {} · 写入先进缓存再后台上传", format::home(&m.path)),
            ),
            _ => (
                "未挂载".to_owned(),
                "挂载后可以在访达中像本地磁盘一样使用云盘".to_owned(),
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
            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, "挂载"))
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
                                .child("在访达中打开"),
                        )
                    })
                    .child(
                        MaterialButton::new("mount")
                            .variant(if mounted.is_some() { ButtonVariant::Outlined } else { ButtonVariant::Filled })
                            .disabled(self.account.is_none())
                            .loading(self.mount_busy, if mounted.is_some() { "正在卸载" } else { "正在挂载" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_mount(cx)))
                            .child(if mounted.is_some() { "卸载" } else { "挂载" }),
                    ),
            )
            .child(ui::group(vec![
                ui::setting_row(
                    "folder_open",
                    "挂载位置",
                    Some(format::home(&s.mount_path).into()),
                    Some(
                        material_button("mount-dir", ButtonVariant::Text)
                            .disabled(mounted.is_some())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_dir(true, cx)))
                            .child("更改")
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "lock",
                    "只读",
                    Some("下次挂载时生效".into()),
                    Some(
                        MaterialSwitch::new("read-only", "只读挂载")
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
                "挂载通过 rclone 1.75.2+ 进行，经由扫描到的线路访问 Google。macOS 使用内置 NFS，无需 FUSE。",
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
                "外观",
            ))
            .child(ui::group(vec![
                ui::setting_row(
                    "dark_mode",
                    "主题",
                    None,
                    Some(
                        ConnectedButtonGroup::new("theme", "主题")
                            .options([("system", "跟随系统"), ("light", "浅色"), ("dark", "深色")])
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
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(12.))
                    .child(ui::setting_row("palette", "强调色", None, None))
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
                "关于",
            ))
            .child(ui::group(vec![
                ui::setting_row(
                    "system_update",
                    format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                    Some("桌面客户端 · Material 3".into()),
                    Some(
                        material_button("releases", ButtonVariant::Text)
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/mizorewww/gdirect/releases")
                            })
                            .child("检查更新")
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "database",
                    "数据文件",
                    Some(format::home(&self.db_path).into()),
                    Some(
                        material_button("reveal-db", ButtonVariant::Text)
                            .on_click(move |_, _, cx| cx.reveal_path(&db_path))
                            .child("在访达中显示")
                            .into_any_element(),
                    ),
                ),
                ui::setting_row(
                    "code",
                    "源代码与更新日志",
                    Some("github.com/mizorewww/gdirect".into()),
                    Some(
                        IconButton::new("source", Symbol::OPEN_IN_NEW, "在浏览器中打开")
                            .on_click(|_, _, cx| {
                                cx.open_url("https://github.com/mizorewww/gdirect")
                            })
                            .into_any_element(),
                    ),
                ),
            ]))
    }
}
