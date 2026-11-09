//! 设置: a category list beside the chosen category's settings.
use crate::{
    app::{ACCENTS, Dialog, Gdirect, Pane},
    format,
    i18n::{Language, t},
    keymap::{self, Binding, Section},
    settings::{self, Identity, Theme},
    tf, ui,
};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::path::PathBuf;

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

    /// How the settings page splits the content area at this window width.
    fn settings_layout(&self) -> Layout {
        let content = self.width - RAIL - MARGIN;
        let two_panes = content >= 1000.;
        let list_w = (content * 0.32).clamp(260., 320.);
        let pad = if content < 1200. { 24. } else { 32. };
        let pane = if two_panes { content - list_w } else { content };
        Layout {
            two_panes,
            list_w,
            pad,
            card_w: pane.min(MAX_W) - 2. * pad,
        }
    }

    /// Takes `&mut self` to keep a pane open once it has been shown; the
    /// caller's `self.render_settings(cx)` borrows mutably on its own.
    pub fn render_settings(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        // Help › Keyboard Shortcuts opens its pane.
        if keymap::take_open_shortcuts() {
            self.pane = Pane::Shortcuts;
            self.pane_open = true;
        }
        let layout = self.settings_layout();
        let two_panes = layout.two_panes;
        // A pane shown beside the list stays open when the window narrows to
        // one pane; the back arrow returns to the list.
        if two_panes {
            self.pane_open = true;
        }
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
        // Pane, supporting text, icon and tint of each category.
        type Category = (Pane, String, &'static str, Tint);
        let groups: [Vec<Category>; 3] = [
            vec![(Pane::Account, account_sub, "account_circle", Tint::Blue)],
            vec![
                (
                    Pane::Transfer,
                    {
                        // The count first, so a long folder is what gets cut.
                        let dir = format::home(&self.settings.download_dir);
                        let n = self.settings.connections;
                        if n == 1 {
                            tf!("{n} 个连接 · {dir}", "{n} connection · {dir}")
                        } else {
                            tf!("{n} 个连接 · {dir}", "{n} connections · {dir}")
                        }
                    },
                    "download",
                    Tint::Green,
                ),
                (
                    Pane::Network,
                    match self.settings.pinned {
                        Some(ip) => tf!(
                            "固定 {ip} · TLS 身份 · 证书",
                            "Pinned to {ip} · TLS identity · certificates"
                        ),
                        None => t(
                            "自动选择线路 · TLS 身份 · 证书",
                            "Auto route · TLS identity · certificates",
                        )
                        .into(),
                    },
                    "network_ping",
                    Tint::Teal,
                ),
                (
                    Pane::Mount,
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
                    t("主题、强调色、语言", "Theme, accent color, language").into(),
                    "palette",
                    Tint::Orange,
                ),
                (
                    Pane::Shortcuts,
                    t("查看和修改快捷键", "View and change shortcuts").into(),
                    "keyboard",
                    Tint::Pink,
                ),
                (
                    Pane::About,
                    format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                    "info",
                    Tint::Amber,
                ),
            ],
        ];
        // Wide content areas show the list beside the pane; narrow ones show
        // one at a time, with a way back to the list.
        let list = div()
            .id("settings-list")
            .when(two_panes, |d| d.w(px(layout.list_w)).flex_none())
            .when(!two_panes, |d| d.flex_1().min_w_0())
            .h_full()
            .track_scroll(&self.settings_scroll)
            .overflow_y_scroll();
        // Alone, the index's cards take the panes' margins, so a pane opens
        // on cards of the same width.
        let list = list.child(
            div()
                .w_full()
                .when(!two_panes, |d| d.max_w(rem(MAX_W)).mx_auto())
                .px(rem(if two_panes { 16. } else { layout.pad }))
                .py(rem(16.))
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
                    segmented_list(group.into_iter().map(|(pane, sub, icon, tint)| {
                        let title = pane_title(pane);
                        // Card segments on the surface, as the panes' groups are.
                        ListItem::new(title, title)
                            .container(ui::card())
                            .supporting(sub)
                            .leading(ListLeading::Tinted(ui::symbol(icon, true), tint, 40.))
                            .selected(two_panes && self.pane == pane)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.pane = pane;
                                this.pane_open = true;
                                this.reload_accounts();
                                // The item leaves the tree with the list: the
                                // page keeps the keys (Back, the menu bar).
                                this.focus_now(window, cx);
                                cx.notify();
                            }))
                    }))
                })),
        );
        let detail = match self.pane {
            Pane::Account => self.pane_account(cx).into_any_element(),
            Pane::Transfer => self.pane_transfer(cx).into_any_element(),
            Pane::Network => self.pane_network(cx).into_any_element(),
            Pane::Mount => self.pane_mount(layout.card_w, cx).into_any_element(),
            Pane::Appearance => self.pane_appearance(cx).into_any_element(),
            Pane::Shortcuts => self.pane_shortcuts(cx),
            Pane::About => self.pane_about(cx).into_any_element(),
        };
        let title = pane_title(self.pane);
        // Beside the list the pane has a headline; alone, one bar holds the way
        // back and the pane's name.
        let header = if two_panes {
            ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, title).into_any_element()
        } else {
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
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| {
                            this.pane_open = false;
                            this.focus_now(window, cx);
                            cx.notify();
                        },
                    )),
                )
                .child(ui::line(TypeRole::TitleLarge, ColorRole::ON_SURFACE, title))
                .into_any_element()
        };
        // Each pane scrolls on its own, so switching starts at the top; its
        // header (and the way back) stays above what scrolls, with a divider
        // once the pane scrolls beneath it.
        let shown = (two_panes || self.pane_open).then_some(self.pane);
        if self.pane_scrolled != shown {
            self.pane_scrolled = shown;
            self.pane_scroll.set_offset(Point::default());
        }
        let scrolled = self.pane_scroll.offset().y < px(0.);
        let header = div()
            .flex_none()
            .w_full()
            .border_b_1()
            .border_color(if scrolled {
                color(ColorRole::OUTLINE_VARIANT)
            } else {
                gpui_kit::transparent_black().into()
            })
            .child(
                div()
                    .w_full()
                    .max_w(rem(MAX_W))
                    .when(!two_panes, |d| d.mx_auto())
                    .px(rem(layout.pad))
                    .pt(rem(if two_panes { layout.pad } else { 12. }))
                    .pb(rem(if two_panes { 16. } else { 8. }))
                    .child(header),
            );
        let body = div()
            .id(("settings-detail", self.pane as usize))
            .flex_1()
            .min_h_0()
            .track_scroll(&self.pane_scroll)
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(rem(MAX_W))
                    .when(!two_panes, |d| d.mx_auto())
                    .px(rem(layout.pad))
                    .pt(rem(8.))
                    .pb(rem(layout.pad))
                    .child(detail),
            );
        let detail = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(color(ColorRole::SURFACE))
            .child(header)
            .child(body);
        div()
            .size_full()
            .flex()
            .when(two_panes || !self.pane_open, |d| d.child(list))
            .when(two_panes || self.pane_open, |d| d.child(detail))
    }

    /// Whether a pane is shown alone, which Back, Escape and the Settings
    /// rail item close to show the list.
    pub fn settings_pane_alone(&self) -> bool {
        self.page == crate::app::Page::Settings
            && self.pane_open
            && !self.settings_layout().two_panes
    }

    fn pane_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // With one account there is nothing to choose between.
        let single = self.accounts.len() == 1;
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
                    .min_h(rem(64.))
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
                            .child(ui::name_line(
                                TypeRole::BodyLarge,
                                ColorRole::ON_SURFACE,
                                email.clone(),
                            ))
                            .when(mounted, |d| {
                                d.child(ui::text(
                                    TypeRole::BodyMedium,
                                    ColorRole::ON_SURFACE_VARIANT,
                                    t("已挂载", "Mounted"),
                                ))
                            }),
                    )
                    .when(is_default && !single, |d| {
                        d.child(status_chip(StatusKind::Tonal).child(t("默认", "Default")))
                    })
                    .when(!is_default, |d| {
                        d.child(
                            material_button(("make-default", ix), ButtonVariant::Text)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.switch_account(switch.clone(), cx)
                                }))
                                .child(t("设为默认", "Make default")),
                        )
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
            .child(
                section(t("Google 账号", "Google accounts"))
                    .child(ui::group(rows))
                    .child(
                        div()
                            .pt(rem(4.))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(rem(8.))
                            .child(
                                MaterialButton::new("add-account")
                                    .variant(ButtonVariant::Tonal)
                                    .icon(Symbol::PERSON_ADD)
                                    // While a sign-in waits, it starts a fresh one.
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.add_account(window, cx)
                                    }))
                                    .accessibility_label(t("添加账号", "Add account"))
                                    .child(t("添加账号", "Add account")),
                            )
                            .when(self.signing_in && self.dialog.is_none(), |d| {
                                d.child(self.render_waiting(true, cx))
                            }),
                    ),
            )
            .child(section(t("OAuth 客户端", "OAuth client")).child(self.client_rows(cx)))
    }

    /// The saved OAuth client, its secret masked, and what can be done with it.
    fn client_rows(&self, cx: &mut Context<Self>) -> Div {
        let change = |id: &'static str, label: &'static str| {
            ui::flat_text_button(material_button(id, ButtonVariant::Text), self.signing_in)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.open_dialog(Dialog::Client { sign_in: false }, window, cx)
                }))
                .accessibility_label(label)
                .child(label)
        };
        let Some(client) = &self.oauth_client else {
            return ui::group(vec![ui::setting_row(
                "key",
                t("未设置", "Not set"),
                Some(
                    t(
                        "登录用的“桌面应用”OAuth 客户端",
                        "The “Desktop app” OAuth client to sign in with",
                    )
                    .into(),
                ),
                Some(change("client-set-up", t("设置…", "Set up…")).into_any_element()),
            )]);
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .gap(rem(4.))
            .pl(rem(ui::ROW_INDENT + 8.))
            .pr(rem(12.))
            .py(rem(8.))
            .child(
                material_button("client-copy", ButtonVariant::Text)
                    .icon(Symbol::CONTENT_COPY)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy_client(cx)))
                    .accessibility_label(t("复制为 JSON", "Copy as JSON"))
                    .child(t("复制为 JSON", "Copy as JSON")),
            )
            .child(
                material_button("client-export", ButtonVariant::Text)
                    .icon(Symbol::FILE_SAVE)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_client(cx)))
                    .accessibility_label(t("导出 JSON 文件…", "Export JSON file…"))
                    .child(t("导出 JSON 文件…", "Export JSON file…")),
            )
            .child(
                change("client-change", t("更换客户端…", "Change client…"))
                    .icon(Symbol::SWAP_HORIZ),
            );
        ui::group(vec![
            ui::setting_row_with(
                "key",
                crate::client_form::short_id(&client.client_id),
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .child(ui::latin(ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            tf!(
                                "客户端密钥 {}",
                                "Client secret {}",
                                crate::client_form::masked_secret(&client.client_secret)
                            ),
                        )))
                        .child(ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            t(
                                "应用处于测试阶段时，每个账号都要加为测试用户，登录 7 天后过期",
                                "While the app is in testing, add each account as a test user; sign-ins expire after 7 days",
                            ),
                        ))
                        .into_any_element(),
                ),
                None,
            ),
            actions.into_any_element(),
        ])
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
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(rem(2.))
                            .child(ui::text(
                                TypeRole::BodyLarge,
                                ColorRole::ON_SURFACE,
                                t("连接数", "Connections per download"),
                            ))
                            // The range in Roboto: the CJK face draws
                            // its en dash as a long dash.
                            .child(ui::latin(ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                t(
                                    "每个文件用多少条连接并行下载，1–16",
                                    "Parallel connections used for each file, 1–16",
                                ),
                            ))),
                    )
                    .child(ui::text(
                        TypeRole::HeadlineSmall,
                        ColorRole::PRIMARY,
                        s.connections.to_string(),
                    )),
            )
            .child(
                div().pl(rem(ui::ROW_INDENT)).child(
                    MaterialSlider::new(&self.connections, t("连接数", "Connections per download"))
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
            .child(
                section(t("下载", "Downloads")).child(ui::group(vec![
                    ui::setting_row_with(
                        "folder_open",
                        t("下载位置", "Download location"),
                        Some(ui::path("download-dir-path", format::home(&s.download_dir))),
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
                section(t("上传", "Uploads")).child(notice_banner(
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
        let identity = ConnectedButtonGroup::new("identity", t("TLS 身份", "TLS identity"))
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
            }));
        let certificate = match &s.ca_cert {
            Some(path) => div()
                .flex()
                .flex_col()
                .child(ui::supporting(t(
                    "下载使用自定义 CA",
                    "Downloads use a custom CA",
                )))
                .child(ui::path("ca-path", format::home(path)))
                .into_any_element(),
            None => ui::supporting(t(
                "内置 Google GTS 根证书，不使用系统 CA",
                "Built-in Google GTS root certificates; the system CA isn’t used",
            )),
        };
        // What bypassing does on this network right now.
        let network = gdirect::netif::status();
        let bypass_now = match (
            s.bypass_tun,
            network.bound(),
            &network.physical,
            &network.tun,
        ) {
            (true, Some(bound), _, Some(tun)) => Some(tf!(
                "当前：{}，绕过 {tun}",
                "Now: {}, bypassing {tun}",
                bound.name
            )),
            (true, Some(bound), _, None) => Some(tf!("当前：{}", "Now: {}", bound.name)),
            (true, None, Some(physical), _) => Some(tf!(
                "无法绑定到 {}，连接仍可能被代理接管",
                "Can’t bind to {}; your proxy may still take these connections",
                physical.name
            )),
            (false, _, _, Some(tun)) => Some(tf!("当前：经过 {tun}", "Now: through {tun}",)),
            _ => None,
        };
        // What it does, then on a quieter line of its own what it does now.
        let bypass_text = div()
            .flex()
            .flex_col()
            .gap(rem(2.))
            .child(ui::supporting(t(
                "TUN 模式的代理或 VPN 开着时，gdirect 连接 Google 仍走物理网卡；关闭后由代理规则决定。切换后会重新扫描线路，大约需要半分钟。",
                "With a TUN-mode proxy or VPN on, gdirect still reaches Google through your network adapter. Turn off to let your proxy rules decide. Switching rescans routes, which takes about half a minute.",
            )))
            .when_some(bypass_now, |d, now| {
                d.child(ui::text(TypeRole::BodySmall, ColorRole::ON_SURFACE_VARIANT, now))
            })
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(
                section(t("连接", "Connection")).child(ui::group(vec![
                    ui::setting_row(
                        if s.pinned.is_some() { "keep" } else { "route" },
                        t("线路选择", "Route"),
                        Some(match s.pinned {
                            Some(ip) => tf!(
                                "固定为 {ip}，所有请求只走这个地址",
                                "Pinned to {ip}; all requests go through it"
                            )
                            .into(),
                            None => t(
                                "自动：按延迟选择，失效时切换到下一个",
                                "Automatic: chosen by latency; switches to the next one if it fails",
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
                    ui::setting_row_stacked(
                        "badge",
                        t("TLS 身份", "TLS identity"),
                        Some(t(
                            "自动：先用真实 SNI，被重置时改用伪装身份并按 IP 记住。“不发送”只用于下载。",
                            "Automatic: tries the real SNI first, switches to a disguised identity when reset and remembers it per IP. “Don’t send” applies to downloads only.",
                        ).into()),
                        identity,
                    ),
                    ui::setting_row_with(
                        "alt_route",
                        t("绕过 TUN 代理直连 Google", "Connect to Google directly, bypassing TUN proxies"),
                        Some(bypass_text),
                        Some(
                            MaterialSwitch::new(
                                "bypass-tun",
                                t("绕过 TUN 代理直连 Google", "Connect to Google directly, bypassing TUN proxies"),
                            )
                            .checked(s.bypass_tun)
                            .on_change(cx.listener(|this, on: &bool, _, cx| {
                                this.settings.bypass_tun = *on;
                                let _ = gdirect::netif::save(&this.db, *on);
                                // Endpoints verified over the other path no
                                // longer count: connecting scans again.
                                let _ = gdirect::netif::sync(&this.db);
                                this.reload_endpoints();
                                this.connect(cx);
                            }))
                            .into_any_element(),
                        ),
                    ),
                ])),
            )
            .child(
                section(t("登录与证书", "Sign-in & certificates")).child(ui::group(vec![
                    ui::setting_row(
                        "vpn_lock",
                        t("登录时使用的代理", "Sign-in proxy"),
                        Some(t(
                            "跟随系统代理（HTTPS_PROXY 等）。只用于登录和账号验证；传输始终直连已扫描的地址。",
                            "Follows the system proxy (HTTPS_PROXY and similar). Used only for sign-in and account checks; transfers always connect directly to scanned addresses.",
                        ).into()),
                        None,
                    ),
                    ui::setting_row_with(
                        "verified_user",
                        t("证书", "Certificate"),
                        Some(certificate),
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
    }

    fn pane_mount(&self, card_w: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let mounted = self.mount.as_ref();
        // The rclone a mount would run, found the way the engine finds it.
        let rclone = gdirect::mount::rclone();
        let found = rclone.is_absolute();
        let status_bg = if mounted.is_some() {
            ColorRole::TERTIARY_CONTAINER
        } else {
            ui::card()
        };
        let status_fg = if mounted.is_some() {
            ColorRole::ON_TERTIARY_CONTAINER
        } else {
            ColorRole::ON_SURFACE_VARIANT
        };
        let (title, sub) = match (mounted, &self.account) {
            (Some(m), Some(account)) => (
                tf!("已挂载 · {account}", "Mounted · {account}"),
                ui::path_in("mount-card-path", status_fg, format::home(&m.path)),
            ),
            _ => (
                t("未挂载", "Not mounted").to_owned(),
                ui::text(
                    TypeRole::BodyMedium,
                    status_fg,
                    t(
                        "挂载后可以在访达中像本地磁盘一样使用云端硬盘",
                        "Once mounted, use Google Drive in Finder like a local disk",
                    ),
                )
                .into_any_element(),
            ),
        };
        let icon = ui::symbol("hard_drive", true)
            .size(rem(32.))
            .color(color(status_fg));
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(rem(2.))
            .child(ui::line(TypeRole::TitleLarge, status_fg, title))
            .child(sub);
        let buttons = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(rem(8.))
            .when_some(mounted.map(|m| m.path.clone()), |d, path| {
                d.child(
                    material_button("reveal-mount", ButtonVariant::Text)
                        .on_click(move |_, _, cx| cx.open_with_system(&path))
                        .child(t("在访达中打开", "Open in Finder")),
                )
            })
            .child(
                MaterialButton::new("mount")
                    .variant(if mounted.is_some() {
                        ButtonVariant::Outlined
                    } else {
                        ButtonVariant::Filled
                    })
                    // Mounting needs rclone; unmounting does not.
                    .disabled(self.account.is_none() || (mounted.is_none() && !found))
                    .loading(
                        self.mount_busy,
                        if mounted.is_some() {
                            t("正在卸载", "Unmounting")
                        } else {
                            t("正在挂载", "Mounting")
                        },
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_mount(cx)))
                    .child(if mounted.is_some() {
                        t("卸载", "Unmount")
                    } else {
                        t("挂载", "Mount")
                    }),
            );
        let card = div().rounded(rem(24.)).bg(color(status_bg)).p(rem(20.));
        // A narrow card gives the text the whole width and puts the buttons
        // under it.
        let card = if card_w < 520. {
            card.flex()
                .flex_col()
                .gap(rem(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rem(16.))
                        .child(icon)
                        .child(text),
                )
                .child(buttons.justify_end())
        } else {
            card.flex()
                .items_center()
                .gap(rem(16.))
                .child(icon)
                .child(text)
                .child(buttons)
        };
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(card)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rem(8.))
                    .child(ui::group(vec![
                        ui::setting_row_with(
                            "folder_open",
                            t("挂载位置", "Mount location"),
                            Some(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(ui::path("mount-path", format::home(&s.mount_path)))
                                    .when(mounted.is_some(), |d| {
                                        d.child(ui::supporting(t("卸载后才能更改", "Unmount to change")))
                                    })
                                    .into_any_element(),
                            ),
                            Some(if mounted.is_some() {
                                ui::text_button_disabled(t("更改", "Change")).into_any_element()
                            } else {
                                material_button("mount-dir", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_dir(true, cx)))
                                    .child(t("更改", "Change"))
                                    .into_any_element()
                            }),
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
                        if found {
                            ui::setting_row_with(
                                "check_circle",
                                "rclone",
                                Some(ui::path("rclone-path", format::home(&rclone))),
                                None,
                            )
                        } else {
                            self.rclone_missing(cx)
                        },
                    ]))
                    .child(ui::footnote(t(
                        "挂载通过 rclone 1.75.2+ 进行，经由扫描到的线路访问 Google。写入先进缓存，再在后台上传。macOS 使用内置 NFS，无需 FUSE。",
                        "Mounting uses rclone 1.75.2+ and reaches Google over the scanned routes. Writes go to the cache first, then upload in the background. macOS uses its built-in NFS, so no FUSE is needed.",
                    ))),
            )
    }

    /// The rclone row when there is none: how to install it, the command
    /// in monospace with a button that copies it.
    fn rclone_missing(&self, cx: &mut Context<Self>) -> AnyElement {
        use ui::Face::{Interface, Mono};
        // Commands and flags in monospace, as the command under them is.
        let (how, command): (Vec<(&str, ui::Face)>, &str) = if cfg!(target_os = "macos") {
            (
                vec![
                    (
                        t(
                            "未找到，挂载前需要安装。用 Homebrew 安装，或者用 gdirect 安装脚本的 ",
                            "Not found; mounting needs it. Install it with Homebrew, or use the gdirect installer with ",
                        ),
                        Interface,
                    ),
                    ("--with-mount", Mono),
                    (t("：", ":"), Interface),
                ],
                "brew install rclone",
            )
        } else if cfg!(target_os = "windows") {
            (
                vec![
                    (
                        t(
                            "未找到，挂载前需要安装。用 winget 安装，或者用 Scoop（",
                            "Not found; mounting needs it. Install it with winget, or with Scoop (",
                        ),
                        Interface,
                    ),
                    ("scoop install rclone", Mono),
                    (t("）：", "):"), Interface),
                ],
                "winget install Rclone.Rclone",
            )
        } else {
            (
                vec![
                    (
                        t(
                            "未找到，挂载前需要安装。发行版的软件包常常早于 1.75.2，可以用 rclone 官方脚本安装，或者用 gdirect 安装脚本的 ",
                            "Not found; mounting needs it. Distribution packages are often older than 1.75.2, so install it with rclone’s script, or use the gdirect installer with ",
                        ),
                        Interface,
                    ),
                    ("--with-mount", Mono),
                    (t("：", ":"), Interface),
                ],
                "sudo -v ; curl https://rclone.org/install.sh | sudo bash",
            )
        };
        let command_line = div()
            .mt(rem(6.))
            .flex()
            .items_center()
            .gap(rem(4.))
            .child(
                ui::mono(command)
                    .min_w_0()
                    .px(rem(10.))
                    .py(rem(6.))
                    .rounded(rem(8.))
                    .bg(color(ColorRole::SURFACE_CONTAINER_HIGHEST))
                    .text_color(color(ColorRole::ON_SURFACE)),
            )
            .child(
                IconButton::new(
                    "copy-rclone",
                    Symbol::CONTENT_COPY,
                    t("复制命令", "Copy command"),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(command.to_owned()));
                    this.say(t("已复制命令", "Copied the command"), cx);
                })),
            );
        ui::setting_row_with(
            "error",
            "rclone",
            Some(
                div()
                    .flex()
                    .flex_col()
                    .child(ui::rich(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        &how,
                    ))
                    .child(command_line)
                    .into_any_element(),
            ),
            None,
        )
    }

    fn pane_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let swatches =
            div()
                .flex()
                .flex_wrap()
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
        let theme = ConnectedButtonGroup::new("theme", t("主题", "Theme"))
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
            }));
        let language = ConnectedButtonGroup::new("language", t("语言", "Language"))
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
            }));
        // Every control here is too wide to share a line with its title.
        div()
            .flex()
            .flex_col()
            .gap(rem(24.))
            .child(
                section(t("主题与颜色", "Theme & color")).child(ui::group(vec![
                    ui::setting_row_stacked("dark_mode", t("主题", "Theme"), None, theme),
                    ui::setting_row_stacked("palette", t("强调色", "Accent color"), None, swatches),
                ])),
            )
            .child(
                section(t("语言", "Language")).child(ui::group(vec![ui::setting_row_stacked(
                    "translate",
                    t("界面语言", "App language"),
                    None,
                    language,
                )])),
            )
    }

    /// The Keyboard shortcuts pane: a view of its own, kept for the app's
    /// life, as it records keystrokes with its own focus.
    fn pane_shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = match cx.try_global::<ShortcutsPane>() {
            Some(pane) => pane.0.clone(),
            None => {
                let db_path = self.db_path.clone();
                let view = cx.new(|cx| ShortcutsView::new(db_path, cx));
                cx.set_global(ShortcutsPane(view.clone()));
                view
            }
        };
        view.into_any_element()
    }

    fn pane_about(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let db_path = self.db_path.clone();
        ui::group(vec![
            ui::setting_row(
                "system_update",
                format!("gdirect {}", env!("CARGO_PKG_VERSION")),
                Some(
                    t(
                        "Google 云端硬盘桌面客户端",
                        "Desktop client for Google Drive",
                    )
                    .into(),
                ),
                Some(
                    // Opens the page; the app does not check by itself.
                    material_button("releases", ButtonVariant::Text)
                        .on_click(|_, _, cx| {
                            cx.open_url("https://github.com/mizorewww/gdirect/releases")
                        })
                        .child(t("发布页", "Releases"))
                        .into_any_element(),
                ),
            ),
            ui::setting_row_with(
                "database",
                t("数据文件", "Data file"),
                Some(ui::path("db-path", format::home(&self.db_path))),
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
                    material_button("source", ButtonVariant::Text)
                        .on_click(|_, _, cx| cx.open_url("https://github.com/mizorewww/gdirect"))
                        .child(t("打开", "Open"))
                        .into_any_element(),
                ),
            ),
        ])
    }
}

/// The navigation rail's width and the margin right of the content area.
const RAIL: f32 = 88.;
const MARGIN: f32 = 8.;
/// The widest the pane, and the list when alone, grows.
const MAX_W: f32 = 720.;

/// How the settings page splits the content area, in logical px.
#[derive(Clone, Copy)]
struct Layout {
    two_panes: bool,
    list_w: f32,
    /// The pane's padding.
    pad: f32,
    /// The width of the pane's cards.
    card_w: f32,
}

fn pane_title(pane: Pane) -> &'static str {
    match pane {
        Pane::Account => t("账号", "Account"),
        Pane::Transfer => t("下载与上传", "Downloads & uploads"),
        Pane::Network => t("网络", "Network"),
        Pane::Mount => t("挂载", "Mount"),
        Pane::Appearance => t("外观", "Appearance"),
        Pane::Shortcuts => t("键盘快捷键", "Keyboard shortcuts"),
        Pane::About => t("关于", "About"),
    }
}

/// A labelled section of a pane; its body follows as children.
fn section(label: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rem(8.))
        .child(ui::section_label(label))
}

/// The Keyboard shortcuts pane's view, made the first time it shows.
struct ShortcutsPane(Entity<ShortcutsView>);

/// A shortcut is being recorded, which takes Escape before anything else.
static RECORDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the Keyboard shortcuts pane is recording a keystroke.
pub fn recording_shortcut() -> bool {
    RECORDING.load(std::sync::atomic::Ordering::Relaxed)
}

impl Global for ShortcutsPane {}

/// Lists the shortcuts and rebinds them: a click on a binding records the
/// next keystroke as its new keys.
pub struct ShortcutsView {
    db_path: PathBuf,
    /// Held while recording, so the keystroke comes here.
    focus: FocusHandle,
    /// The binding being recorded.
    recording: Option<String>,
    /// The last keystroke recorded could not be a shortcut.
    rejected: bool,
    /// A recorded keystroke that other bindings already have.
    conflict: Option<Conflict>,
    _intercept: Subscription,
}

struct Conflict {
    id: String,
    keys: String,
    /// The ids of the bindings that have the keys.
    others: Vec<String>,
}

impl ShortcutsView {
    fn new(db_path: PathBuf, cx: &mut Context<Self>) -> Self {
        // Key bindings are matched before any element's key-down listener,
        // so a keystroke being recorded is taken ahead of them, before it can
        // run the command it is bound to now.
        let view = cx.weak_entity();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            let taken = view
                .update(cx, |view, cx| view.record(&event.keystroke, window, cx))
                .unwrap_or(false);
            if taken {
                cx.stop_propagation();
            }
        });
        Self {
            db_path,
            focus: cx.focus_handle(),
            recording: None,
            rejected: false,
            conflict: None,
            _intercept: intercept,
        }
    }

    fn start(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        RECORDING.store(true, std::sync::atomic::Ordering::Relaxed);
        self.recording = Some(id);
        self.rejected = false;
        self.conflict = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        RECORDING.store(false, std::sync::atomic::Ordering::Relaxed);
        self.recording = None;
        self.rejected = false;
        cx.notify();
    }

    /// Takes a keystroke while recording; true when it was taken.
    fn record(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(id) = self.recording.clone() else {
            return false;
        };
        if !self.focus.is_focused(window) {
            self.stop(cx);
            return false;
        }
        if keystroke.key == "escape" && !keystroke.modifiers.modified() {
            self.stop(cx);
            return true;
        }
        // Modifiers alone wait for the key they go with.
        if keystroke.is_modifier_key() {
            return true;
        }
        let os = keymap::host_os(cx);
        let list = keymap::bindings(&keymap::overrides(cx), os);
        let Some(ix) = list.iter().position(|b| b.id == id) else {
            self.stop(cx);
            return true;
        };
        if !keymap::accepts(keystroke, list[ix].context) {
            self.rejected = true;
            cx.notify();
            return true;
        }
        let Some(keys) = keymap::normalize(&keystroke.unparse(os), os) else {
            return true;
        };
        self.stop(cx);
        let others: Vec<String> = keymap::conflicts(&list, ix, &keys)
            .into_iter()
            .map(|j| list[j].id.clone())
            .collect();
        if others.is_empty() {
            self.commit(
                |overrides, list| keymap::set(overrides, list, &id, Some(keys)),
                cx,
            );
        } else {
            self.conflict = Some(Conflict { id, keys, others });
        }
        true
    }

    /// Changes the overrides, stores them and puts them in force.
    fn commit(
        &mut self,
        change: impl FnOnce(&mut settings::KeymapOverrides, &[Binding]),
        cx: &mut Context<Self>,
    ) {
        let mut overrides = keymap::overrides(cx);
        let list = keymap::bindings(&overrides, keymap::host_os(cx));
        change(&mut overrides, &list);
        if let Ok(db) = gdirect::store::Store::open(&self.db_path) {
            settings::save_keymap(&db, &overrides);
        }
        keymap::apply(overrides, cx);
        cx.notify();
    }

    /// Gives the conflicting keys to the binding recorded, unsetting the others.
    fn replace(&mut self, cx: &mut Context<Self>) {
        let Some(Conflict { id, keys, others }) = self.conflict.take() else {
            return;
        };
        self.commit(
            |overrides, list| {
                for other in &others {
                    keymap::set(overrides, list, other, None);
                }
                keymap::set(overrides, list, &id, Some(keys));
            },
            cx,
        );
    }

    /// One row: an action, its bindings as key caps, and the ways to change them.
    fn row(
        &self,
        ix: usize,
        bindings: &[&Binding],
        all: &[Binding],
        os: OperatingSystem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = bindings[0].label();
        let changed = bindings.iter().any(|b| b.changed());
        let set: Vec<&&Binding> = bindings.iter().filter(|b| b.keys.is_some()).collect();
        // The edit button records the first unset binding, else the first.
        let target = bindings
            .iter()
            .find(|b| b.keys.is_none())
            .unwrap_or(&bindings[0])
            .id
            .clone();
        let recording = bindings
            .iter()
            .any(|b| self.recording.as_deref() == Some(b.id.as_str()));
        let mut keys = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(rem(6.));
        if recording {
            keys = keys.child(
                ui::pill(
                    t("按下新的快捷键…", "Press the new shortcut…"),
                    ColorRole::PRIMARY_CONTAINER,
                    ColorRole::ON_PRIMARY_CONTAINER,
                )
                .py(rem(4.)),
            );
        } else if set.is_empty() {
            let id = target.clone();
            keys = keys.child(
                div()
                    .id(("unset", ix))
                    .cursor_pointer()
                    .rounded(rem(8.))
                    .px(rem(6.))
                    .py(rem(4.))
                    .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.start(id.clone(), window, cx)
                    }))
                    .child(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        t("未设置", "Not set"),
                    )),
            );
        } else {
            for (n, b) in set.iter().enumerate() {
                let id = b.id.clone();
                let tip: SharedString = tf!(
                    "点按以更改 {}",
                    "Click to change {}",
                    keymap::display(b.keys.as_deref().unwrap_or_default(), os)
                )
                .into();
                let chip = div()
                    .id(("keys", ix * 16 + n))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(rem(3.))
                    .rounded(rem(8.))
                    .px(rem(4.))
                    .py(rem(3.))
                    .hover(|s| s.bg(color(ColorRole::ON_SURFACE).opacity(0.08)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.start(id.clone(), window, cx)
                    }))
                    .child(key_caps(b.keys.as_deref().unwrap_or_default(), os));
                if n > 0 {
                    keys = keys.child(ui::text(
                        TypeRole::BodySmall,
                        ColorRole::ON_SURFACE_VARIANT,
                        t("或", "or"),
                    ));
                }
                keys = keys
                    .child(MaterialTooltip::plain(("keys-tip", ix * 16 + n), tip).trigger(chip));
            }
        }
        let reset_ids: Vec<String> = bindings.iter().map(|b| b.id.clone()).collect();
        let line = div()
            .flex()
            .items_center()
            .gap(rem(12.))
            .min_h(rem(56.))
            .pl(rem(20.))
            .pr(rem(8.))
            .py(rem(6.))
            .child(
                ui::text(TypeRole::BodyLarge, ColorRole::ON_SURFACE, label)
                    .flex_1()
                    .min_w(rem(120.)),
            )
            .child(keys)
            .when(changed && !recording, |d| {
                d.child(
                    material_button(("reset", ix), ButtonVariant::Text)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.conflict = None;
                            let ids = reset_ids.clone();
                            this.commit(
                                |overrides, _| {
                                    for id in &ids {
                                        overrides.remove(id);
                                    }
                                },
                                cx,
                            );
                        }))
                        .child(t("重置", "Reset")),
                )
            })
            .child(if recording {
                IconButton::new(("cancel-record", ix), Symbol::CLOSE, t("取消", "Cancel"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop(cx)))
                    .into_any_element()
            } else {
                IconButton::new(
                    ("edit", ix),
                    ui::symbol("edit", false),
                    tf!("更改“{}”的快捷键", "Change the shortcut for “{}”", label),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.start(target.clone(), window, cx)
                }))
                .into_any_element()
            });
        let rejected = recording && self.rejected;
        let conflict = self
            .conflict
            .as_ref()
            .filter(|c| bindings.iter().any(|b| b.id == c.id));
        div()
            .flex()
            .flex_col()
            .child(line)
            .when(rejected, |d| {
                d.child(
                    ui::text(
                        TypeRole::BodySmall,
                        ColorRole::ERROR,
                        if os == OperatingSystem::Mac {
                            t(
                                "快捷键要包含 ⌘、⌃ 或 ⌥，或者是功能键。按 Esc 取消。",
                                "A shortcut needs ⌘, ⌃ or ⌥, or a function key. Press Esc to cancel.",
                            )
                        } else {
                            t(
                                "快捷键要包含 Ctrl 或 Alt，或者是功能键。按 Esc 取消。",
                                "A shortcut needs Ctrl or Alt, or a function key. Press Esc to cancel.",
                            )
                        },
                    )
                    .px(rem(20.))
                    .pb(rem(10.)),
                )
            })
            .when_some(conflict, |d, c| {
                let names: Vec<&str> = c
                    .others
                    .iter()
                    .filter_map(|id| all.iter().find(|b| &b.id == id))
                    .map(|b| b.label())
                    .collect();
                let names = names.join(t("”、“", "”, “"));
                let keys = keymap::display(&c.keys, os);
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(rem(8.))
                        .mx(rem(12.))
                        .mb(rem(12.))
                        .pl(rem(12.))
                        .pr(rem(4.))
                        .py(rem(4.))
                        .rounded(rem(12.))
                        .bg(color(ColorRole::ERROR_CONTAINER))
                        .child(ui::icon("warning", 20., ColorRole::ON_ERROR_CONTAINER))
                        .child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_ERROR_CONTAINER,
                                tf!("{keys} 已被“{names}”使用", "{keys} is already used by “{names}”"),
                            )
                            .flex_1()
                            .min_w(rem(160.)),
                        )
                        .child(
                            material_button(("conflict-cancel", ix), ButtonVariant::Text)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.conflict = None;
                                    cx.notify();
                                }))
                                .accessibility_label(t("取消", "Cancel"))
                                .child(t("取消", "Cancel")),
                        )
                        .child(
                            material_button(("conflict-replace", ix), ButtonVariant::Tonal)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.replace(cx)))
                                .child(t("替换", "Replace")),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for ShortcutsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Focus that went elsewhere ends the recording.
        if self.recording.is_some() && !self.focus.is_focused(window) {
            self.recording = None;
            self.rejected = false;
        }
        let os = keymap::host_os(cx);
        let overrides = keymap::overrides(cx);
        let all = keymap::bindings(&overrides, os);
        let mut row_ix = 0;
        let sections: Vec<AnyElement> = Section::ALL
            .into_iter()
            .filter_map(|kind| {
                // An action's bindings share a row, in the table's order.
                let mut rows: Vec<Vec<&Binding>> = Vec::new();
                for b in all.iter().filter(|b| b.section == kind) {
                    match rows.iter_mut().find(|r| {
                        r[0].action.partial_eq(b.action.as_ref()) && r[0].context == b.context
                    }) {
                        Some(row) => row.push(b),
                        None => rows.push(vec![b]),
                    }
                }
                if rows.is_empty() {
                    return None;
                }
                let rows: Vec<AnyElement> = rows
                    .iter()
                    .map(|row| {
                        row_ix += 1;
                        self.row(row_ix, row, &all, os, cx)
                    })
                    .collect();
                Some(
                    section(kind.label())
                        .child(ui::group(rows))
                        .into_any_element(),
                )
            })
            .collect();
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(rem(24.))
            .children(sections)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(rem(12.))
                    .child(
                        ui::footnote(t(
                            "文本框中的编辑快捷键（复制、粘贴、撤销等）不受这里的设置影响。",
                            "Editing shortcuts inside text fields (copy, paste, undo…) aren’t affected by these settings.",
                        ))
                        .flex_1()
                        .min_w(rem(200.)),
                    )
                    .child(
                        MaterialButton::new("restore-keys")
                            .variant(ButtonVariant::Outlined)
                            .icon(Symbol::RESTART_ALT)
                            .disabled(overrides.is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.conflict = None;
                                this.recording = None;
                                this.commit(|overrides, _| overrides.clear(), cx);
                            }))
                            .child(t("全部恢复默认", "Restore all defaults")),
                    ),
            )
    }
}

/// A keystroke as key caps: small rounded mono chips, joined by + off a Mac.
fn key_caps(keys: &str, os: OperatingSystem) -> Div {
    let mac = os == OperatingSystem::Mac;
    let caps = keymap::caps(keys, os);
    let count = caps.len();
    let mut row = div().flex().items_center().gap(rem(3.));
    for (i, cap) in caps.into_iter().enumerate() {
        row = row.child(
            ui::mono(cap)
                .flex_none()
                .min_w(rem(24.))
                .h(rem(24.))
                .px(rem(6.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(rem(6.))
                // A step above the card it sits on: in dark, the brightest surface.
                .bg(color(if gpui_kit_material::theme::is_dark() {
                    ColorRole::SURFACE_BRIGHT
                } else {
                    ColorRole::SURFACE_CONTAINER_HIGHEST
                }))
                .border_1()
                .border_color(color(ColorRole::OUTLINE_VARIANT))
                .text_color(color(ColorRole::ON_SURFACE)),
        );
        if !mac && i + 1 < count {
            row = row.child(ui::text(
                TypeRole::LabelMedium,
                ColorRole::ON_SURFACE_VARIANT,
                "+",
            ));
        }
    }
    row
}
