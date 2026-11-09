//! The OAuth client form: the client ID and secret to sign in with, filled
//! in by hand, from a client JSON file or from the clipboard. The welcome
//! page and the account dialog both show it.
use crate::i18n::t;
use crate::tf;
use gdirect::auth::{CLIENT_ID_SUFFIX, Client, ClientError};
use gpui_kit::base::input::{InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;

/// Where to create a client and copy its values.
pub const CONSOLE: &str = "https://console.cloud.google.com/auth/clients";

/// The setup guide, at `anchor` when given, in the interface language.
pub fn guide(anchor: Option<&str>) -> String {
    let page = if crate::i18n::chinese() {
        "google-cloud.zh-CN.md"
    } else {
        "google-cloud.md"
    };
    let base = format!("https://github.com/mizorewww/gdirect/blob/main/docs/configuration/{page}");
    match anchor {
        Some(anchor) => format!("{base}#{anchor}"),
        None => base,
    }
}

pub enum ClientFormEvent {
    /// Enter was pressed in a field.
    Submit,
    /// A message for the snackbar.
    Say(SharedString),
}

/// What filling in the form from a file or the clipboard came to, shown
/// under an inline form until either field changes.
struct Notice {
    text: SharedString,
    ok: bool,
    /// The fields as the notice found them.
    values: (String, String),
}

/// Where a client was read from, for the wording of what was wrong with it.
enum Source<'a> {
    Clipboard,
    File(&'a str),
}

pub struct ClientForm {
    pub id: Entity<InputState>,
    pub secret: Entity<InputState>,
    /// Reports under the form rather than in the snackbar, which would cover
    /// the welcome page's sign-in button.
    inline: bool,
    notice: Option<Notice>,
    show_secret: bool,
    /// Left once, so a malformed value shows its error; an empty one waits
    /// for a submit.
    id_left: bool,
    secret_left: bool,
    submitted: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ClientFormEvent> for ClientForm {}

impl ClientForm {
    pub fn new(inline: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let id = cx.new(|cx| InputState::new(window, cx));
        let secret = cx.new(|cx| InputState::new(window, cx));
        let subs = [&id, &secret]
            .into_iter()
            .enumerate()
            .map(|(ix, input)| {
                cx.subscribe_in(
                    input,
                    window,
                    move |this, state, event: &InputEvent, window, cx| match event {
                        InputEvent::Change => {
                            let value = state.read(cx).value().to_string();
                            this.recognise(&value, window, cx);
                            let values = this.values(cx);
                            if this.notice.as_ref().is_some_and(|n| n.values != values) {
                                this.notice = None;
                            }
                            cx.notify();
                        }
                        InputEvent::Blur => {
                            if ix == 0 {
                                this.id_left = true;
                            } else {
                                this.secret_left = true;
                            }
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } => cx.emit(ClientFormEvent::Submit),
                        InputEvent::Focus => {}
                    },
                )
            })
            .collect();
        Self {
            id,
            secret,
            inline,
            notice: None,
            show_secret: false,
            id_left: false,
            secret_left: false,
            submitted: false,
            _subs: subs,
        }
    }

    /// Fills in both fields, as a fresh form.
    pub fn fill(&mut self, client: Option<&Client>, window: &mut Window, cx: &mut Context<Self>) {
        let (id, secret) = client
            .map(|c| (c.client_id.clone(), c.client_secret.clone()))
            .unwrap_or_default();
        self.id
            .update(cx, |input, cx| input.set_value(id, window, cx));
        self.secret
            .update(cx, |input, cx| input.set_value(secret, window, cx));
        self.show_secret = false;
        self.id_left = false;
        self.secret_left = false;
        self.submitted = false;
        self.notice = None;
        cx.notify();
    }

    fn values(&self, cx: &App) -> (String, String) {
        (
            self.id.read(cx).value().to_string(),
            self.secret.read(cx).value().to_string(),
        )
    }

    /// Reports how filling in the form went: under the form when it is
    /// inline, otherwise in the snackbar.
    fn announce(&mut self, text: String, ok: bool, cx: &mut Context<Self>) {
        if self.inline {
            self.notice = Some(Notice {
                text: text.into(),
                ok,
                values: self.values(cx),
            });
            cx.notify();
        } else {
            cx.emit(ClientFormEvent::Say(text.into()));
        }
    }

    /// A whole client pasted into either field fills in both. A single-line
    /// field drops the new lines of what is pasted, so text that only reads
    /// as a client with them is taken from the clipboard itself.
    fn recognise(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let value = value.trim();
        if value.is_empty() || (!value.contains(char::is_whitespace) && valid_part(value)) {
            return;
        }
        let client = Client::from_text(value).ok().or_else(|| {
            let pasted = cx.read_from_clipboard()?.text()?;
            let joined = pasted.replace('\n', "");
            (!joined.trim().is_empty() && value.contains(joined.trim()))
                .then(|| Client::from_text(&pasted).ok())
                .flatten()
        });
        if let Some(client) = client {
            self.fill(Some(&client), window, cx);
            self.announce(filled_from_clipboard().to_owned(), true, cx);
        }
    }

    pub fn client(&self, cx: &App) -> Client {
        Client::new(&self.id.read(cx).value(), &self.secret.read(cx).value())
    }

    pub fn valid(&self, cx: &App) -> bool {
        self.client(cx).validate().is_ok()
    }

    /// The client when both fields are valid; otherwise shows every error.
    pub fn submit(&mut self, cx: &mut Context<Self>) -> Option<Client> {
        self.submitted = true;
        cx.notify();
        let client = self.client(cx);
        client.validate().is_ok().then_some(client)
    }

    fn id_error(&self, cx: &App) -> Option<&'static str> {
        let id = self.client(cx).client_id;
        if id.is_empty() {
            return self
                .submitted
                .then(|| t("请填写客户端 ID", "Enter the client ID"));
        }
        let shown = self.submitted || self.id_left;
        (shown && Client::new(&id, "x").validate() == Err(ClientError::ClientId)).then(|| {
            t(
                "这看起来不是客户端 ID，它应以 .apps.googleusercontent.com 结尾",
                "This doesn’t look like a client ID; it should end in .apps.googleusercontent.com",
            )
        })
    }

    fn secret_error(&self, cx: &App) -> Option<&'static str> {
        let secret = self.client(cx).client_secret;
        if secret.is_empty() {
            return self
                .submitted
                .then(|| t("请填写客户端密钥", "Enter the client secret"));
        }
        let shown = self.submitted || self.secret_left;
        (shown && secret.ends_with(CLIENT_ID_SUFFIX)).then(|| {
            t(
                "这是客户端 ID，不是密钥",
                "This is a client ID, not the secret",
            )
        })
    }

    /// Asks for a client JSON file and fills in the form from it.
    pub fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("导入", "Import").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = rx.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let read = std::fs::read(&path);
            let _ = this.update_in(cx, |this, window, cx| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let (message, ok) = match read {
                    Err(e) => (
                        tf!("无法读取 {name}：{e}", "Couldn’t read {name}: {e}"),
                        false,
                    ),
                    Ok(data) => match Client::from_json(&data) {
                        Ok(client) => {
                            this.fill(Some(&client), window, cx);
                            (
                                tf!("已从 {name} 填入客户端", "Client filled in from {name}"),
                                true,
                            )
                        }
                        Err(e) => (problem(e, Source::File(&name)), false),
                    },
                };
                this.announce(message, ok, cx);
            });
        })
        .detach();
    }

    pub fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        let (message, ok) = if text.trim().is_empty() {
            (
                t("剪贴板是空的", "The clipboard is empty").to_owned(),
                false,
            )
        } else {
            match Client::from_text(&text) {
                Ok(client) => {
                    self.fill(Some(&client), window, cx);
                    (filled_from_clipboard().to_owned(), true)
                }
                Err(e) => (problem(e, Source::Clipboard), false),
            }
        };
        self.announce(message, ok, cx);
    }
}

/// Text that is already a whole client ID or a plausible secret by itself.
fn valid_part(value: &str) -> bool {
    !value.contains(['{', ',', '"'])
        && (Client::new(value, "x").validate().is_ok() || !value.contains(CLIENT_ID_SUFFIX))
}

fn filled_from_clipboard() -> &'static str {
    t(
        "已从剪贴板填入客户端",
        "Client filled in from the clipboard",
    )
}

/// Why what was read from `source` is not a client, as a sentence.
fn problem(e: ClientError, source: Source) -> String {
    match (source, e) {
        (Source::Clipboard, ClientError::Unrecognized) => t(
            "剪贴板里没有 OAuth 客户端信息。请复制下载的客户端 JSON，或者客户端 ID 和密钥。",
            "Nothing in the clipboard looks like an OAuth client. Copy the downloaded client JSON, or the client ID and secret.",
        )
        .into(),
        (Source::File(name), ClientError::Unrecognized) => tf!(
            "{name} 不是桌面应用的 OAuth 客户端。请选择从 Google Cloud 下载的客户端 JSON 文件。",
            "{name} isn’t a Desktop app OAuth client. Choose the client JSON file you downloaded from Google Cloud."
        ),
        (Source::Clipboard, ClientError::WebClient) => t(
            "剪贴板里是“Web 应用”客户端。请改为创建“桌面应用”类型的客户端。",
            "The clipboard holds a web application client. Create a Desktop app client instead.",
        )
        .into(),
        (Source::File(name), ClientError::WebClient) => tf!(
            "{name} 是“Web 应用”客户端。请改为创建“桌面应用”类型的客户端。",
            "{name} is a web application client. Create a Desktop app client instead."
        ),
        (Source::Clipboard, ClientError::ClientId) => t(
            "剪贴板里的客户端 ID 不对，它应以 .apps.googleusercontent.com 结尾。",
            "The client ID in the clipboard isn’t valid; it should end in .apps.googleusercontent.com.",
        )
        .into(),
        (Source::File(name), ClientError::ClientId) => tf!(
            "{name} 里的客户端 ID 不对，它应以 .apps.googleusercontent.com 结尾。",
            "The client ID in {name} isn’t valid; it should end in .apps.googleusercontent.com."
        ),
        (Source::Clipboard, ClientError::MissingSecret) => t(
            "剪贴板里有客户端 ID，但缺少客户端密钥。",
            "The clipboard has a client ID but no client secret.",
        )
        .into(),
        (Source::File(name), ClientError::MissingSecret) => tf!(
            "{name} 里缺少客户端密钥。",
            "{name} has no client secret."
        ),
    }
}

/// The form's two fields and their guidance; `compact` for a dialog, and
/// `disabled` while a sign-in waits.
pub fn fields(form: &Entity<ClientForm>, compact: bool, disabled: bool, cx: &App) -> Div {
    let this = form.read(cx);
    let help = |id: &'static str| {
        material_button(id, ButtonVariant::Text)
            .button_size(ButtonSize::ExtraSmall)
            .on_click(|_, _, cx| cx.open_url(&guide(Some("client-id-and-secret"))))
            .accessibility_label(t("在哪里找到？", "Where do I find this?"))
            .child(t("在哪里找到？", "Where do I find this?"))
    };
    let field = |input: TextField,
                 help_id: &'static str,
                 error: Option<&'static str>,
                 hint: &'static str| {
        let input = input
            .variant(TextFieldVariant::Outlined)
            .error(error.is_some())
            .disabled(disabled)
            .w_full();
        // A dialog keeps the hint and its link on one line where they fit.
        if compact {
            return div()
                .flex()
                .flex_col()
                .child(input.when_some(error, |input, e| input.supporting_text(e)))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(rem(4.))
                        .child(
                            crate::ui::text(
                                TypeRole::BodySmall,
                                ColorRole::ON_SURFACE_VARIANT,
                                hint,
                            )
                            .pl(rem(16.)),
                        )
                        .child(help(help_id)),
                );
        }
        // An error comes first; the hint stays under it.
        div()
            .flex()
            .flex_col()
            .child(input.when_some(error, |input, e| input.supporting_text(e)))
            .child(
                crate::ui::text(TypeRole::BodySmall, ColorRole::ON_SURFACE_VARIANT, hint)
                    .px(rem(16.))
                    .pt(rem(4.)),
            )
            .child(div().flex().child(help(help_id)))
    };
    let (id_hint, secret_hint) = if compact {
        (
            t(
                "Google Cloud 控制台 › Google Auth Platform › 客户端",
                "Google Auth Platform › Clients",
            ),
            t(
                "创建客户端时显示，也在下载的 JSON 中",
                "Shown on creation, and in its JSON",
            ),
        )
    } else {
        (
            t(
                "在 Google Cloud 控制台 › Google Auth Platform › 客户端 中，打开你的桌面应用客户端即可复制。",
                "In Google Cloud console › Google Auth Platform › Clients, open your Desktop app client and copy it.",
            ),
            t(
                "创建客户端时弹出的窗口里会显示，也包含在下载的 JSON 中。找不到时可在客户端详情里添加新的密钥。",
                "Shown in the dialog when you create the client, and inside the downloaded JSON. If you’ve lost it, add a new secret in the client’s details.",
            ),
        )
    };
    let show = this.show_secret;
    let toggle = {
        let form = form.clone();
        IconButton::new(
            "client-secret-visibility",
            if show {
                Symbol::VISIBILITY_OFF
            } else {
                Symbol::VISIBILITY
            },
            if show {
                t("隐藏密钥", "Hide secret")
            } else {
                t("显示密钥", "Show secret")
            },
        )
        .disabled(disabled)
        .on_click(move |_, _, cx| {
            form.update(cx, |form, cx| {
                form.show_secret = !form.show_secret;
                cx.notify();
            })
        })
    };
    div()
        .flex()
        .flex_col()
        .gap(rem(8.))
        .child(field(
            TextField::new("client-id", &this.id)
                .label(t("客户端 ID", "Client ID"))
                .placeholder(format!("xxxx{CLIENT_ID_SUFFIX}"))
                .required(true),
            "client-id-help",
            this.id_error(cx),
            id_hint,
        ))
        .child(field(
            TextField::new("client-secret", &this.secret)
                .label(t("客户端密钥", "Client secret"))
                .placeholder("GOCSPX-…")
                .required(true)
                .input_type(if show {
                    TextFieldInputType::Text
                } else {
                    TextFieldInputType::Password
                })
                .trailing_icon(toggle),
            "client-secret-help",
            this.secret_error(cx),
            secret_hint,
        ))
}

/// Import JSON file… and Paste from clipboard.
pub fn import_buttons(
    form: &Entity<ClientForm>,
    variant: ButtonVariant,
    disabled: bool,
) -> [AnyElement; 2] {
    let file = form.clone();
    let clipboard = form.clone();
    [
        material_button("client-import", variant)
            .disabled(disabled)
            .icon(Symbol::UPLOAD_FILE)
            .on_click(move |_, window, cx| file.update(cx, |form, cx| form.import_file(window, cx)))
            .accessibility_label(t("从 JSON 文件导入…", "Import JSON file…"))
            .child(t("从 JSON 文件导入…", "Import JSON file…"))
            .into_any_element(),
        material_button("client-paste", variant)
            .disabled(disabled)
            .icon(Symbol::CONTENT_PASTE)
            .on_click(move |_, window, cx| clipboard.update(cx, |form, cx| form.paste(window, cx)))
            .accessibility_label(t("从剪贴板粘贴", "Paste from clipboard"))
            .child(t("从剪贴板粘贴", "Paste from clipboard"))
            .into_any_element(),
    ]
}

/// The inline form's last report on filling it in, until it is edited.
pub fn notice(form: &Entity<ClientForm>, cx: &App) -> Option<Div> {
    let notice = form.read(cx).notice.as_ref()?;
    let (icon, ink) = if notice.ok {
        ("check_circle", ColorRole::PRIMARY)
    } else {
        ("error", ColorRole::ERROR)
    };
    Some(
        div()
            .flex()
            .items_start()
            .gap(rem(8.))
            .child(
                div()
                    .flex_none()
                    .mt(rem(1.))
                    .child(crate::ui::icon(icon, 18., ink)),
            )
            .child(
                crate::ui::latin(crate::ui::text(
                    TypeRole::BodyMedium,
                    if notice.ok {
                        ColorRole::ON_SURFACE_VARIANT
                    } else {
                        ColorRole::ERROR
                    },
                    notice.text.clone(),
                ))
                .flex_1()
                .min_w_0(),
            ),
    )
}

/// The client ID with its middle elided, keeping where it starts and its
/// Google suffix.
pub fn short_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    if chars.len() <= 40 {
        return id.to_owned();
    }
    let tail = CLIENT_ID_SUFFIX.chars().count() + 4;
    let head: String = chars[..10].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}…{end}")
}

/// The secret as bullets, keeping only Google's public prefix.
pub fn masked_secret(secret: &str) -> String {
    if secret.starts_with("GOCSPX-") {
        "GOCSPX-••••".into()
    } else {
        "••••••••".into()
    }
}

#[cfg(test)]
mod tests {
    use super::{masked_secret, short_id, valid_part};
    #[test]
    fn short_id_keeps_both_ends() {
        let id = "123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com";
        let short = short_id(id);
        assert!(short.starts_with("1234567890…"));
        assert!(short.ends_with("2345.apps.googleusercontent.com"));
        assert_eq!(
            short_id("1-a.apps.googleusercontent.com"),
            "1-a.apps.googleusercontent.com"
        );
        assert_eq!(masked_secret("GOCSPX-abc"), "GOCSPX-••••");
        assert!(!masked_secret("legacy").contains("legacy"));
    }
    #[test]
    fn single_values_are_left_alone() {
        assert!(valid_part("123-abc.apps.googleusercontent.com"));
        assert!(valid_part("GOCSPX-abc"));
        assert!(!valid_part("123-abc.apps.googleusercontent.comGOCSPX-abc"));
        assert!(!valid_part("{\"installed\":{}}"));
    }
}
