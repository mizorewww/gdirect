use crate::store::Store;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub const CONFIG_GUIDE: &str =
    "https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.md";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive";

// Only these fields are kept, so an imported token_uri or credential source is ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "authorized_user")]
pub struct Credentials {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    pub refresh_token: String,
}
/// A Google "Desktop app" OAuth client. Serialized flat, as the saved
/// `oauth_client` setting; [`Client::to_json`] writes Google's download format.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}
/// What every Google OAuth client ID ends with.
pub const CLIENT_ID_SUFFIX: &str = ".apps.googleusercontent.com";
const AUTH_URI: &str = "https://accounts.google.com/o/oauth2/auth";
/// Why text could not be taken as a client; front ends word it themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientError {
    /// Neither a client JSON nor a client ID and secret.
    Unrecognized,
    /// A "Web application" client, which cannot sign in from a desktop.
    WebClient,
    /// The client ID does not end in [`CLIENT_ID_SUFFIX`].
    ClientId,
    MissingSecret,
}
impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unrecognized => {
                "not an OAuth client: expected the downloaded client JSON, or a client ID and secret"
            }
            Self::WebClient => {
                "download a Desktop app OAuth client JSON (web clients are not supported)"
            }
            Self::ClientId => {
                "invalid Google OAuth client ID: it ends in .apps.googleusercontent.com"
            }
            Self::MissingSecret => {
                "the OAuth client secret is missing (Desktop app clients have one, usually GOCSPX-…)"
            }
        })
    }
}
impl std::error::Error for ClientError {}
/// Pasted text read as a client, or as one of its two parts: Google's
/// console copies the client ID and the secret with separate buttons.
#[derive(Clone, PartialEq, Eq)]
pub enum Clip {
    Client(Client),
    /// A valid client ID by itself.
    Id(String),
    /// A secret in Google's format (`GOCSPX-…`) by itself, or after its label.
    Secret(String),
}
// The secret stays out of logs and panics.
impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Client(client) => f.debug_tuple("Client").field(client).finish(),
            Self::Id(id) => f.debug_tuple("Id").field(id).finish(),
            Self::Secret(_) => f.write_str("Secret(…)"),
        }
    }
}
impl Clip {
    /// Pasted text: what [`Client::from_text`] reads, or a client ID or a
    /// secret alone.
    pub fn from_text(text: &str) -> Result<Self, ClientError> {
        let text = trimmed(text);
        if text.starts_with('{') {
            return Client::from_json(text.as_bytes()).map(Self::Client);
        }
        let tokens: Vec<&str> = text
            .split(|c: char| c.is_whitespace() || c == ',' || c == ':')
            .map(|t| t.trim_matches(|c| c == '"' || c == '\''))
            .filter(|t| !t.is_empty())
            .collect();
        let ids: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| t.ends_with(CLIENT_ID_SUFFIX))
            .collect();
        let others: Vec<&str> = tokens
            .iter()
            .copied()
            .filter(|t| !t.ends_with(CLIENT_ID_SUFFIX))
            .collect();
        let google: Vec<&str> = others
            .iter()
            .copied()
            .filter(|t| t.starts_with("GOCSPX-"))
            .collect();
        // Labelled text, such as "Client secret: GOCSPX-…" copied from the console.
        let labelled = others
            .iter()
            .all(|t| is_label(t) || t.starts_with("GOCSPX-"));
        let secret = match (&others[..], &google[..]) {
            ([secret], _) | (_, [secret]) => Some(*secret),
            _ => None,
        };
        match (&ids[..], secret) {
            ([id], Some(secret)) => {
                let client = Client::new(id, secret);
                client.validate()?;
                Ok(Self::Client(client))
            }
            ([id], None) if others.iter().all(|t| is_label(t)) => {
                Client::new(id, "x").validate()?;
                Ok(Self::Id((*id).to_owned()))
            }
            ([], Some(secret)) if secret.starts_with("GOCSPX-") && labelled => {
                Ok(Self::Secret(secret.to_owned()))
            }
            ([], Some(secret)) if secret.starts_with("GOCSPX-") => Err(ClientError::ClientId),
            _ => Err(ClientError::Unrecognized),
        }
    }
}
/// A word of the labels the console shows beside a client's values.
fn is_label(token: &str) -> bool {
    ["client", "id", "secret", "oauth"]
        .iter()
        .any(|label| token.eq_ignore_ascii_case(label))
}
// The secret stays out of logs and panics.
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("client_id", &self.client_id)
            .field("client_secret", &"…")
            .finish()
    }
}
impl Client {
    pub fn new(client_id: &str, client_secret: &str) -> Self {
        Self {
            client_id: client_id.trim().into(),
            client_secret: client_secret.trim().into(),
        }
    }
    /// A client JSON: Google's download (`{"installed": {…}}`) or a flat
    /// `{"client_id", "client_secret"}`.
    pub fn from_json(data: &[u8]) -> Result<Self, ClientError> {
        let text = std::str::from_utf8(data).map_err(|_| ClientError::Unrecognized)?;
        let value: Value =
            serde_json::from_str(trimmed(text)).map_err(|_| ClientError::Unrecognized)?;
        let fields = if value["installed"].is_object() {
            &value["installed"]
        } else if value["web"].is_object() {
            return Err(ClientError::WebClient);
        } else {
            &value
        };
        let field = |name: &str| fields[name].as_str().unwrap_or_default();
        if field("client_id").is_empty() {
            return Err(ClientError::Unrecognized);
        }
        let client = Self::new(field("client_id"), field("client_secret"));
        client.validate()?;
        Ok(client)
    }
    /// Pasted text: a client JSON, or the client ID and secret in either
    /// order, separated by spaces, new lines, a comma or a colon. Only one
    /// of them is [`ClientError::MissingSecret`] or [`ClientError::ClientId`];
    /// [`Clip::from_text`] takes it as that part.
    pub fn from_text(text: &str) -> Result<Self, ClientError> {
        match Clip::from_text(text)? {
            Clip::Client(client) => Ok(client),
            Clip::Id(_) => Err(ClientError::MissingSecret),
            Clip::Secret(_) => Err(ClientError::ClientId),
        }
    }
    /// Whether this can be a Desktop app client: a Google client ID and a secret.
    pub fn validate(&self) -> Result<(), ClientError> {
        // The project number, a hyphen and a random part, then the suffix.
        let prefix = self.client_id.strip_suffix(CLIENT_ID_SUFFIX);
        if !prefix.is_some_and(|p| {
            p.starts_with(|c: char| c.is_ascii_digit())
                && p.contains('-')
                && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }) {
            return Err(ClientError::ClientId);
        }
        if self.client_secret.is_empty() {
            return Err(ClientError::MissingSecret);
        }
        Ok(())
    }
    /// Google's download format, which gdirect, rclone and Google's own
    /// tools read back.
    pub fn to_json(&self) -> String {
        let value = serde_json::json!({
            "installed": {
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "auth_uri": AUTH_URI,
                "token_uri": TOKEN_URI,
                "redirect_uris": ["http://localhost"],
            }
        });
        serde_json::to_string_pretty(&value).expect("a JSON value serializes")
    }
}
fn trimmed(text: &str) -> &str {
    text.trim_start_matches('\u{feff}').trim()
}
impl Credentials {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let value: Value =
            serde_json::from_slice(data).context("credentials are not valid JSON")?;
        ensure!(
            value["type"] == "authorized_user",
            "expected authorized_user credentials, not a service account"
        );
        let credentials: Self = serde_json::from_value(value)
            .context("credentials are missing client_id or refresh_token")?;
        ensure!(
            !credentials.client_id.is_empty() && !credentials.refresh_token.is_empty(),
            "credentials are missing client_id or refresh_token"
        );
        Ok(credentials)
    }
    pub fn refresh_form(&self) -> [(&'static str, &str); 4] {
        [
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("refresh_token", &self.refresh_token),
            ("grant_type", "refresh_token"),
        ]
    }
    pub fn client(&self) -> Client {
        Client {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
        }
    }
}
pub fn credentials(db: &Store, account: Option<&str>) -> Result<Credentials> {
    if let Some(raw) = account.map(|a| db.credentials(a)).transpose()?.flatten() {
        return Credentials::parse(raw.as_bytes());
    }
    bail!(
        "first login requires a Desktop OAuth client JSON: gdirect login --client-secret PATH\nSetup: {CONFIG_GUIDE}"
    )
}
#[derive(Debug)]
pub struct Token {
    pub access_token: String,
    pub expires_in: i64,
}
fn http() -> Result<reqwest::blocking::Client> {
    Ok(crate::api::client_builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
fn post_token(endpoint: &str, form: &[(&str, &str)]) -> Result<(StatusCode, Value)> {
    let response = http()?.post(endpoint).form(form).send().map_err(|e| {
        anyhow::anyhow!(
            "cannot reach Google's token endpoint ({}); check system HTTP/HTTPS proxy settings or HTTPS_PROXY and access to oauth2.googleapis.com",
            e.without_url()
        )
    })?;
    let status = response.status();
    let text = response.text().map_err(|e| e.without_url())?;
    Ok((status, serde_json::from_str(&text).unwrap_or_default()))
}
// Google's OAuth error code and description, e.g. "invalid_client: The OAuth client was deleted."
fn google_error(status: StatusCode, data: &Value) -> String {
    let code = data["error"].as_str().unwrap_or("unknown_error");
    match data["error_description"].as_str() {
        Some(description) => format!("{status} {code}: {description}"),
        None => format!("{status} {code}"),
    }
}
/// Whether a token-endpoint reply is Google's answer rather than some other page.
pub(crate) fn from_google(data: &Value) -> bool {
    data["access_token"].is_string() || data["error"].is_string()
}
pub(crate) fn refreshed(status: StatusCode, data: &Value) -> Result<Token> {
    if !status.is_success() {
        if data["error"] == "invalid_grant" {
            bail!("refresh credential expired or revoked; run gdirect login --reauth");
        }
        bail!(
            "Google token refresh failed ({}); run gdirect login --reauth",
            google_error(status, data)
        );
    }
    let access_token = data["access_token"]
        .as_str()
        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_graphic()))
        .context("Google returned an invalid access token")?;
    Ok(Token {
        access_token: access_token.into(),
        expires_in: data["expires_in"].as_i64().unwrap_or(3600),
    })
}
pub fn refresh(credentials: &Credentials) -> Result<Token> {
    let (status, data) = post_token(TOKEN_URI, &credentials.refresh_form())?;
    refreshed(status, &data)
}
/// The client to sign in with: the saved one, or else the one the account
/// (or the default account) signed in with.
pub fn saved_client(db: &Store, account: Option<&str>) -> Result<Option<Client>> {
    if let Some(raw) = db.setting("oauth_client")? {
        return Ok(Some(serde_json::from_str(&raw)?));
    }
    let account = match account {
        Some(account) => Some(account.to_owned()),
        None => db.default_account()?,
    };
    Ok(
        match account.map(|a| db.credentials(&a)).transpose()?.flatten() {
            Some(raw) => Some(Credentials::parse(raw.as_bytes())?.client()),
            None => None,
        },
    )
}
pub fn save_client(db: &Store, client: &Client) -> Result<()> {
    client.validate()?;
    db.set("oauth_client", &serde_json::to_string(client)?)
}
pub fn read_client(path: &Path) -> Result<Client> {
    let data = fs::read(path).context("cannot read OAuth client JSON")?;
    Ok(Client::from_json(&data)?)
}
pub fn prepare_login(
    db: &Store,
    account: Option<&str>,
    import: Option<&Path>,
    reauth: bool,
) -> Result<Credentials> {
    if let Some(path) = import {
        return Credentials::parse(&fs::read(path).context("cannot read credentials JSON")?);
    }
    if reauth {
        // Any signed-in account's client works for adding another account.
        let client = saved_client(db, account)?.with_context(|| {
            format!(
                "no saved OAuth client: gdirect login --client-secret PATH\nSetup: {CONFIG_GUIDE}"
            )
        })?;
        return browser_login(&client, &|| false);
    }
    credentials(db, account)
}
fn random_string() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("system random generator unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn callback(target: &str, state: &str) -> Result<Option<String>> {
    ensure!(target.starts_with("/callback?"), "unexpected callback path");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))?;
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    ensure!(
        query.get("state").map(String::as_str) == Some(state),
        "OAuth state mismatch"
    );
    if query.contains_key("error") {
        return Ok(None);
    }
    Ok(Some(
        query
            .get("code")
            .filter(|s| !s.is_empty())
            .context("missing authorization code")?
            .clone(),
    ))
}
/// Opens `url` in the default browser.
pub fn open_in_browser(url: &str) {
    let mut browser = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    let _ = browser
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
/// Signs in in the browser; `cancelled` is polled while waiting for it.
pub fn browser_login(client: &Client, cancelled: &dyn Fn() -> bool) -> Result<Credentials> {
    browser_login_with(client, cancelled, &open_in_browser)
}
/// [`browser_login`], with `open` given the sign-in page's address to open
/// (it may open it again later, while the sign-in waits).
pub fn browser_login_with(
    client: &Client,
    cancelled: &dyn Fn() -> bool,
    open: &dyn Fn(&str),
) -> Result<Credentials> {
    client.validate()?;
    let client_id = client.client_id.as_str();
    let secret = client.client_secret.as_str();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let state = random_string()?;
    let verifier = random_string()?;
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
        ("state", state.as_str()),
        ("code_challenge", challenge(&verifier).as_str()),
        ("code_challenge_method", "S256"),
    ]);
    eprintln!("Open this URL in a browser on this computer:\n{url}");
    open(url.as_str());
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        ensure!(
            Instant::now() < deadline,
            "browser login timed out; run the login command again"
        );
        if cancelled() {
            return Err(LoginCancelled.into());
        }
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut first = String::new();
        if BufReader::new(stream.try_clone()?.take(8192))
            .read_line(&mut first)
            .is_err()
        {
            continue;
        }
        let parts: Vec<_> = first.split_whitespace().collect();
        let result = if parts.first() == Some(&"GET") && parts.len() == 3 {
            callback(parts[1], &state)
        } else {
            Err(anyhow::anyhow!("invalid callback"))
        };
        let (status, body) = if result.is_ok() {
            (
                "200 OK",
                "Authorization received. Return to the terminal to finish login.",
            )
        } else {
            (
                "400 Bad Request",
                "Invalid authorization callback. Return to the login page.",
            )
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        if let Ok(code) = result {
            break code.context("Google authorization was declined")?;
        }
    };
    let (status, token) = post_token(
        TOKEN_URI,
        &[
            ("client_id", client_id),
            ("client_secret", secret),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ],
    )?;
    ensure!(
        status.is_success(),
        "Google rejected the authorization code ({}); verify the Desktop OAuth client configuration",
        google_error(status, &token)
    );
    let refresh_token = token["refresh_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Google did not return a refresh token")?;
    Ok(Credentials {
        client_id: client_id.into(),
        client_secret: secret.into(),
        refresh_token: refresh_token.into(),
    })
}
/// The browser sign-in was cancelled before Google answered.
#[derive(Debug)]
pub struct LoginCancelled;
impl std::fmt::Display for LoginCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("sign-in cancelled")
    }
}
impl std::error::Error for LoginCancelled {}
/// Writes a file only its owner can read: it holds a secret.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(data)?;
    Ok(())
}
pub fn export_credentials(credentials: &Credentials, path: &Path) -> Result<()> {
    write_private(path, serde_json::to_string(credentials)?.as_bytes())
}
pub fn export_client(client: &Client, path: &Path) -> Result<()> {
    write_private(path, format!("{}\n", client.to_json()).as_bytes())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_exchange_uses_proxy() {
        crate::test_network::assert_route("auth::tests::token_exchange_uses_proxy", true, |url| {
            let (status, data) = post_token(url, &[("grant_type", "refresh_token")])?;
            ensure!(status.is_success(), "fixture token request failed");
            Ok(data)
        });
    }
    #[test]
    fn token_responses_keep_google_error_codes() {
        let token = refreshed(
            StatusCode::OK,
            &serde_json::json!({"access_token":"ya29.a","expires_in":3599}),
        )
        .unwrap();
        assert_eq!(
            (token.access_token.as_str(), token.expires_in),
            ("ya29.a", 3599)
        );
        let expired = refreshed(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({"error":"invalid_grant","error_description":"Token has been expired or revoked."}),
        )
        .unwrap_err();
        assert!(
            expired
                .to_string()
                .starts_with("refresh credential expired or revoked")
        );
        let deleted = refreshed(
            StatusCode::UNAUTHORIZED,
            &serde_json::json!({"error":"deleted_client","error_description":"The OAuth client was deleted."}),
        )
        .unwrap_err();
        assert!(
            deleted
                .to_string()
                .contains("401 Unauthorized deleted_client: The OAuth client was deleted.")
        );
        assert!(
            refreshed(StatusCode::BAD_GATEWAY, &Value::Null)
                .unwrap_err()
                .to_string()
                .contains("502 Bad Gateway unknown_error")
        );
        assert!(
            refreshed(
                StatusCode::OK,
                &serde_json::json!({"access_token":"bad token"})
            )
            .is_err()
        );
    }
    #[test]
    fn only_google_token_replies_count() {
        assert!(from_google(&serde_json::json!({"access_token":"ya29.a"})));
        assert!(from_google(&serde_json::json!({"error":"invalid_client"})));
        assert!(!from_google(&serde_json::json!({"error":{"code":404}})));
        assert!(!from_google(&Value::Null));
    }
    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn callback_rejects_wrong_state_and_missing_code() {
        assert!(callback("/callback?state=other&code=secret", "expected").is_err());
        assert!(callback("/callback?state=expected", "expected").is_err());
        assert_eq!(
            callback("/callback?state=expected&code=ok", "expected").unwrap(),
            Some("ok".into())
        );
        assert!(
            callback("/callback?state=expected&error=access_denied", "expected")
                .unwrap()
                .is_none()
        );
    }
    const ID: &str = "123-abc.apps.googleusercontent.com";
    #[test]
    fn browser_login_hands_over_its_page_and_stops_when_cancelled() {
        let client = Client::new(ID, SECRET);
        let opened = std::cell::RefCell::new(Vec::<String>::new());
        let result = browser_login_with(&client, &|| !opened.borrow().is_empty(), &|url| {
            opened.borrow_mut().push(url.to_owned())
        });
        assert!(result.unwrap_err().is::<LoginCancelled>());
        let opened = opened.into_inner();
        assert_eq!(opened.len(), 1);
        let url = url::Url::parse(&opened[0]).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        assert_eq!(query["client_id"], ID);
        assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));
        assert!(!opened[0].contains(SECRET));
    }
    const SECRET: &str = "GOCSPX-test_Secret-1";
    #[test]
    fn client_json_formats() {
        let installed = format!(
            "\u{feff}\n {{\"installed\":{{\"client_id\":\" {ID} \",\"project_id\":\"p\",\"client_secret\":\"{SECRET}\",\"redirect_uris\":[\"http://localhost\"]}}}}\n"
        );
        let flat = format!("{{\"client_id\":\"{ID}\",\"client_secret\":\"{SECRET}\"}}");
        for json in [installed, flat] {
            let client = Client::from_json(json.as_bytes()).unwrap();
            assert_eq!(client, Client::new(ID, SECRET));
        }
        let web = format!("{{\"web\":{{\"client_id\":\"{ID}\",\"client_secret\":\"{SECRET}\"}}}}");
        assert_eq!(
            Client::from_json(web.as_bytes()),
            Err(ClientError::WebClient)
        );
        assert!(
            ClientError::WebClient
                .to_string()
                .contains("Desktop app OAuth client")
        );
        let no_secret = format!("{{\"installed\":{{\"client_id\":\"{ID}\"}}}}");
        assert_eq!(
            Client::from_json(no_secret.as_bytes()),
            Err(ClientError::MissingSecret)
        );
        assert_eq!(
            Client::from_json(br#"{"client_id":"123","client_secret":"s"}"#),
            Err(ClientError::ClientId)
        );
        for bad in [
            &b"not json"[..],
            b"{}",
            b"[]",
            b"{\"type\":\"service_account\"}",
            b"\xff",
        ] {
            assert_eq!(Client::from_json(bad), Err(ClientError::Unrecognized));
        }
    }
    #[test]
    fn client_text_in_either_order() {
        let expected = Client::new(ID, SECRET);
        for text in [
            format!("{ID} {SECRET}"),
            format!("{SECRET}\n{ID}\n"),
            format!("{ID},{SECRET}"),
            format!("{SECRET}:{ID}"),
            format!(" {ID}\r\n{SECRET} "),
            format!("\"{ID}\", \"{SECRET}\""),
            format!("Client ID\n{ID}\nClient secret\n{SECRET}"),
            format!("{{\"client_id\":\"{ID}\",\"client_secret\":\"{SECRET}\"}}"),
        ] {
            assert_eq!(Client::from_text(&text), Ok(expected.clone()), "{text:?}");
        }
        // The secret need not look like Google's current format.
        assert_eq!(
            Client::from_text(&format!("{ID} legacy-secret"))
                .unwrap()
                .client_secret,
            "legacy-secret"
        );
        assert_eq!(Client::from_text(ID), Err(ClientError::MissingSecret));
        assert_eq!(Client::from_text(SECRET), Err(ClientError::ClientId));
        assert_eq!(Client::from_text(""), Err(ClientError::Unrecognized));
        assert_eq!(
            Client::from_text("hello world"),
            Err(ClientError::Unrecognized)
        );
        assert_eq!(
            Client::from_text(&format!("123 {SECRET}")),
            Err(ClientError::ClientId)
        );
        assert_eq!(
            Client::from_text(&format!("{ID} a b")),
            Err(ClientError::Unrecognized)
        );
    }
    #[test]
    fn a_lone_client_id_or_secret_is_that_part() {
        for text in [
            ID.to_owned(),
            format!(" \"{ID}\"\n"),
            format!("Client ID: {ID}"),
        ] {
            assert_eq!(Clip::from_text(&text), Ok(Clip::Id(ID.into())), "{text:?}");
        }
        for text in [
            SECRET.to_owned(),
            format!("'{SECRET}' "),
            format!("Client secret\n{SECRET}"),
        ] {
            assert_eq!(
                Clip::from_text(&text),
                Ok(Clip::Secret(SECRET.into())),
                "{text:?}"
            );
        }
        assert_eq!(
            Clip::from_text(&format!("{SECRET}\n{ID}")),
            Ok(Clip::Client(Client::new(ID, SECRET)))
        );
        // A malformed ID is not taken as one, nor a secret beside a stray word.
        assert_eq!(
            Clip::from_text("abc.apps.googleusercontent.com"),
            Err(ClientError::ClientId)
        );
        assert_eq!(
            Clip::from_text(&format!("123 {SECRET}")),
            Err(ClientError::ClientId)
        );
        assert_eq!(
            Clip::from_text("legacy-secret"),
            Err(ClientError::Unrecognized)
        );
        assert_eq!(
            Clip::from_text(&format!("{ID} a")),
            Ok(Clip::Client(Client::new(ID, "a")))
        );
        assert_eq!(
            Clip::from_text(&format!("{ID} a b")),
            Err(ClientError::Unrecognized)
        );
        assert!(!format!("{:?}", Clip::Secret(SECRET.into())).contains(SECRET));
    }
    #[test]
    fn client_validation() {
        assert!(Client::new(ID, SECRET).validate().is_ok());
        assert_eq!(
            Client::new(".apps.googleusercontent.com", SECRET).validate(),
            Err(ClientError::ClientId)
        );
        assert_eq!(
            Client::new("a b.apps.googleusercontent.com", SECRET).validate(),
            Err(ClientError::ClientId)
        );
        // A secret pasted in front of the ID, its line break dropped.
        assert_eq!(
            Client::new(&format!("{SECRET}{ID}"), SECRET).validate(),
            Err(ClientError::ClientId)
        );
        assert_eq!(
            Client::new(ID, " ").validate(),
            Err(ClientError::MissingSecret)
        );
        assert!(!format!("{:?}", Client::new(ID, SECRET)).contains(SECRET));
    }
    #[test]
    fn client_json_round_trip() {
        let client = Client::new(ID, SECRET);
        let json = client.to_json();
        let value: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["installed"]["redirect_uris"][0], "http://localhost");
        assert_eq!(value["installed"]["token_uri"], TOKEN_URI);
        assert_eq!(Client::from_json(json.as_bytes()), Ok(client.clone()));
        assert_eq!(Client::from_text(&json), Ok(client.clone()));
        // The saved setting stays flat.
        let saved = serde_json::to_string(&client).unwrap();
        assert_eq!(serde_json::from_str::<Client>(&saved).unwrap(), client);
        assert_eq!(Client::from_json(saved.as_bytes()), Ok(client));
    }
    #[test]
    fn imported_credentials_cannot_redirect_tokens() {
        let credentials = Credentials::parse(br#"{"type":"authorized_user","client_id":"test","client_secret":"test","refresh_token":"test","token_uri":"https://evil.test"}"#).unwrap();
        let value = serde_json::to_value(&credentials).unwrap();
        assert!(value.get("token_uri").is_none());
        assert_eq!(value["type"], "authorized_user");
        assert!(
            Credentials::parse(
                br#"{"type":"service_account","client_id":"a","refresh_token":"b"}"#
            )
            .is_err()
        );
        assert!(
            Credentials::parse(br#"{"type":"authorized_user","client_id":"a","refresh_token":""}"#)
                .is_err()
        );
    }
}
