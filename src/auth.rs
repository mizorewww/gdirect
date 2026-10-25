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
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
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
    pub fn client(&self) -> Client {
        Client {
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
        }
    }
}
pub fn credentials(db: &Store) -> Result<Credentials> {
    if let Some(raw) = db.setting("oauth_credentials")? {
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
    Ok(reqwest::blocking::Client::builder()
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
fn refreshed(status: StatusCode, data: &Value) -> Result<Token> {
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
    let (status, data) = post_token(
        TOKEN_URI,
        &[
            ("client_id", &credentials.client_id),
            ("client_secret", &credentials.client_secret),
            ("refresh_token", &credentials.refresh_token),
            ("grant_type", "refresh_token"),
        ],
    )?;
    refreshed(status, &data)
}
pub fn prepare_login(
    db: &Store,
    client: Option<&Path>,
    import: Option<&Path>,
    reauth: bool,
) -> Result<Credentials> {
    if let Some(path) = import {
        return Credentials::parse(&fs::read(path).context("cannot read credentials JSON")?);
    }
    if let Some(path) = client {
        let data: Value =
            serde_json::from_slice(&fs::read(path).context("cannot read OAuth client JSON")?)?;
        ensure!(
            data["installed"].is_object(),
            "download a Desktop app OAuth client JSON (web clients are not supported)"
        );
        let client = serde_json::from_value(data["installed"].clone())
            .context("OAuth client JSON is missing client_id")?;
        return browser_login(&client);
    }
    if reauth {
        let client = match db.setting("oauth_client")? {
            Some(raw) => serde_json::from_str(&raw)?,
            None => credentials(db)?.client(),
        };
        return browser_login(&client);
    }
    credentials(db)
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
fn browser_login(client: &Client) -> Result<Credentials> {
    let client_id = client.client_id.as_str();
    ensure!(
        client_id.ends_with(".apps.googleusercontent.com"),
        "invalid Google OAuth client ID"
    );
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
        .arg(url.as_str())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    let deadline = Instant::now() + Duration::from_secs(300);
    let code = loop {
        ensure!(
            Instant::now() < deadline,
            "browser login timed out; run the login command again"
        );
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
pub fn export_credentials(credentials: &Credentials, path: &Path) -> Result<()> {
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
    file.write_all(serde_json::to_string(credentials)?.as_bytes())?;
    Ok(())
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
