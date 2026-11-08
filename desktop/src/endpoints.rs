//! 线路: the Google front ends reachable from this network, and scanning for them.
use crate::{
    app::{Gdirect, ScanState},
    i18n::t,
    tf, ui,
};
use gdirect::{scan::Event, store::Store};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const COLUMNS: [f32; 4] = [280., 180., 160., 56.];

impl Gdirect {
    /// Sweeps Google's front ends, or verifies the addresses in `file`.
    pub fn start_scan(&mut self, file: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan = Some((cancel.clone(), ScanState::default()));
        self.scan_error = None;
        self.intercepted = false;
        let db_path = self.db_path.clone();
        let tx = self.sender();
        self.background(
            move || -> anyhow::Result<()> {
                let db = Store::open(&db_path)?;
                gdirect::scan::run_with(&db, file.as_deref(), &cancel, &mut |event| {
                    let _ = tx.send(Box::new(move |this: &mut Gdirect, _| {
                        this.scan_event(event)
                    }));
                })
            },
            |this, result, cx| {
                let cancelled = this
                    .scan
                    .take()
                    .is_some_and(|(cancel, _)| cancel.load(Ordering::Relaxed));
                this.reload_endpoints();
                let finished = result.is_ok() && !cancelled;
                match result {
                    Ok(()) => this.say(
                        if cancelled {
                            t("已停止扫描", "Scan stopped")
                        } else {
                            t("扫描完成，可直达的地址已保存", "Scan complete · reachable addresses saved")
                        },
                        cx,
                    ),
                    Err(e) if !cancelled => this.scan_error = Some(format!("{e:#}")),
                    Err(_) => {}
                }
                // A first scan unblocks the connection that was waiting for one;
                // a later one moves an unpinned connection to the fastest front end.
                let reroute = finished && this.settings.pinned.is_none();
                if (this.api.is_none() || reroute) && this.account.is_some() && !this.connecting {
                    if this.settings.pinned.is_some() || this.has_route() {
                        this.connect(cx);
                    } else if this.api.is_none() && this.scan_error.is_none() {
                        this.connect_error = Some(
                            t(
                                "没有找到可直达的 Google 前端，请在“线路”页重新扫描或导入候选 IP。",
                                "No directly reachable Google front end found. Rescan or import candidate IPs on the Routes page.",
                            )
                            .into(),
                        );
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn scan_event(&mut self, event: Event) {
        let Some((_, state)) = self.scan.as_mut() else {
            return;
        };
        match event {
            Event::Candidates(n) => state.candidates = Some(n),
            Event::Sweeping { ipv6, target } => {
                state.ipv6 = ipv6;
                state.target = target;
            }
            Event::Swept { probed, open } => {
                state.probed = probed;
                state.open = open;
            }
            Event::Progress { probed, open, good } => {
                state.probed = probed;
                state.open = open;
                state.good = good;
            }
            Event::Intercepted { .. } => self.intercepted = true,
            Event::Verified { .. } => {
                state.good += 1;
                self.reload_endpoints();
            }
            Event::Unavailable(_) => self.reload_endpoints(),
            // The Settings page shows how connections leave this machine.
            Event::Skipped(_) | Event::ProbeWarning(_) | Event::Network(_) => {}
        }
    }

    fn stop_scan(&mut self, cx: &mut Context<Self>) {
        if let Some((cancel, _)) = &self.scan {
            cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    fn import_candidates(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                t(
                    "导入候选 IP（每行一个）",
                    "Import candidate IPs (one per line)",
                )
                .into(),
            ),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update(cx, |this, cx| this.start_scan(Some(path), cx));
            }
        })
        .detach();
    }

    fn pin(&mut self, ip: Option<IpAddr>, cx: &mut Context<Self>) {
        self.settings.pinned = ip;
        self.settings.save(&self.db);
        self.say(
            match ip {
                Some(ip) => tf!("所有请求只走 {ip}", "All requests now go through {ip}"),
                None => t(
                    "已取消固定，按延迟自动选择",
                    "Unpinned · choosing by latency",
                )
                .into(),
            },
            cx,
        );
        self.connect(cx);
    }

    pub fn render_endpoints(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scanning = self.scan.is_some();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(rem(12.))
                    .pt(rem(24.))
                    .px(rem(24.))
                    .pb(rem(16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(rem(240.))
                            .flex()
                            .flex_col()
                            .gap(rem(4.))
                            .child(ui::text(TypeRole::HeadlineSmall, ColorRole::ON_SURFACE, t("线路", "Routes")))
                            .child(ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                t(
                                    "从本机网络可直达的 Google 前端。证书始终用内置的 Google GTS 根证书校验。",
                                    "Google front ends reachable directly from this network. Certificates are always verified against the built-in Google GTS root.",
                                ),
                            )),
                    )
                    // The actions wrap together.
                    .child(div().flex().items_center().gap(rem(12.)).child(
                        MaterialButton::new("import")
                            .variant(ButtonVariant::Outlined)
                            .icon(Symbol::PLAYLIST_ADD)
                            .disabled(scanning)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.import_candidates(cx)))
                            .child(t("导入候选 IP", "Import candidate IPs")),
                    )
                    .child(if scanning {
                        let stopping = self
                            .scan
                            .as_ref()
                            .is_some_and(|(cancel, _)| cancel.load(Ordering::Relaxed));
                        MaterialButton::new("scan")
                            .icon(Symbol::STOP)
                            .disabled(stopping)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.stop_scan(cx)))
                            .child(if stopping { t("正在停止…", "Stopping…") } else { t("停止扫描", "Stop scan") })
                    } else {
                        MaterialButton::new("scan")
                            .icon(Symbol::RADAR)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.start_scan(None, cx)))
                            .child(t("重新扫描", "Rescan"))
                    })),
            )
            .child(
                div()
                    .id("endpoints-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                    .px(rem(24.))
                    .pb(rem(24.))
                    .flex()
                    .flex_col()
                    .gap(rem(16.))
                    .when(self.intercepted, |d| {
                        d.child(
                            notice_banner(
                                "tun",
                                NoticeKind::Warning,
                                if self.settings.bypass_tun {
                                    t(
                                        "几乎所有地址都响应了 TCP。连接已经绕过 TUN 代理走物理网卡，所以拦截可能来自网卡之下的 VPN、防火墙或路由器，结果反映的是它们而不是你的直连网络。",
                                        "Almost every address answered TCP. Connections already bypass TUN proxies through your network adapter, so a VPN, firewall or router below it is probably intercepting, and the results reflect it, not your direct network.",
                                    )
                                } else {
                                    t(
                                        "几乎所有地址都响应了 TCP，可能有 TUN 代理或 VPN 在拦截，结果反映的是代理而不是你的直连网络。在“设置 › 网络”中打开“绕过 TUN 代理直连 Google”，然后重新扫描。",
                                        "Almost every address answered TCP, so a TUN proxy or VPN is probably intercepting, and the results reflect the proxy, not your direct network. Turn on “Connect to Google directly, bypassing TUN proxies” in Settings › Network, then rescan.",
                                    )
                                },
                            )
                            .child(
                                material_button("tun-ok", ButtonVariant::Text)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.intercepted = false;
                                        cx.notify();
                                    }))
                                    .child(t("知道了", "Got it")),
                            ),
                        )
                    })
                    .when_some(self.scan_error.clone(), |d, error| {
                        d.child(notice_banner("scan-error", NoticeKind::Danger, error))
                    })
                    .child(match &self.scan {
                        Some((_, state)) => self.render_scan_progress(state).into_any_element(),
                        None => self.render_current().into_any_element(),
                    })
                    .child(self.render_filters(cx))
                    .child(self.render_table(cx))
                    .child(ui::text(
                        TypeRole::BodySmall,
                        ColorRole::ON_SURFACE_VARIANT,
                        t(
                            "固定一个地址相当于 --ip，所有请求只走它；取消固定后按延迟自动选择，并在失效时切换到下一个。",
                            "Pinning an address works like --ip: every request goes through it. Unpinned, gdirect chooses by latency and switches to the next address if one fails.",
                        ),
                    )),
                    ),
            )
    }

    fn render_current(&self) -> impl IntoElement {
        let usable = self
            .endpoints
            .iter()
            .filter(|e| e.latency_ms.is_some())
            .count();
        let (ip, ms, sni) = self
            .current_route()
            .unwrap_or_else(|| ("—".into(), None, String::new()));
        let mode = if self.settings.pinned.is_some() {
            t("已固定此地址", "Pinned address")
        } else if self.api.is_some() {
            t(
                "当前线路 · 按延迟自动选择",
                "Current route · chosen by latency",
            )
        } else {
            t("未连接", "Not connected")
        };
        let stat = |label: &'static str, value: String, unit: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap(rem(2.))
                .child(ui::text(
                    TypeRole::LabelMedium,
                    ColorRole::ON_SURFACE_VARIANT,
                    label,
                ))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(rem(4.))
                        .child(ui::text(
                            TypeRole::HeadlineMedium,
                            ColorRole::ON_SURFACE,
                            value,
                        ))
                        .child(ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            unit,
                        )),
                )
        };
        div()
            .rounded(rem(24.))
            .bg(color(ColorRole::SURFACE_CONTAINER_LOW))
            .p(rem(24.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rem(24.))
            .child(
                div()
                    .flex_1()
                    .min_w(rem(260.))
                    .flex()
                    .flex_col()
                    .gap(rem(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rem(8.))
                            // Green, amber or red by latency, as a route's health reads.
                            .child(div().size(rem(8.)).rounded_full().bg(color(
                                if self.api.is_some() {
                                    crate::format::latency_role(ms)
                                } else {
                                    ColorRole::OUTLINE
                                },
                            )))
                            .child(ui::text(
                                TypeRole::LabelLarge,
                                ColorRole::ON_SURFACE_VARIANT,
                                mode,
                            )),
                    )
                    .child(
                        div()
                            .font_family("Roboto Mono")
                            .text_size(rem(28.))
                            .child(ip),
                    )
                    .child(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE_VARIANT,
                        if sni.is_empty() {
                            t("真实 SNI", "Real SNI").to_owned()
                        } else {
                            tf!(
                                "伪装 TLS 身份 {sni}（Host 保持真实）",
                                "Disguised TLS identity {sni} (real Host kept)"
                            )
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(rem(32.))
                    .child(stat(
                        t("延迟", "Latency"),
                        ms.map_or("—".into(), |ms| ms.to_string()),
                        "ms",
                    ))
                    .child(stat(
                        t("可直达", "Reachable"),
                        usable.to_string(),
                        if usable == 1 {
                            t("个地址", "address")
                        } else {
                            t("个地址", "addresses")
                        },
                    )),
            )
    }

    fn render_scan_progress(&self, state: &ScanState) -> impl IntoElement {
        let target = state.target.max(1);
        let (title, detail) = match state.candidates {
            Some(n) => (
                {
                    let s = if n == 1 { "" } else { "es" };
                    tf!(
                        "正在验证导入的 {n} 个候选地址",
                        "Verifying {n} imported candidate address{s}"
                    )
                },
                t(
                    "TLS 握手 → Drive API → 测试下载",
                    "TLS handshake → Drive API → test download",
                )
                .to_owned(),
            ),
            None => (
                tf!(
                    "正在扫描 Google 前端 · 已验证 {} / {target}",
                    "Scanning Google front ends · {} / {target} verified",
                    state.good.min(target)
                ),
                {
                    let probed = state.probed;
                    let es = if probed == 1 { "" } else { "es" };
                    tf!(
                        "已探测 {probed} 个地址，{} 个响应 TCP · {}",
                        "Probed {probed} address{es}, {} answered TCP · {}",
                        state.open,
                        if state.ipv6 {
                            "IPv4 + IPv6"
                        } else {
                            t(
                                "仅 IPv4（本机没有 IPv6 路由）",
                                "IPv4 only (no IPv6 route on this device)",
                            )
                        }
                    )
                },
            ),
        };
        div()
            .rounded(rem(24.))
            .bg(color(ColorRole::PRIMARY_CONTAINER))
            .p(rem(24.))
            .flex()
            .flex_col()
            .gap(rem(12.))
            .child(ui::text(
                TypeRole::TitleLarge,
                ColorRole::ON_PRIMARY_CONTAINER,
                title,
            ))
            .child(ui::text(
                TypeRole::BodyMedium,
                ColorRole::ON_PRIMARY_CONTAINER,
                detail,
            ))
            .child(
                LinearProgress::new("scan-progress", t("扫描进度", "Scan progress")).value(
                    match state.candidates {
                        Some(_) => None,
                        None if state.good == 0 => None,
                        None => Some(state.good.min(target) as f32 / target as f32),
                    },
                ),
            )
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div().flex().items_center().child(
            ConnectedButtonGroup::new("family", t("地址类型", "Address type"))
                .options([("all", t("全部", "All")), ("v4", "IPv4"), ("v6", "IPv6")])
                .selected(self.family.clone())
                .on_change(cx.listener(|this, value: &SharedString, _, cx| {
                    this.family = value.clone();
                    cx.notify();
                })),
        )
    }

    fn render_table(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current_route().map(|(ip, ..)| ip);
        let mut rows: Vec<_> = self
            .endpoints
            .iter()
            .filter(|e| match self.family.as_ref() {
                "v4" => !e.ip.contains(':'),
                "v6" => e.ip.contains(':'),
                _ => true,
            })
            .collect();
        rows.sort_by_key(|e| (e.latency_ms.is_none(), e.latency_ms));
        if rows.is_empty() {
            let family = match self.family.as_ref() {
                "v4" => Some("IPv4"),
                "v6" => Some("IPv6"),
                _ => None,
            };
            return match family {
                Some(family) if !self.endpoints.is_empty() => ui::empty(
                    "filter_alt_off",
                    tf!("没有 {family} 地址", "No {family} addresses"),
                    t(
                        "扫描只会在本机有对应路由时探测这类地址。",
                        "Scans probe these addresses only when this device has a route for them.",
                    ),
                ),
                _ => ui::empty(
                    "radar",
                    t("还没有扫描到地址", "No addresses scanned yet"),
                    t(
                        "点“重新扫描”寻找从本机网络可直达的 Google 前端。",
                        "Choose Rescan to find Google front ends reachable directly from this network.",
                    ),
                ),
            }
            .into_any_element();
        }
        // Narrow windows drop the TLS column, then the latency bars.
        let show_tls = self.wide(1000.);
        let show_bar = self.wide(860.);
        let (address_w, latency_w) = if show_bar {
            (COLUMNS[0], COLUMNS[1])
        } else {
            (180., 88.)
        };
        let header = ui::type_role(div(), TypeRole::TitleSmall)
            .flex()
            .items_center()
            .h(rem(48.))
            .px(rem(16.))
            .text_color(color(ColorRole::ON_SURFACE_VARIANT))
            .child(
                div()
                    .flex_1()
                    .min_w(rem(address_w))
                    .child(t("地址", "Address")),
            )
            .child(div().w(rem(latency_w)).child(t("延迟", "Latency")))
            .when(show_tls, |d| {
                d.child(
                    div()
                        .w(rem(COLUMNS[2]))
                        .child(t("TLS 身份", "TLS identity")),
                )
            })
            .child(div().flex_1().min_w(rem(120.)).child(t("状态", "Status")))
            .child(div().w(rem(COLUMNS[3])));
        // Bars compare the listed front ends, on a scale of at least 200 ms.
        let slowest = rows
            .iter()
            .filter_map(|e| e.latency_ms)
            .max()
            .unwrap_or(0)
            .max(200) as f32;
        let body: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, e)| {
                let ok = e.latency_ms.is_some();
                let in_use = current.as_deref() == Some(e.ip.as_str());
                let ip: Option<IpAddr> = e.ip.parse().ok();
                let pinned = ip.is_some() && self.settings.pinned == ip;
                let ink = if ok {
                    ColorRole::ON_SURFACE
                } else {
                    ColorRole::ON_SURFACE_VARIANT
                };
                let bar = e
                    .latency_ms
                    .filter(|_| show_bar)
                    .map(|ms| ((ms as f32 / slowest).clamp(0.03, 1.), ms < 300));
                div()
                    .flex()
                    .items_center()
                    .min_h(rem(56.))
                    .px(rem(16.))
                    .when(in_use, |d| {
                        d.bg(color(ColorRole::SECONDARY_CONTAINER).opacity(0.5))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(rem(address_w))
                            .flex()
                            .items_center()
                            .gap(rem(8.))
                            .child(
                                ui::mono(e.ip.clone())
                                    .text_color(color(ink))
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis(),
                            )
                            .child(ui::text(
                                TypeRole::LabelSmall,
                                ColorRole::ON_SURFACE_VARIANT,
                                if e.ip.contains(':') { "IPv6" } else { "IPv4" },
                            )),
                    )
                    .child(
                        div()
                            .w(rem(latency_w))
                            .flex()
                            .items_center()
                            .gap(rem(12.))
                            .child(
                                ui::text(
                                    TypeRole::BodyMedium,
                                    ink,
                                    e.latency_ms.map_or("—".into(), |ms| format!("{ms} ms")),
                                )
                                .w(rem(56.)),
                            )
                            .when_some(bar, |d, (fraction, fast)| {
                                d.child(
                                    div()
                                        .w(rem(80.))
                                        .h(rem(4.))
                                        .rounded_full()
                                        .bg(color(ColorRole::SURFACE_CONTAINER_HIGHEST))
                                        .child(
                                            div().h_full().w(relative(fraction)).rounded_full().bg(
                                                color(if fast {
                                                    ColorRole::PRIMARY
                                                } else {
                                                    ColorRole::OUTLINE
                                                }),
                                            ),
                                        ),
                                )
                            }),
                    )
                    .when(show_tls, |d| {
                        d.child(
                            ui::text(
                                TypeRole::BodyMedium,
                                ColorRole::ON_SURFACE_VARIANT,
                                if !ok {
                                    "—".to_owned()
                                } else if e.sni.is_empty() {
                                    t("真实 SNI", "Real SNI").into()
                                } else {
                                    tf!("伪装 · {}", "Disguised · {}", e.sni)
                                },
                            )
                            .w(rem(COLUMNS[2])),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(rem(120.))
                            .flex()
                            .items_center()
                            .gap(rem(8.))
                            .when(in_use, |d| {
                                d.child(ui::pill(
                                    t("使用中", "In use"),
                                    ColorRole::PRIMARY,
                                    ColorRole::ON_PRIMARY,
                                ))
                            })
                            .child(ui::line(
                                TypeRole::BodyMedium,
                                if ok {
                                    ColorRole::ON_SURFACE_VARIANT
                                } else {
                                    ColorRole::ERROR
                                },
                                if ok {
                                    t("可直达", "Reachable").to_owned()
                                } else {
                                    e.error
                                        .clone()
                                        .unwrap_or_else(|| t("不可直达", "Unreachable").into())
                                },
                            )),
                    )
                    .child(div().w(rem(COLUMNS[3])).when(ok, |d| {
                        d.child(
                            IconButton::new(
                                ("pin", ix),
                                Symbol::KEEP,
                                if pinned {
                                    t("取消固定", "Unpin")
                                } else {
                                    t("固定使用此地址", "Pin this address")
                                },
                            )
                            .toggle(true)
                            .selected(pinned)
                            .selected_icon(Symbol::KEEP.icon().filled(true))
                            .on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    this.pin(if pinned { None } else { ip }, cx)
                                },
                            )),
                        )
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .rounded(rem(16.))
            .border_1()
            .border_color(color(ColorRole::OUTLINE_VARIANT))
            .overflow_hidden()
            .child(header)
            .children(body.into_iter().map(|row| {
                div()
                    .border_t_1()
                    .border_color(color(ColorRole::OUTLINE_VARIANT))
                    .child(row)
            }))
            .into_any_element()
    }
}
