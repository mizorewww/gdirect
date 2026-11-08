//! Signing in and out, switching accounts, and the background mount.
use crate::{app::Gdirect, ui};
use gdirect::{api::Api, store::Store};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::path::PathBuf;

impl Gdirect {
    /// Signs in in the browser, with a client JSON or the saved client.
    pub fn login(&mut self, client: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.signing_in {
            return;
        }
        self.signing_in = true;
        let db_path = self.db_path.clone();
        let current = self.account.clone();
        let reauth = client.is_none();
        self.say("已在浏览器中打开 Google 登录页…", cx);
        self.background(
            move || -> anyhow::Result<String> {
                let api = Api::new(db_path, current, None, None, false)?;
                api.login(reauth, client.as_deref(), None)
            },
            |this, result, cx| {
                this.signing_in = false;
                match result {
                    Ok(email) => {
                        this.say(format!("已登录 {email}（默认账号）"), cx);
                        this.reload_accounts();
                        this.reset_files();
                        this.connect(cx);
                    }
                    Err(e) => this.say(format!("登录失败：{e:#}"), cx),
                }
            },
        );
    }

    /// Asks for a Google Desktop OAuth client JSON and signs in with it.
    pub fn choose_client(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 OAuth 客户端 JSON".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await
                && let Some(path) = paths.pop()
            {
                let _ = this.update(cx, |this, cx| this.login(Some(path), cx));
            }
        })
        .detach();
    }

    pub fn add_account(&mut self, cx: &mut Context<Self>) {
        if self.oauth_client {
            self.login(None, cx);
        } else {
            self.choose_client(cx);
        }
    }

    fn reset_files(&mut self) {
        self.path.truncate(1);
        self.entries.clear();
        self.selected.clear();
        self.clipboard = None;
        self.list_error = None;
    }

    pub fn switch_account(&mut self, email: String, cx: &mut Context<Self>) {
        if self.account.as_ref() == Some(&email) {
            return;
        }
        if let Err(e) = self.db.set_default_account(&email) {
            self.say(format!("{e:#}"), cx);
            return;
        }
        self.reload_accounts();
        self.reset_files();
        self.say(format!("默认账号：{email}"), cx);
        self.connect(cx);
    }

    pub fn logout(&mut self, account: String, cx: &mut Context<Self>) {
        if self
            .db
            .setting(&Store::mount_key(&account))
            .ok()
            .flatten()
            .is_some()
        {
            self.say(format!("{account} 有后台挂载，请先卸载"), cx);
            return;
        }
        if let Err(e) = self.db.remove_account(&account) {
            self.say(format!("{e:#}"), cx);
            return;
        }
        if let Ok(Some(next)) = self.db.default_account() {
            let _ = self.db.set_default_account(&next);
        }
        let was_current = self.account.as_ref() == Some(&account);
        self.reload_accounts();
        self.say(format!("已退出 {account}"), cx);
        if was_current {
            self.reset_files();
            self.api = None;
            self.connect(cx);
        }
    }

    pub fn toggle_mount(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        if self.mount_busy {
            return;
        }
        self.mount_busy = true;
        let db_path = self.db_path.clone();
        if self.mount.is_some() {
            self.background(
                move || -> anyhow::Result<()> {
                    let db = Store::open(&db_path)?;
                    gdirect::mount::unmount(&db, Some(&account))
                },
                |this, result, cx| {
                    this.mount_busy = false;
                    this.reload_accounts();
                    match result {
                        Ok(()) => this.say("已卸载，缓存中的写入会保留到下次挂载", cx),
                        Err(e) => this.say(format!("卸载失败：{e:#}"), cx),
                    }
                },
            );
        } else {
            let path = self.settings.mount_path.clone();
            let read_only = self.settings.read_only;
            let connections = self.settings.connections;
            let pinned = self.settings.pinned;
            self.background(
                move || -> anyhow::Result<()> {
                    let db = Store::open(&db_path)?;
                    // rclone reaches Google through the verified endpoints; with
                    // none, it falls back to the system network.
                    let (api, endpoints) = match gdirect::route::resolve(&db, pinned, None) {
                        Ok(direct) => (
                            direct.api(db_path.clone(), Some(account))?,
                            direct.tunnel_endpoints(),
                        ),
                        Err(_) => (
                            Api::new(db_path, Some(account), None, None, false)?,
                            Vec::new(),
                        ),
                    };
                    gdirect::mount::run(&api, Some(&path), read_only, false, connections, endpoints)
                },
                |this, result, cx| {
                    this.mount_busy = false;
                    this.reload_accounts();
                    match result {
                        Ok(()) => this.say("已挂载，可以在访达中打开", cx),
                        Err(e) => this.say(format!("挂载失败：{e:#}"), cx),
                    }
                },
            );
        }
        cx.notify();
    }

    /// Shown until the first account signs in.
    pub fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .max_w(rem(560.))
                    .p(rem(32.))
                    .flex()
                    .flex_col()
                    .gap(rem(16.))
                    .child(
                        div()
                            .size(rem(72.))
                            .rounded(rem(24.))
                            .bg(color(ColorRole::PRIMARY_CONTAINER))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(ui::symbol("add_to_drive", true).size(rem(40.)).color(color(ColorRole::ON_PRIMARY_CONTAINER))),
                    )
                    .child(ui::text(TypeRole::HeadlineMedium, ColorRole::ON_SURFACE, "登录 Google 云盘"))
                    .child(ui::text(
                        TypeRole::BodyLarge,
                        ColorRole::ON_SURFACE_VARIANT,
                        "gdirect 用你自己的 Google 桌面应用 OAuth 客户端登录，直接连到可达的 Google 前端传输文件。选择从 Google Cloud 控制台下载的客户端 JSON，然后在浏览器中选择账号。",
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(rem(8.))
                            .pt(rem(8.))
                            .child(
                                MaterialButton::new("choose-client")
                                    .button_size(ButtonSize::Medium)
                                    .icon(Symbol::KEY)
                                    .loading(self.signing_in, "正在等待浏览器登录")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_client(cx)))
                                    .child("选择客户端 JSON…"),
                            )
                            .when(self.oauth_client, |d| {
                                d.child(
                                    MaterialButton::new("saved-client")
                                        .button_size(ButtonSize::Medium)
                                        .variant(ButtonVariant::Tonal)
                                        .icon(Symbol::LOGIN)
                                        .disabled(self.signing_in)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.login(None, cx)))
                                        .child("使用已保存的客户端"),
                                )
                            }),
                    )
                    .child(
                        notice_banner(
                            "welcome-tip",
                            NoticeKind::Info,
                            "应用处于测试阶段时，每个账号都要加为测试用户，登录 7 天后过期。",
                        )
                        .mt(rem(8.)),
                    ),
            )
    }
}
