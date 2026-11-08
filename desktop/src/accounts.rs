//! Signing in and out, switching accounts, and the background mount.
use crate::i18n::t;
use crate::tf;
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
        self.say(
            t(
                "已在浏览器中打开 Google 登录页…",
                "Opened Google sign-in in your browser…",
            ),
            cx,
        );
        self.background(
            move || -> anyhow::Result<String> {
                let api = Api::new(db_path, current, None, None, false)?;
                api.login(reauth, client.as_deref(), None)
            },
            |this, result, cx| {
                this.signing_in = false;
                match result {
                    Ok(email) => {
                        this.say(
                            tf!(
                                "已登录 {email}（默认账号）",
                                "Signed in as {email} (default account)"
                            ),
                            cx,
                        );
                        this.reload_accounts();
                        this.reset_files();
                        this.connect(cx);
                    }
                    Err(e) => this.say(tf!("登录失败：{e:#}", "Sign-in failed: {e:#}"), cx),
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
            prompt: Some(t("选择 OAuth 客户端 JSON", "Choose OAuth client JSON").into()),
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
        // Nothing done through the previous account's client applies now.
        self.api = None;
        self.loading = false;
        self.location = crate::places::Location::new(crate::places::Place::MyDrive);
        self.before_search = None;
        self.clear_search = true;
        self.next_page = None;
        self.undo = None;
        self.drives.clear();
        self.about = None;
        self.root_id = None;
        self.folder_names.clear();
        self.naming.clear();
        self.entries.clear();
        self.selected.clear();
        self.filter.clear();
        self.clear_filter = true;
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
        self.say(tf!("默认账号：{email}", "Default account: {email}"), cx);
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
            self.say(
                tf!(
                    "{account} 有后台挂载，请先卸载",
                    "{account} has a background mount. Unmount it first."
                ),
                cx,
            );
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
        self.say(tf!("已退出 {account}", "Signed out of {account}"), cx);
        if was_current {
            self.reset_files();
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
                        Ok(()) => this.say(
                            t(
                                "已卸载，缓存中的写入会保留到下次挂载",
                                "Unmounted. Cached writes are kept until the next mount.",
                            ),
                            cx,
                        ),
                        Err(e) => this.say(tf!("卸载失败：{e:#}", "Couldn’t unmount: {e:#}"), cx),
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
                        Ok(()) => this.say(
                            t(
                                "已挂载，可以在访达中打开",
                                "Mounted. You can open it in Finder.",
                            ),
                            cx,
                        ),
                        Err(e) => this.say(tf!("挂载失败：{e:#}", "Couldn’t mount: {e:#}"), cx),
                    }
                },
            );
        }
        cx.notify();
    }

    /// Shown until the first account signs in.
    pub fn render_welcome(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let guide = if crate::i18n::chinese() {
            "https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.zh-CN.md"
        } else {
            "https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.md"
        };
        // A numbered step: what to do, then why or how.
        let step = |n: &'static str, title: &'static str, body: &'static str| {
            div()
                .flex()
                .gap(rem(16.))
                .child(
                    ui::text(TypeRole::TitleSmall, ColorRole::ON_PRIMARY_CONTAINER, n)
                        .flex_none()
                        .size(rem(28.))
                        .rounded_full()
                        .bg(color(ColorRole::PRIMARY_CONTAINER))
                        .flex()
                        .items_center()
                        .justify_center(),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(rem(4.))
                        .child(ui::text(
                            TypeRole::TitleMedium,
                            ColorRole::ON_SURFACE,
                            title,
                        ))
                        .child(ui::text(
                            TypeRole::BodyMedium,
                            ColorRole::ON_SURFACE_VARIANT,
                            body,
                        )),
                )
        };
        div()
            .id("welcome")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(rem(600.))
                    .px(rem(24.))
                    .py(rem(40.))
                    .flex()
                    .flex_col()
                    .gap(rem(20.))
                    .child(
                        div()
                            .size(rem(64.))
                            .rounded(rem(20.))
                            .bg(color(ColorRole::PRIMARY_CONTAINER))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(ui::symbol("add_to_drive", true).size(rem(36.)).color(color(ColorRole::ON_PRIMARY_CONTAINER))),
                    )
                    .child(ui::text(TypeRole::HeadlineMedium, ColorRole::ON_SURFACE, t("连接你的 Google 云盘", "Connect your Google Drive")))
                    .child(ui::text(
                        TypeRole::BodyLarge,
                        ColorRole::ON_SURFACE_VARIANT,
                        t(
                            "gdirect 不内置共用的登录凭据：你要先在 Google Cloud 里免费创建一个只属于你的“桌面应用”客户端，再用它登录。只需做一次，大约 10 分钟。",
                            "gdirect has no shared sign-in credentials built in: you first create a free “Desktop app” client of your own in Google Cloud, then sign in with it. You do this once; it takes about 10 minutes.",
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(rem(16.))
                            .p(rem(20.))
                            .rounded(rem(20.))
                            .bg(color(ColorRole::SURFACE_CONTAINER_LOW))
                            .child(
                                step(
                                    "1",
                                    t("创建你的客户端", "Create your client"),
                                    t(
                                        "在 Google Cloud 控制台新建项目，启用 Google Drive API，配置同意屏幕并把你的 Google 账号加为测试用户，然后创建“桌面应用”类型的 OAuth 客户端，下载它的 JSON 文件。配置指南里每一步都有截图。",
                                        "In the Google Cloud console, create a project, enable the Google Drive API, set up the consent screen and add your Google account as a test user, then create an OAuth client of type “Desktop app” and download its JSON file. The setup guide shows every step with screenshots.",
                                    ),
                                ),
                            )
                            .child(
                                div().pl(rem(44.)).child(
                                    MaterialButton::new("open-guide")
                                        .variant(ButtonVariant::Tonal)
                                        .icon(Symbol::OPEN_IN_NEW)
                                        .on_click(move |_, _, cx| cx.open_url(guide))
                                        .child(t("打开配置指南", "Open the setup guide")),
                                ),
                            )
                            .child(step(
                                "2",
                                t("选择下载的 JSON 文件", "Choose the downloaded JSON file"),
                                t(
                                    "点下面的按钮，选中刚下载的 client_secret_….json。gdirect 会保存它，以后不再需要这个文件。",
                                    "Click the button below and pick the client_secret_….json you just downloaded. gdirect keeps a copy, so you won’t need the file again.",
                                ),
                            ))
                            .child(step(
                                "3",
                                t("在浏览器里登录", "Sign in in your browser"),
                                t(
                                    "浏览器会打开 Google 登录页。看到“Google 尚未验证此应用”时点“继续”——这是你自己创建的应用。授权后回到这里，文件就会出现。",
                                    "Your browser opens Google’s sign-in page. If it says “Google hasn’t verified this app”, choose Continue — it is the app you just created. After you allow access, come back here and your files appear.",
                                ),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(rem(8.))
                            .child(
                                MaterialButton::new("choose-client")
                                    .button_size(ButtonSize::Medium)
                                    .icon(Symbol::KEY)
                                    .loading(self.signing_in, t("正在等待浏览器登录", "Waiting for browser sign-in"))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_client(cx)))
                                    .child(t("选择客户端 JSON 并登录…", "Choose client JSON and sign in…")),
                            )
                            .when(self.oauth_client, |d| {
                                d.child(
                                    MaterialButton::new("saved-client")
                                        .button_size(ButtonSize::Medium)
                                        .variant(ButtonVariant::Tonal)
                                        .icon(Symbol::LOGIN)
                                        .disabled(self.signing_in)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.login(None, cx)))
                                        .child(t("用已保存的客户端登录", "Sign in with the saved client")),
                                )
                            }),
                    )
                    .child(notice_banner(
                        "welcome-network",
                        NoticeKind::Info,
                        t(
                            "登录这一步需要能访问 Google（accounts.google.com）。如果 Google 被屏蔽，请暂时开着代理；登录后浏览和传输文件都不需要代理。",
                            "Signing in needs access to Google (accounts.google.com). If Google is blocked, keep a proxy on for this step only; browsing and transfers afterwards need no proxy.",
                        ),
                    ))
                    .child(notice_banner(
                        "welcome-testing",
                        NoticeKind::Info,
                        t(
                            "客户端保持“测试”状态时，只有加为测试用户的账号能登录，而且登录 7 天后失效；到时在“设置 › 账号”里重新登录即可。",
                            "While your client stays in “Testing”, only accounts added as test users can sign in, and a sign-in lasts 7 days; after that, sign in again under Settings › Account.",
                        ),
                    )),
            )
    }
}
