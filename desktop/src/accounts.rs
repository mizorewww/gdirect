//! Signing in and out, switching accounts, and the background mount.
use crate::i18n::t;
use crate::tf;
use crate::{
    app::{Dialog, Gdirect},
    client_form, ui,
};
use gdirect::{
    api::Api,
    auth::{Client, LoginCancelled},
    store::Store,
};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

impl Gdirect {
    /// Signs in in the browser with `client`, which is saved once it works.
    /// A sign-in still waiting is dropped for this one.
    pub fn login(&mut self, client: Client, cx: &mut Context<Self>) {
        self.abandon_login();
        self.login_serial += 1;
        let serial = self.login_serial;
        self.signing_in = true;
        self.login_error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.login_cancel = Some(cancel.clone());
        let db_path = self.db_path.clone();
        let current = self.account.clone();
        let tx = self.sender();
        self.background(
            move || -> anyhow::Result<String> {
                // The page opens now; its address is kept to open it again.
                let open = move |url: &str| {
                    gdirect::auth::open_in_browser(url);
                    let url = url.to_owned();
                    let _ = tx.send(Box::new(move |this: &mut Gdirect, cx| {
                        if this.login_serial == serial {
                            this.login_url = Some(url);
                            cx.notify();
                        }
                    }));
                };
                Api::new(db_path, current, None, None, false)?
                    .cancellable(cancel)
                    .login_with_client_opening(&client, &open)
            },
            move |this, result, cx| {
                // A sign-in dropped for a later one, or cancelled, no longer
                // applies; cancelled, it saved nothing.
                if this.login_serial != serial {
                    return;
                }
                this.signing_in = false;
                this.login_cancel = None;
                this.login_url = None;
                match result {
                    Ok(email) => {
                        if let Some(Dialog::Client { .. }) = this.dialog {
                            this.dialog = None;
                            this.focus_page = true;
                        }
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
                    Err(e) if e.is::<LoginCancelled>() => {}
                    Err(e) => {
                        let message: SharedString =
                            tf!("登录失败：{e:#}", "Sign-in failed: {e:#}").into();
                        // The welcome page and the dialog show it by the
                        // button that retries.
                        if this.dialog.is_some() || this.welcome() {
                            this.login_error = Some(message);
                        } else {
                            this.say(message, cx);
                        }
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Whether the welcome page is on screen.
    pub fn welcome(&self) -> bool {
        self.accounts.is_empty() && self.page != crate::app::Page::Settings
    }

    /// Stops the sign-in waiting, if any, and drops what it comes to.
    fn abandon_login(&mut self) -> bool {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let waiting = std::mem::take(&mut self.signing_in);
        self.login_serial += 1;
        self.login_url = None;
        waiting
    }

    /// Stops waiting for the browser sign-in.
    pub fn cancel_login(&mut self, cx: &mut Context<Self>) {
        if self.abandon_login() && !self.welcome() && self.dialog.is_none() {
            self.say(t("已取消登录", "Sign-in cancelled"), cx);
        }
        cx.notify();
    }

    /// Opens the waiting sign-in's page again, for a tab closed too soon.
    pub fn reopen_login(&mut self) {
        if let Some(url) = &self.login_url {
            gdirect::auth::open_in_browser(url);
        }
    }

    /// Acts on the client form: the welcome page's or the dialog's.
    pub fn submit_client(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let in_dialog = matches!(self.dialog, Some(Dialog::Client { .. }));
        let client = if in_dialog && self.client_summary {
            self.oauth_client.clone()
        } else if in_dialog {
            self.dialog_form.update(cx, |form, cx| form.submit(cx))
        } else {
            self.client_form.update(cx, |form, cx| form.submit(cx))
        };
        let Some(client) = client else {
            return;
        };
        if let Some(Dialog::Client { sign_in: false }) = self.dialog {
            if let Err(e) = gdirect::auth::save_client(&self.db, &client) {
                self.say(format!("{e:#}"), cx);
                return;
            }
            self.reload_accounts();
            self.say(t("已保存 OAuth 客户端", "OAuth client saved"), cx);
            self.dialog = None;
            self.focus_page = true;
            self.focus_now(window, cx);
        } else {
            // The dialog stays up, showing the wait, until the sign-in ends.
            self.login(client, cx);
        }
        cx.notify();
    }

    /// Signs in another account, through the client form. A sign-in still
    /// waiting (its tab closed, say) is dropped for a fresh one.
    pub fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.abandon_login();
        self.open_dialog(Dialog::Client { sign_in: true }, window, cx);
    }

    /// The wait for the browser: a spinner and what to do, Cancel beside
    /// it where asked, and the page to open again.
    pub fn render_waiting(&self, cancel: bool, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(rem(8.))
            .gap_y(rem(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rem(12.))
                    .pr(rem(4.))
                    .child(
                        CircularProgress::new(
                            "sign-in-progress",
                            t("正在等待浏览器登录", "Waiting for sign-in in the browser"),
                        )
                        .size(20.),
                    )
                    .child(ui::text(
                        TypeRole::BodyMedium,
                        ColorRole::ON_SURFACE,
                        t("请在浏览器中继续…", "Continue in your browser…"),
                    )),
            )
            .when(cancel, |d| {
                d.child(
                    material_button("cancel-sign-in", ButtonVariant::Text)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.cancel_login(cx)))
                        .accessibility_label(t("取消", "Cancel"))
                        .child(t("取消", "Cancel")),
                )
            })
            .when(self.login_url.is_some(), |d| {
                d.child(
                    material_button("reopen-sign-in", ButtonVariant::Text)
                        .icon(Symbol::OPEN_IN_NEW)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, _| this.reopen_login()))
                        .accessibility_label(t("重新打开登录页面", "Open the sign-in page again"))
                        .child(t("重新打开登录页面", "Open the sign-in page again")),
                )
            })
    }

    /// Why the last sign-in failed, in the error colour.
    pub fn render_login_error(&self) -> Option<Div> {
        let message = self.login_error.clone()?;
        Some(
            div()
                .flex()
                .items_start()
                .gap(rem(8.))
                .child(div().flex_none().mt(rem(1.)).child(ui::icon(
                    "error",
                    18.,
                    ColorRole::ERROR,
                )))
                .child(
                    ui::latin(ui::text(TypeRole::BodyMedium, ColorRole::ERROR, message))
                        .flex_1()
                        .min_w_0(),
                ),
        )
    }

    /// Copies the saved client as JSON, which holds its secret.
    pub fn copy_client(&mut self, cx: &mut Context<Self>) {
        let Some(client) = &self.oauth_client else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(client.to_json()));
        self.say(
            t(
                "已复制客户端 JSON。其中包含客户端密钥，请勿公开分享",
                "Copied the client JSON. It contains the client secret; don’t share it publicly.",
            ),
            cx,
        );
    }

    /// Saves the client as a JSON file only its owner can read.
    pub fn export_client(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.oauth_client.clone() else {
            return;
        };
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let rx = cx.prompt_for_new_path(&dir, Some("client_secret.json"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else {
                return;
            };
            let result = gdirect::auth::export_client(&client, &path);
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => this.say(
                    tf!(
                        "已导出到 {}。其中包含客户端密钥，请妥善保管",
                        "Exported to {}. It contains the client secret; keep it private.",
                        path.display()
                    ),
                    cx,
                ),
                Err(e) => this.say(tf!("无法导出：{e:#}", "Couldn’t export: {e:#}"), cx),
            });
        })
        .detach();
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
        // Under a step's text, past its number.
        let indented = || div().pl(rem(44.));
        let valid = self.client_form.read(cx).valid(cx);
        let waiting = self.signing_in;
        let [import, paste] =
            client_form::import_buttons(&self.client_form, ButtonVariant::Text, waiting);
        let notice = client_form::notice(&self.client_form, cx);
        // The column keeps its own height, taller than a short window, and
        // the page scrolls to its buttons; a stretched one would be clipped.
        div()
            .id("welcome")
            .size_full()
            .overflow_y_scrollbar()
            .flex()
            .justify_center()
            .items_start()
            .child(
                div()
                    .flex_none()
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
                    .child(ui::text(TypeRole::HeadlineMedium, ColorRole::ON_SURFACE, t("连接你的 Google 云端硬盘", "Connect your Google Drive")))
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
                            .child(step(
                                "1",
                                t("创建你的客户端", "Create your client"),
                                t(
                                    "在 Google Cloud 控制台新建项目，启用 Google Drive API，配置同意屏幕并把你的 Google 账号加为测试用户，然后创建“桌面应用”类型的 OAuth 客户端。配置指南里每一步都有截图。",
                                    "In the Google Cloud console, create a project, enable the Google Drive API, set up the consent screen and add your Google account as a test user, then create an OAuth client of type “Desktop app”. The setup guide shows every step with screenshots.",
                                ),
                            ))
                            .child(
                                indented().flex().flex_wrap().gap(rem(8.)).child(
                                    MaterialButton::new("open-guide")
                                        .variant(ButtonVariant::Tonal)
                                        .icon(Symbol::OPEN_IN_NEW)
                                        .on_click(|_, _, cx| cx.open_url(&client_form::guide(None)))
                                        .accessibility_label(t("打开配置指南", "Open the setup guide"))
                                        .child(t("打开配置指南", "Open the setup guide")),
                                )
                                .child(
                                    MaterialButton::new("open-console")
                                        .variant(ButtonVariant::Outlined)
                                        .icon(Symbol::OPEN_IN_NEW)
                                        .on_click(|_, _, cx| cx.open_url(client_form::CONSOLE))
                                        .accessibility_label(t("打开 Google Cloud 控制台", "Open Google Cloud console"))
                                        .child(t("打开 Google Cloud 控制台", "Open Google Cloud console")),
                                ),
                            )
                            .child(step(
                                "2",
                                t("填写 OAuth 客户端", "Enter your OAuth client"),
                                t(
                                    "填写客户端 ID 和密钥，或者导入下载的 JSON 文件、从剪贴板粘贴。gdirect 会保存它，以后不必再填。",
                                    "Enter the client ID and secret, or import the downloaded JSON file or paste from the clipboard. gdirect keeps a copy, so you only do this once.",
                                ),
                            ))
                            .child(
                                indented()
                                    .flex()
                                    .flex_col()
                                    .gap(rem(8.))
                                    .child(client_form::fields(&self.client_form, false, waiting, cx))
                                    .child(div().flex().flex_wrap().gap(rem(4.)).child(import).child(paste))
                                    .children(notice.map(|n| n.pl(rem(12.)))),
                            )
                            .child(step(
                                "3",
                                t("在浏览器里登录", "Sign in with your browser"),
                                t(
                                    "浏览器会打开 Google 登录页。看到“Google 尚未验证此应用”时点“继续”——这是你自己创建的应用。授权后回到这里，文件就会出现。",
                                    "Your browser opens Google’s sign-in page. If it says “Google hasn’t verified this app”, choose Continue — it is the app you just created. After you allow access, come back here and your files appear.",
                                ),
                            ))
                            // Waiting, the button gives way to the wait itself, in
                            // its place: on a short window it stays in view.
                            .child(indented().child(if waiting {
                                self.render_waiting(true, cx)
                            } else {
                                div().flex().child(
                                    MaterialButton::new("sign-in")
                                        .button_size(ButtonSize::Medium)
                                        .icon(Symbol::LOGIN)
                                        .disabled(!valid)
                                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.submit_client(window, cx)))
                                        .accessibility_label(t("使用 Google 账号登录", "Sign in with Google"))
                                        .child(t("使用 Google 账号登录", "Sign in with Google")),
                                )
                            }))
                            .children(self.render_login_error().map(|e| indented().child(e))),
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
