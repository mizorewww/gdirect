use crate::store::Store;
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const FIELDS: &str = "id,name,mimeType,size,modifiedTime,md5Checksum,resourceKey";
pub(crate) const API_HOST: &str = "www.googleapis.com";
pub(crate) const TOKEN_HOST: &str = "oauth2.googleapis.com";
/// TLS identity used when a censor resets the real SNI: connect and verify
/// as g.cn (a Google name this edge serves a matching certificate for) while
/// the HTTP Host header still names the real Google host.
pub(crate) const FORGED_HOST: &str = "g.cn";
/// Google issues every endpoint we touch under its own Google Trust Services
/// roots. Anchoring verification there (instead of a full CA bundle) means a
/// compromised or mis-issuing third-party CA cannot impersonate Google for
/// this client, even with a publicly leaked intermediate key.
pub(crate) const GTS_PEM: &str = include_str!("gts-roots.pem");
pub(crate) fn client_builder() -> reqwest::blocking::ClientBuilder {
    let mut builder = Client::builder().tls_built_in_root_certs(false);
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    for block in GTS_PEM.split(BEGIN).skip(1) {
        let end = block.find(END).expect("bundled GTS root must be complete") + END.len();
        builder = builder.add_root_certificate(
            reqwest::Certificate::from_pem(format!("{BEGIN}{}", &block[..end]).as_bytes())
                .expect("bundled GTS root must parse"),
        );
    }
    builder
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub md5_checksum: Option<String>,
    pub resource_key: Option<String>,
}
impl File {
    pub fn folder(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.folder"
    }
    pub fn workspace(&self) -> bool {
        self.mime_type.starts_with("application/vnd.google-apps.")
    }
    /// Binary files, or Google documents that Drive can export (as PDF in browse).
    pub fn downloadable(&self) -> bool {
        !self.workspace()
            || matches!(
                self.mime_type
                    .trim_start_matches("application/vnd.google-apps."),
                "document" | "spreadsheet" | "presentation" | "drawing"
            )
    }
}
fn client_for(ip: Option<IpAddr>, forged: Option<&str>, host: &str) -> Result<Client> {
    let mut builder = client_builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30));
    if let Some(ip) = ip {
        // Only pinned transfers bypass proxies; login verifies the account without a pin.
        builder = builder
            .no_proxy()
            .resolve(forged.unwrap_or(host), SocketAddr::new(ip, 443));
    }
    Ok(builder.build()?)
}
/// Request URL for `path` (e.g. "drive/v3/about") under the current TLS identity.
pub(crate) fn url_for(forged: Option<&str>, path: &str) -> String {
    format!("https://{}/{path}", forged.unwrap_or(API_HOST))
}
/// Rewrites a validated Google URL for the forged identity: the connection
/// and certificate name become `forged`, only the Host header stays real.
pub(crate) fn gate(forged: Option<&str>, url: &url::Url) -> url::Url {
    let mut url = url.clone();
    if let Some(host) = forged {
        let _ = url.set_host(Some(host));
    }
    url
}
/// A verified endpoint and the TLS identity stored for it (None = real name).
pub type Endpoint = (IpAddr, Option<String>);
struct Live {
    client: Client,
    ip: Option<IpAddr>,
    forged: Option<String>,
    /// Further verified endpoints (with their stored identity), fastest first.
    backups: VecDeque<Endpoint>,
}
#[derive(Clone)]
pub struct Api {
    pub db: PathBuf,
    /// The signed-in account this client acts as; None before the first login.
    pub account: Option<String>,
    /// False once the user picks --sni/--no-sni explicitly: no quiet fallback.
    auto: bool,
    live: Arc<Mutex<Live>>,
    /// Set to stop an upload between chunks; see [`Api::cancellable`].
    cancel: Option<Arc<AtomicBool>>,
}
impl Api {
    pub fn new(
        db: PathBuf,
        account: Option<String>,
        ip: Option<IpAddr>,
        forged: Option<String>,
        auto: bool,
    ) -> Result<Self> {
        ensure!(
            forged.as_deref() != Some(""),
            "--no-sni applies to downloads; Drive API requests need an SNI name"
        );
        Ok(Self {
            live: Arc::new(Mutex::new(Live {
                client: client_for(ip, forged.as_deref(), API_HOST)?,
                ip,
                forged,
                backups: VecDeque::new(),
            })),
            db,
            account,
            auto,
            cancel: None,
        })
    }
    /// A client whose uploads stop, between chunks, once `cancel` is set. The
    /// upload session is kept, so uploading the same file again resumes it.
    pub fn cancellable(&self, cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel: Some(cancel),
            ..self.clone()
        }
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }
    /// Endpoints to move to when the current one stops answering.
    pub fn with_backups(self, backups: Vec<Endpoint>) -> Self {
        self.live.lock().unwrap().backups = backups.into();
        self
    }
    /// Current endpoint, its identity and the remaining backups, for downloads.
    pub fn route(&self) -> (Option<IpAddr>, Option<String>, Vec<Endpoint>) {
        let live = self.live.lock().unwrap();
        (
            live.ip,
            live.forged.clone(),
            live.backups.iter().cloned().collect(),
        )
    }
    /// Sends one request. On a connect/timeout failure it is retried under
    /// the other TLS identity (real <-> forged g.cn), then on each backup
    /// endpoint in turn; dead endpoints are marked in SQLite. Whatever works
    /// is kept for this process and the working identity is persisted.
    pub(crate) fn deliver(
        &self,
        build: impl Fn(&Client, Option<&str>) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response> {
        self.deliver_to(API_HOST, build)
    }
    /// `deliver` for another Google host served by the same endpoints.
    fn deliver_to(
        &self,
        host: &str,
        build: impl Fn(&Client, Option<&str>) -> reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response> {
        let (client, ip, forged) = {
            let live = self.live.lock().unwrap();
            let client = if host == API_HOST {
                live.client.clone()
            } else {
                client_for(live.ip, live.forged.as_deref(), host)?
            };
            (client, live.ip, live.forged.clone())
        };
        let send = |client: &Client, forged: Option<&str>| {
            build(client, forged)
                .header(reqwest::header::HOST, host)
                .send()
                .map_err(|e| e.without_url())
        };
        let transport = |e: &reqwest::Error| e.is_connect() || e.is_timeout();
        let first = match send(&client, forged.as_deref()) {
            Err(error) if ip.is_some() && transport(&error) => error,
            result => return Ok(result?),
        };
        let db = Store::open(&self.db).ok();
        let mut endpoint = ip.map(|ip| (ip, forged.clone()));
        let mut tried_current = true;
        let mut last = first;
        while let Some((ip, preferred)) = endpoint {
            // The identity already tried on the current endpoint is skipped.
            let mut identities = Vec::new();
            if !tried_current {
                identities.push(preferred.clone());
            }
            if self.auto {
                identities.push(match &preferred {
                    Some(_) => None,
                    None => Some(FORGED_HOST.to_owned()),
                });
            }
            for identity in identities {
                let client = client_for(Some(ip), identity.as_deref(), host)?;
                match send(&client, identity.as_deref()) {
                    Ok(response) => {
                        let mut live = self.live.lock().unwrap();
                        live.client = client_for(Some(ip), identity.as_deref(), API_HOST)?;
                        live.ip = Some(ip);
                        live.forged = identity.clone();
                        if let Some(db) = &db {
                            let _ = db
                                .set_sni(&ip.to_string(), identity.as_deref().unwrap_or_default());
                        }
                        return Ok(response);
                    }
                    Err(error) if transport(&error) => last = error,
                    Err(error) => return Err(error.into()),
                }
            }
            if let Some(db) = &db {
                let _ = db.endpoint(&ip.to_string(), None, Some("connection failed"), "");
            }
            tried_current = false;
            endpoint = self
                .live
                .lock()
                .unwrap()
                .backups
                .pop_front()
                .map(|(ip, stored)| {
                    // An explicit --sni applies to every endpoint.
                    (ip, if self.auto { stored } else { forged.clone() })
                });
        }
        Err(last.into())
    }
    pub fn token(&self) -> Result<String> {
        if let Ok(token) = std::env::var("GDIRECT_ACCESS_TOKEN") {
            return validate_token(token);
        }
        let db = Store::open(&self.db)?;
        if let Some(token) = self
            .account
            .as_deref()
            .map(|a| db.token(a))
            .transpose()?
            .flatten()
        {
            return Ok(token);
        }
        self.refresh()
    }
    pub fn download_token(&self) -> Result<String> {
        if let Ok(token) = std::env::var("GDIRECT_ACCESS_TOKEN") {
            return validate_token(token);
        }
        self.refresh()
    }
    fn refresh(&self) -> Result<String> {
        let db = Store::open(&self.db)?;
        // The account is verified at login; refreshing its credentials cannot change it.
        let credentials = crate::auth::credentials(&db, self.account.as_deref())?;
        let token = self.refresh_pinned(&credentials)?;
        if let Some(account) = &self.account {
            db.save_token(account, &token.access_token, token.expires_in)?;
        }
        Ok(token.access_token)
    }
    /// Refreshes over the pinned endpoint so transfers need no proxy for
    /// OAuth either; without one, or when it does not answer as Google's
    /// token endpoint, falls back to the system-proxy route.
    fn refresh_pinned(&self, credentials: &crate::auth::Credentials) -> Result<crate::auth::Token> {
        if self.route().0.is_some() {
            let form = credentials.refresh_form();
            let reply = self
                .deliver_to(TOKEN_HOST, |client, forged| {
                    client
                        .post(format!("https://{}/token", forged.unwrap_or(TOKEN_HOST)))
                        .form(&form)
                })
                .ok()
                .and_then(|response| {
                    let status = response.status();
                    let data = response.json().ok()?;
                    crate::auth::from_google(&data).then_some((status, data))
                });
            if let Some((status, data)) = reply {
                return crate::auth::refreshed(status, &data);
            }
        }
        crate::auth::refresh(credentials)
    }
    fn account_for_token(&self, token: &str) -> Result<String> {
        let response = self
            .deliver(|client, forged| {
                client
                    .get(url_for(forged, "drive/v3/about"))
                    .query(&[("fields", "user(emailAddress)")])
                    .bearer_auth(token)
            })
            .context(
                "cannot verify the account; check system HTTP/HTTPS proxy settings or HTTPS_PROXY and access to www.googleapis.com",
            )?;
        let data = checked(response)?;
        Ok(data["user"]["emailAddress"]
            .as_str()
            .context("Drive API did not return an account")?
            .into())
    }
    pub fn login(
        &self,
        reauth: bool,
        client: Option<&Path>,
        import: Option<&Path>,
    ) -> Result<String> {
        let db = Store::open(&self.db)?;
        let credentials =
            crate::auth::prepare_login(&db, self.account.as_deref(), client, import, reauth)?;
        let token = crate::auth::refresh(&credentials)?;
        // Whichever account was chosen in the browser is added (or updated) and becomes the default.
        let email = self.account_for_token(&token.access_token)?;
        db.save_account(&email, &serde_json::to_string(&credentials)?)?;
        if client.is_some() {
            db.set(
                "oauth_client",
                &serde_json::to_string(&credentials.client())?,
            )?;
        }
        db.save_token(&email, &token.access_token, token.expires_in)?;
        db.set_default_account(&email)?;
        Ok(email)
    }
    fn get(&self, path: &str, query: &[(String, String)]) -> Result<serde_json::Value> {
        self.request(reqwest::Method::GET, path, query, None)
    }
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let build = |client: &Client, forged: Option<&str>, token: &str| {
            let mut request = client
                .request(method.clone(), url_for(forged, &format!("drive/v3/{path}")))
                .query(query)
                .bearer_auth(token);
            if let Some(body) = body {
                request = request.json(body);
            }
            request
        };
        let token = self.token()?;
        let mut response = self.deliver(|client, forged| build(client, forged, &token))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && std::env::var_os("GDIRECT_ACCESS_TOKEN").is_none()
        {
            let token = self.refresh()?;
            response = self.deliver(|client, forged| build(client, forged, &token))?;
        }
        checked(response)
    }
    /// Sends a metadata change and returns the file as Drive now reports it.
    fn write(
        &self,
        method: reqwest::Method,
        path: &str,
        extra: &[(&str, &str)],
        body: serde_json::Value,
    ) -> Result<File> {
        let mut query: Vec<(String, String)> = vec![
            ("fields".into(), FIELDS.into()),
            ("supportsAllDrives".into(), "true".into()),
        ];
        query.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let data = self.request(method, path, &query, Some(&body))?;
        Ok(serde_json::from_value(data)?)
    }
    /// Creates an empty file, or a folder when `folder` is set.
    pub fn create(&self, parent: &str, name: &str, folder: bool) -> Result<File> {
        ensure!(crate::drive::valid_id(parent), "invalid folder ID");
        let mut body = serde_json::json!({"name": checked_name(name)?, "parents": [parent]});
        if folder {
            body["mimeType"] = "application/vnd.google-apps.folder".into();
        }
        self.write(reqwest::Method::POST, "files", &[], body)
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<File> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        let body = serde_json::json!({"name": checked_name(name)?});
        self.write(reqwest::Method::PATCH, &format!("files/{id}"), &[], body)
    }
    /// Moves a file to the Drive trash, where it can still be restored.
    pub fn trash(&self, id: &str) -> Result<File> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        let body = serde_json::json!({"trashed": true});
        self.write(reqwest::Method::PATCH, &format!("files/{id}"), &[], body)
    }
    /// Copies a file (not a folder; Drive cannot copy those) into `parent`, keeping its name.
    pub fn copy(&self, id: &str, parent: &str) -> Result<File> {
        ensure!(
            crate::drive::valid_id(id) && crate::drive::valid_id(parent),
            "invalid file ID"
        );
        let body = serde_json::json!({"parents": [parent]});
        self.write(
            reqwest::Method::POST,
            &format!("files/{id}/copy"),
            &[],
            body,
        )
    }
    /// Moves a file or folder out of all its current folders into `parent`.
    /// Returns false when it is already there.
    pub fn move_to(&self, id: &str, parent: &str) -> Result<bool> {
        ensure!(
            crate::drive::valid_id(id) && crate::drive::valid_id(parent),
            "invalid file ID"
        );
        let data = self.get(
            &format!("files/{id}"),
            &[
                ("fields".into(), "parents".into()),
                ("supportsAllDrives".into(), "true".into()),
            ],
        )?;
        let parents: Vec<String> = serde_json::from_value(data["parents"].clone())?;
        // "root" is an alias; Drive reports the real ID, so compare after resolving it.
        let target = if parent == "root" {
            self.metadata("root")?.id
        } else {
            parent.to_owned()
        };
        if parents == [target.as_str()] {
            return Ok(false);
        }
        let remove = parents
            .iter()
            .filter(|p| **p != target)
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        self.write(
            reqwest::Method::PATCH,
            &format!("files/{id}"),
            &[("addParents", &target), ("removeParents", &remove)],
            serde_json::json!({}),
        )?;
        Ok(true)
    }
    pub fn list(&self, folder: &str) -> Result<Vec<File>> {
        ensure!(crate::drive::valid_id(folder), "invalid folder ID");
        let mut files = Vec::new();
        let mut page = String::new();
        loop {
            let mut query=vec![("q".into(),format!("'{folder}' in parents and trashed = false")),
                ("fields".into(),"nextPageToken,files(id,name,mimeType,size,modifiedTime,md5Checksum,resourceKey)".into()),
                ("pageSize".into(),"1000".into()),("orderBy".into(),"folder,name_natural".into()),
                ("supportsAllDrives".into(),"true".into()),("includeItemsFromAllDrives".into(),"true".into())];
            if !page.is_empty() {
                query.push(("pageToken".into(), page));
            }
            let data = self.get("files", &query)?;
            files.extend(serde_json::from_value::<Vec<File>>(data["files"].clone())?);
            page = data["nextPageToken"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if page.is_empty() {
                break;
            }
        }
        Store::open(&self.db)?.cache(
            self.account.as_deref().unwrap_or_default(),
            folder,
            &serde_json::to_string(&files)?,
        )?;
        Ok(files)
    }
    pub fn metadata(&self, id: &str) -> Result<File> {
        let id = crate::drive::FileRef::parse(id)?.id;
        let data = self.get(
            &format!("files/{id}"),
            &[
                ("fields".into(), FIELDS.into()),
                ("supportsAllDrives".into(), "true".into()),
            ],
        )?;
        Ok(serde_json::from_value(data)?)
    }
    pub fn cached(&self, folder: &str) -> Result<Vec<File>> {
        Store::open(&self.db)?
            .cached(self.account.as_deref().unwrap_or_default(), folder)?
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}
pub fn validate_token(token: String) -> Result<String> {
    ensure!(
        !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()),
        "invalid access token"
    );
    Ok(token)
}
fn checked(response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let body = response.text().map_err(|e| e.without_url())?;
    if status.is_success() {
        return serde_json::from_str(&body).context("Drive API returned invalid JSON");
    }
    anyhow::bail!("{}", drive_error(status, &body))
}
fn drive_error(status: reqwest::StatusCode, body: &str) -> String {
    let data: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let message = data["error"]["message"]
        .as_str()
        .unwrap_or("request failed");
    // Missing scope or a disabled Drive API; per-file permission errors use other reasons.
    let text = data.to_string();
    if status == reqwest::StatusCode::FORBIDDEN
        && [
            "insufficientPermissions",
            "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
            "accessNotConfigured",
            "SERVICE_DISABLED",
        ]
        .iter()
        .any(|reason| text.contains(reason))
    {
        return format!(
            "Drive API {status}: insufficient permission ({message}). Enable the Drive API in the OAuth client's project, then run gdirect login --reauth and allow Drive access"
        );
    }
    format!("Drive API {status}: {message}")
}
fn checked_name(name: &str) -> Result<&str> {
    let name = name.trim();
    ensure!(!name.is_empty(), "name cannot be empty");
    ensure!(
        !name.chars().any(char::is_control),
        "name cannot contain control characters"
    );
    Ok(name)
}
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "download".into()
    } else {
        cleaned
    }
}
pub fn destination(dir: &Path, file: &File) -> PathBuf {
    dir.join(safe_name(&file.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_client_uses_proxy() {
        crate::test_network::assert_route("api::tests::login_client_uses_proxy", true, |url| {
            Ok(client_for(None, None, API_HOST)?
                .get(url)
                .send()?
                .error_for_status()?
                .json()?)
        });
    }
    #[test]
    fn pinned_client_bypasses_proxy() {
        crate::test_network::assert_route(
            "api::tests::pinned_client_bypasses_proxy",
            false,
            |url| {
                Ok(client_for(Some("127.0.0.1".parse()?), None, API_HOST)?
                    .get(url)
                    .send()?
                    .error_for_status()?
                    .json()?)
            },
        );
    }
    #[test]
    fn urls_keep_drive_and_upload_paths() {
        assert_eq!(
            url_for(None, "drive/v3/about"),
            "https://www.googleapis.com/drive/v3/about"
        );
        assert_eq!(
            url_for(None, "upload/drive/v3/files"),
            "https://www.googleapis.com/upload/drive/v3/files"
        );
        assert_eq!(
            url_for(Some(FORGED_HOST), "upload/drive/v3/files"),
            "https://g.cn/upload/drive/v3/files"
        );
    }
    #[test]
    fn drive_errors_keep_status_and_hint_only_for_scope() {
        use reqwest::StatusCode;
        assert_eq!(
            drive_error(StatusCode::BAD_GATEWAY, "<html>bad gateway</html>"),
            "Drive API 502 Bad Gateway: request failed"
        );
        assert_eq!(
            drive_error(
                StatusCode::NOT_FOUND,
                r#"{"error":{"message":"File not found: x."}}"#
            ),
            "Drive API 404 Not Found: File not found: x."
        );
        let scope = drive_error(
            StatusCode::FORBIDDEN,
            r#"{"error":{"message":"Request had insufficient authentication scopes.","errors":[{"reason":"insufficientPermissions"}]}}"#,
        );
        assert!(scope.starts_with("Drive API 403 Forbidden: insufficient permission"));
        assert!(scope.contains("gdirect login --reauth"));
        let file = drive_error(
            StatusCode::FORBIDDEN,
            r#"{"error":{"message":"No access.","errors":[{"reason":"insufficientFilePermissions"}]}}"#,
        );
        assert_eq!(file, "Drive API 403 Forbidden: No access.");
    }
    #[test]
    fn only_binary_and_exportable_items_download() {
        let file = |mime: &str| File {
            id: "x".into(),
            name: "x".into(),
            mime_type: mime.into(),
            size: None,
            modified_time: None,
            md5_checksum: None,
            resource_key: None,
        };
        assert!(file("application/zip").downloadable());
        assert!(file("application/vnd.google-apps.document").downloadable());
        assert!(!file("application/vnd.google-apps.shortcut").downloadable());
        assert!(!file("application/vnd.google-apps.form").downloadable());
    }
    #[test]
    fn names_cannot_escape() {
        assert_eq!(safe_name("../a\n"), ".._a_");
        assert_eq!(safe_name(".."), "download");
    }
}
