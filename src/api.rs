use crate::store::Store;
use anyhow::{Context, Result, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// File fields requested everywhere a [`File`] is read, so every path returns
/// the same shape.
pub const FIELDS: &str = "id,name,mimeType,size,modifiedTime,md5Checksum,resourceKey,\
parents,starred,trashed,explicitlyTrashed,trashedTime,driveId,webViewLink,createdTime,\
viewedByMeTime,sharedWithMeTime,shared,owners(displayName,emailAddress,me),\
sharingUser(displayName,emailAddress,me),lastModifyingUser(displayName,emailAddress,me),\
shortcutDetails(targetId,targetMimeType),capabilities(canAddChildren,canRename,canTrash,\
canDelete,canUntrash,canDownload,canShare,canCopy)";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
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
/// A Drive item. Everything after `resource_key` defaults, so JSON cached
/// before those fields existed still reads.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub md5_checksum: Option<String>,
    pub resource_key: Option<String>,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub explicitly_trashed: bool,
    #[serde(default)]
    pub trashed_time: Option<String>,
    #[serde(default)]
    pub drive_id: Option<String>,
    #[serde(default)]
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub created_time: Option<String>,
    #[serde(default)]
    pub viewed_by_me_time: Option<String>,
    #[serde(default)]
    pub shared_with_me_time: Option<String>,
    #[serde(default)]
    pub shared: bool,
    #[serde(default)]
    pub owners: Vec<User>,
    #[serde(default)]
    pub sharing_user: Option<User>,
    #[serde(default)]
    pub last_modifying_user: Option<User>,
    #[serde(default)]
    pub shortcut_details: Option<ShortcutDetails>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub display_name: Option<String>,
    pub email_address: Option<String>,
    pub me: bool,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutDetails {
    pub target_id: String,
    pub target_mime_type: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub can_add_children: bool,
    pub can_rename: bool,
    pub can_trash: bool,
    pub can_delete: bool,
    pub can_untrash: bool,
    pub can_download: bool,
    pub can_share: bool,
    pub can_copy: bool,
}
/// Which collection a [`Query`] searches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Corpora {
    /// My Drive and items shared with the user.
    User,
    /// One Shared drive, by ID.
    Drive(String),
    /// Everything the user can reach, Shared drives included.
    AllDrives,
}
/// A `files.list` request; see the constructors for the common views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub q: String,
    pub corpora: Corpora,
    pub order_by: Option<String>,
    pub page_size: u32,
}
/// One page of [`Api::query`] results.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub files: Vec<File>,
    /// Token for the next page; None on the last one.
    pub next: Option<String>,
    /// Drive did not search every corpus (only with [`Corpora::AllDrives`]).
    pub incomplete: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    Folder,
    Document,
    Spreadsheet,
    Presentation,
    Pdf,
    Image,
    Video,
    Audio,
    Archive,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Owner {
    #[default]
    Any,
    Me,
    NotMe,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchSpec {
    pub text: String,
    /// Search contents too (`fullText contains`), not just names.
    pub full_text: bool,
    pub kind: Option<SearchKind>,
    pub owner: Owner,
    /// RFC 3339 time, e.g. "2026-01-01T00:00:00".
    pub modified_after: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedDrive {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quota {
    /// None for unlimited storage.
    pub limit: Option<u64>,
    pub usage: u64,
    pub usage_in_drive: u64,
    pub usage_in_trash: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct About {
    pub email: String,
    pub storage: Quota,
}
/// Escapes `s` for use inside a single-quoted Drive query literal.
pub fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}
impl SearchKind {
    fn clause(self) -> &'static str {
        match self {
            Self::Folder => "mimeType = 'application/vnd.google-apps.folder'",
            Self::Document => {
                "(mimeType = 'application/vnd.google-apps.document' or mimeType = 'application/msword' or mimeType = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document')"
            }
            Self::Spreadsheet => {
                "(mimeType = 'application/vnd.google-apps.spreadsheet' or mimeType = 'application/vnd.ms-excel' or mimeType = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')"
            }
            Self::Presentation => {
                "(mimeType = 'application/vnd.google-apps.presentation' or mimeType = 'application/vnd.ms-powerpoint' or mimeType = 'application/vnd.openxmlformats-officedocument.presentationml.presentation')"
            }
            Self::Pdf => "mimeType = 'application/pdf'",
            Self::Image => "mimeType contains 'image/'",
            Self::Video => "mimeType contains 'video/'",
            Self::Audio => "mimeType contains 'audio/'",
            Self::Archive => {
                "(mimeType = 'application/zip' or mimeType = 'application/x-zip-compressed' or mimeType = 'application/x-7z-compressed' or mimeType = 'application/x-rar-compressed' or mimeType = 'application/vnd.rar' or mimeType = 'application/x-tar' or mimeType = 'application/gzip')"
            }
        }
    }
}
impl Query {
    fn new(q: String, corpora: Corpora, order_by: Option<&str>, page_size: u32) -> Self {
        Self {
            q,
            corpora,
            order_by: order_by.map(Into::into),
            page_size,
        }
    }
    /// The live children of a folder, folders first; `drive_id` names the
    /// Shared drive it lives in, if any.
    pub fn folder(id: &str, drive_id: Option<&str>) -> Self {
        Self::new(
            format!("'{}' in parents and trashed = false", quote(id)),
            drive_id.map_or(Corpora::User, |d| Corpora::Drive(d.into())),
            Some("folder,name_natural"),
            1000,
        )
    }
    pub fn shared_with_me() -> Self {
        Self::new(
            "sharedWithMe = true and trashed = false".into(),
            Corpora::User,
            Some("sharedWithMeTime desc"),
            1000,
        )
    }
    pub fn starred() -> Self {
        Self::new(
            "starred = true and trashed = false".into(),
            Corpora::User,
            Some("folder,name_natural"),
            1000,
        )
    }
    /// Recently used files (no folders), newest first.
    pub fn recent() -> Self {
        Self::new(
            format!("trashed = false and mimeType != '{FOLDER_MIME}'"),
            Corpora::User,
            Some("recency desc"),
            100,
        )
    }
    /// The trash, latest first. Drive rejects `trashedTime` as a sort key;
    /// recency is the closest it accepts.
    pub fn trash() -> Self {
        Self::new(
            "trashed = true".into(),
            Corpora::User,
            Some("recency desc"),
            1000,
        )
    }
    /// A search across every drive. Full-text results come in relevance
    /// order (Drive refuses `orderBy` there); name searches by name.
    pub fn search(spec: &SearchSpec) -> Self {
        let mut clauses = Vec::new();
        let text = spec.text.trim();
        if !text.is_empty() {
            let field = if spec.full_text { "fullText" } else { "name" };
            clauses.push(format!("{field} contains '{}'", quote(text)));
        }
        clauses.push("trashed = false".into());
        if let Some(kind) = spec.kind {
            clauses.push(kind.clause().into());
        }
        match spec.owner {
            Owner::Any => {}
            Owner::Me => clauses.push("'me' in owners".into()),
            Owner::NotMe => clauses.push("not 'me' in owners".into()),
        }
        if let Some(after) = &spec.modified_after {
            clauses.push(format!("modifiedTime > '{}'", quote(after)));
        }
        Self::new(
            clauses.join(" and "),
            Corpora::AllDrives,
            (!spec.full_text).then_some("folder,name_natural"),
            1000,
        )
    }
    /// The `files.list` query string for one page.
    fn params(&self, page_token: Option<&str>) -> Vec<(String, String)> {
        let mut query: Vec<(String, String)> = vec![
            ("q".into(), self.q.clone()),
            (
                "fields".into(),
                format!("nextPageToken,incompleteSearch,files({FIELDS})"),
            ),
            ("pageSize".into(), self.page_size.clamp(1, 1000).to_string()),
            ("supportsAllDrives".into(), "true".into()),
            ("includeItemsFromAllDrives".into(), "true".into()),
        ];
        match &self.corpora {
            Corpora::User => query.push(("corpora".into(), "user".into())),
            Corpora::Drive(id) => {
                query.push(("corpora".into(), "drive".into()));
                query.push(("driveId".into(), id.clone()));
            }
            Corpora::AllDrives => query.push(("corpora".into(), "allDrives".into())),
        }
        if let Some(order) = &self.order_by {
            query.push(("orderBy".into(), order.clone()));
        }
        if let Some(token) = page_token.filter(|t| !t.is_empty()) {
            query.push(("pageToken".into(), token.into()));
        }
        query
    }
}
/// Reads `about` (fields `user(emailAddress),storageQuota`); Drive sends the
/// byte counts as strings.
fn parse_about(data: &serde_json::Value) -> Result<About> {
    let number = |v: &serde_json::Value| -> Option<u64> {
        v.as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| v.as_u64())
    };
    let quota = &data["storageQuota"];
    Ok(About {
        email: data["user"]["emailAddress"]
            .as_str()
            .context("Drive API did not return an account")?
            .into(),
        storage: Quota {
            limit: number(&quota["limit"]),
            usage: number(&quota["usage"]).unwrap_or_default(),
            usage_in_drive: number(&quota["usageInDrive"]).unwrap_or_default(),
            usage_in_trash: number(&quota["usageInDriveTrash"]).unwrap_or_default(),
        },
    })
}
impl File {
    pub fn folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
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
        // Only pinned transfers bypass proxies, TUN ones included (bound to
        // the physical interface); login verifies the account without a pin.
        builder = crate::netif::client(builder, ip)
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
/// Item ID -> (name, first parent).
type Tree = HashMap<String, (String, Option<String>)>;
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
    /// Folder ID -> (name, parent) learned by [`Api::ancestors`].
    tree: Arc<Mutex<Tree>>,
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
            tree: Arc::default(),
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
        // The network may have changed under us: look for the adapter again.
        crate::netif::forget();
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
    /// Signs in with a client JSON file, the saved client (`reauth`), or
    /// imported credentials; without any, checks the saved login.
    pub fn login(
        &self,
        reauth: bool,
        client: Option<&Path>,
        import: Option<&Path>,
    ) -> Result<String> {
        if let Some(path) = client {
            return self.login_with_client(&crate::auth::read_client(path)?);
        }
        let db = Store::open(&self.db)?;
        let credentials = crate::auth::prepare_login(&db, self.account.as_deref(), import, reauth)?;
        self.finish_login(&db, &credentials)
    }
    /// Signs in in the browser with `client` and saves it for later sign-ins.
    /// A [`cancellable`](Self::cancellable) client stops waiting once cancelled.
    pub fn login_with_client(&self, client: &crate::auth::Client) -> Result<String> {
        self.login_with_client_opening(client, &crate::auth::open_in_browser)
    }
    /// [`login_with_client`](Self::login_with_client), with `open` given the
    /// sign-in page's address to open.
    pub fn login_with_client_opening(
        &self,
        client: &crate::auth::Client,
        open: &dyn Fn(&str),
    ) -> Result<String> {
        let credentials = crate::auth::browser_login_with(client, &|| self.cancelled(), open)?;
        let db = Store::open(&self.db)?;
        let email = self.finish_login(&db, &credentials)?;
        self.save_client(client)?;
        Ok(email)
    }
    fn finish_login(&self, db: &Store, credentials: &crate::auth::Credentials) -> Result<String> {
        let token = crate::auth::refresh(credentials)?;
        // Whichever account was chosen in the browser is added (or updated) and becomes the default.
        let email = self.account_for_token(&token.access_token)?;
        // A sign-in cancelled once Google had answered saves nothing.
        if self.cancelled() {
            return Err(crate::auth::LoginCancelled.into());
        }
        db.save_account(&email, &serde_json::to_string(credentials)?)?;
        db.save_token(&email, &token.access_token, token.expires_in)?;
        db.set_default_account(&email)?;
        Ok(email)
    }
    /// The client the next sign-in uses; see [`crate::auth::saved_client`].
    pub fn saved_client(&self) -> Result<Option<crate::auth::Client>> {
        crate::auth::saved_client(&Store::open(&self.db)?, self.account.as_deref())
    }
    pub fn save_client(&self, client: &crate::auth::Client) -> Result<()> {
        crate::auth::save_client(&Store::open(&self.db)?, client)
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
        Ok(self.move_to_returning(id, parent)?.is_some())
    }
    /// [`Api::move_to`] that returns the folders the item was in, so the move
    /// can be undone; None when it is already in `parent` alone.
    pub fn move_to_returning(&self, id: &str, parent: &str) -> Result<Option<Vec<String>>> {
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
        let parents: Vec<String> =
            serde_json::from_value(data["parents"].clone()).unwrap_or_default();
        // "root" is an alias; Drive reports the real ID, so compare after resolving it.
        let target = if parent == "root" {
            self.metadata("root")?.id
        } else {
            parent.to_owned()
        };
        if parents == [target.as_str()] {
            return Ok(None);
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
        Ok(Some(parents))
    }
    /// Restores a trashed file or folder.
    pub fn untrash(&self, id: &str) -> Result<File> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        let body = serde_json::json!({"trashed": false});
        self.write(reqwest::Method::PATCH, &format!("files/{id}"), &[], body)
    }
    /// Deletes a file or folder for good, skipping the trash.
    pub fn delete(&self, id: &str) -> Result<()> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        self.request(
            reqwest::Method::DELETE,
            &format!("files/{id}"),
            &[("supportsAllDrives".into(), "true".into())],
            None,
        )?;
        Ok(())
    }
    /// Permanently deletes everything in the user's trash.
    pub fn empty_trash(&self) -> Result<()> {
        self.request(reqwest::Method::DELETE, "files/trash", &[], None)?;
        Ok(())
    }
    pub fn set_starred(&self, id: &str, starred: bool) -> Result<File> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        let body = serde_json::json!({"starred": starred});
        self.write(reqwest::Method::PATCH, &format!("files/{id}"), &[], body)
    }
    /// Creates a shortcut named `name` in `parent` pointing at `target_id`.
    pub fn create_shortcut(&self, parent: &str, target_id: &str, name: &str) -> Result<File> {
        ensure!(
            crate::drive::valid_id(parent) && crate::drive::valid_id(target_id),
            "invalid file ID"
        );
        let body = serde_json::json!({
            "name": checked_name(name)?,
            "mimeType": "application/vnd.google-apps.shortcut",
            "parents": [parent],
            "shortcutDetails": {"targetId": target_id},
        });
        self.write(reqwest::Method::POST, "files", &[], body)
    }
    /// Every Shared drive the user can see.
    pub fn drives(&self) -> Result<Vec<SharedDrive>> {
        #[derive(Deserialize)]
        struct Raw {
            id: String,
            #[serde(default)]
            name: String,
        }
        let mut drives = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut query: Vec<(String, String)> = vec![
                ("pageSize".into(), "100".into()),
                ("fields".into(), "nextPageToken,drives(id,name)".into()),
            ];
            if let Some(token) = page.take() {
                query.push(("pageToken".into(), token));
            }
            let data = self.get("drives", &query)?;
            let raw: Vec<Raw> = serde_json::from_value(data["drives"].clone()).unwrap_or_default();
            drives.extend(raw.into_iter().map(|d| SharedDrive {
                id: d.id,
                name: d.name,
            }));
            page = data["nextPageToken"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(Into::into);
            if page.is_none() {
                return Ok(drives);
            }
        }
    }
    /// The signed-in account and its storage use.
    pub fn about(&self) -> Result<About> {
        let data = self.get(
            "about",
            &[("fields".into(), "user(emailAddress),storageQuota".into())],
        )?;
        parse_about(&data)
    }
    /// The folders above `id`, from the top (My Drive or a Shared drive)
    /// down to its parent, as (ID, name). Folders seen once are remembered
    /// for the life of this client.
    pub fn ancestors(&self, id: &str) -> Result<Vec<(String, String)>> {
        ensure!(crate::drive::valid_id(id), "invalid file ID");
        let mut chain = Vec::new();
        let mut next = self.node(id)?.1;
        // Drive has no cycles, but a bound keeps a bad reply from looping.
        while let Some(parent) = next.take() {
            ensure!(chain.len() < 256, "folder chain too deep");
            let (name, up) = self.node(&parent)?;
            chain.push((parent, name));
            next = up;
        }
        chain.reverse();
        Ok(chain)
    }
    /// (name, first parent) of one item, from the cache or Drive.
    fn node(&self, id: &str) -> Result<(String, Option<String>)> {
        if let Some(hit) = self.tree.lock().unwrap().get(id) {
            return Ok(hit.clone());
        }
        let data = self.get(
            &format!("files/{id}"),
            &[
                ("fields".into(), "id,name,parents,driveId".into()),
                ("supportsAllDrives".into(), "true".into()),
            ],
        )?;
        let parent = data["parents"][0].as_str().map(String::from);
        let mut name = data["name"].as_str().unwrap_or_default().to_owned();
        // A Shared drive's top folder reports a generic name; ask the drive.
        if parent.is_none()
            && data["driveId"].as_str() == Some(id)
            && let Ok(drive) =
                self.get(&format!("drives/{id}"), &[("fields".into(), "name".into())])
            && let Some(real) = drive["name"].as_str()
        {
            name = real.into();
        }
        let node = (name, parent);
        self.tree
            .lock()
            .unwrap()
            .insert(id.to_owned(), node.clone());
        if let Some(real) = data["id"].as_str().filter(|real| *real != id) {
            // "root" resolves to the real ID; remember both.
            self.tree
                .lock()
                .unwrap()
                .insert(real.to_owned(), node.clone());
        }
        Ok(node)
    }
    /// One page of a `files.list` query.
    pub fn query(&self, q: &Query, page_token: Option<&str>) -> Result<Page> {
        let data = self.get("files", &q.params(page_token))?;
        Ok(Page {
            files: serde_json::from_value(data["files"].clone())?,
            next: data["nextPageToken"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(Into::into),
            incomplete: data["incompleteSearch"].as_bool().unwrap_or(false),
        })
    }
    /// Every page of a query.
    pub fn query_all(&self, q: &Query) -> Result<Vec<File>> {
        let mut files = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.query(q, token.as_deref())?;
            files.extend(page.files);
            token = page.next;
            if token.is_none() {
                return Ok(files);
            }
        }
    }
    /// All live children of a folder, also stored as the folder's cached listing.
    pub fn list(&self, folder: &str) -> Result<Vec<File>> {
        ensure!(crate::drive::valid_id(folder), "invalid folder ID");
        let files = self.query_all(&Query::folder(folder, None))?;
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
        // 204 No Content (deletes) carries no body at all.
        if body.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
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
            ..File::default()
        };
        assert!(file("application/zip").downloadable());
        assert!(file("application/vnd.google-apps.document").downloadable());
        assert!(!file("application/vnd.google-apps.shortcut").downloadable());
        assert!(!file("application/vnd.google-apps.form").downloadable());
    }
    #[test]
    fn quotes_drive_literals() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("it's"), r"it\'s");
        assert_eq!(quote(r"a\b"), r"a\\b");
        assert_eq!(quote(r"\'"), r"\\\'");
    }
    #[test]
    fn query_constructors() {
        let q = Query::folder("abc", None);
        assert_eq!(q.q, "'abc' in parents and trashed = false");
        assert_eq!(q.order_by.as_deref(), Some("folder,name_natural"));
        assert_eq!(q.corpora, Corpora::User);
        assert_eq!(
            Query::folder("abc", Some("d1")).corpora,
            Corpora::Drive("d1".into())
        );
        let q = Query::shared_with_me();
        assert_eq!(q.q, "sharedWithMe = true and trashed = false");
        assert_eq!(q.order_by.as_deref(), Some("sharedWithMeTime desc"));
        let q = Query::starred();
        assert_eq!(q.q, "starred = true and trashed = false");
        assert_eq!(q.order_by.as_deref(), Some("folder,name_natural"));
        let q = Query::recent();
        assert_eq!(
            q.q,
            "trashed = false and mimeType != 'application/vnd.google-apps.folder'"
        );
        assert_eq!(q.order_by.as_deref(), Some("recency desc"));
        assert_eq!(q.page_size, 100);
        let q = Query::trash();
        assert_eq!(q.q, "trashed = true");
        // Drive answers 400 Invalid Value to orderBy=trashedTime.
        assert_eq!(q.order_by.as_deref(), Some("recency desc"));
    }
    #[test]
    fn query_params_name_corpora_and_shared_drives() {
        let get = |params: &[(String, String)], key: &str| {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        let params = Query::folder("abc", Some("d1")).params(Some("tok"));
        assert_eq!(get(&params, "corpora").as_deref(), Some("drive"));
        assert_eq!(get(&params, "driveId").as_deref(), Some("d1"));
        assert_eq!(get(&params, "pageToken").as_deref(), Some("tok"));
        assert_eq!(get(&params, "supportsAllDrives").as_deref(), Some("true"));
        assert_eq!(
            get(&params, "includeItemsFromAllDrives").as_deref(),
            Some("true")
        );
        assert!(get(&params, "fields").unwrap().contains(FIELDS));
        let params = Query::search(&SearchSpec {
            text: "x".into(),
            full_text: true,
            ..Default::default()
        })
        .params(None);
        assert_eq!(get(&params, "corpora").as_deref(), Some("allDrives"));
        assert_eq!(get(&params, "orderBy"), None);
        assert_eq!(get(&params, "pageToken"), None);
        assert_eq!(get(&params, "driveId"), None);
    }
    #[test]
    fn search_specs() {
        let q = Query::search(&SearchSpec {
            text: "report's".into(),
            ..Default::default()
        });
        assert_eq!(q.q, r"name contains 'report\'s' and trashed = false");
        assert_eq!(q.order_by.as_deref(), Some("folder,name_natural"));
        assert_eq!(q.corpora, Corpora::AllDrives);
        let q = Query::search(&SearchSpec {
            text: "budget".into(),
            full_text: true,
            kind: Some(SearchKind::Pdf),
            owner: Owner::Me,
            modified_after: Some("2026-01-01T00:00:00".into()),
        });
        assert_eq!(
            q.q,
            "fullText contains 'budget' and trashed = false and mimeType = 'application/pdf' \
             and 'me' in owners and modifiedTime > '2026-01-01T00:00:00'"
        );
        assert_eq!(q.order_by, None);
        let q = Query::search(&SearchSpec {
            kind: Some(SearchKind::Image),
            owner: Owner::NotMe,
            ..Default::default()
        });
        assert_eq!(
            q.q,
            "trashed = false and mimeType contains 'image/' and not 'me' in owners"
        );
        let q = Query::search(&SearchSpec {
            text: "x".into(),
            kind: Some(SearchKind::Folder),
            ..Default::default()
        });
        assert!(q.q.ends_with("mimeType = 'application/vnd.google-apps.folder'"));
        let q = Query::search(&SearchSpec {
            kind: Some(SearchKind::Spreadsheet),
            ..Default::default()
        });
        assert!(q.q.contains("application/vnd.google-apps.spreadsheet"));
    }
    #[test]
    fn files_read_old_and_full_json() {
        let old: File = serde_json::from_str(
            r#"{"id":"a","name":"n","mimeType":"text/plain","size":"3","modifiedTime":null,"md5Checksum":null,"resourceKey":null}"#,
        )
        .unwrap();
        assert_eq!(old.size.as_deref(), Some("3"));
        assert!(old.parents.is_empty() && !old.starred && !old.trashed);
        assert!(old.owners.is_empty() && old.capabilities.is_none());
        let cached: Vec<File> =
            serde_json::from_str(r#"[{"id":"a","name":"n","mimeType":"x"}]"#).unwrap();
        assert_eq!(cached[0].id, "a");
        let full: File = serde_json::from_str(
            r#"{"id":"a","name":"n","mimeType":"application/vnd.google-apps.shortcut",
            "parents":["p"],"starred":true,"trashed":true,"explicitlyTrashed":true,
            "trashedTime":"2026-01-02T00:00:00Z","driveId":"d","webViewLink":"https://x",
            "createdTime":"c","viewedByMeTime":"v","sharedWithMeTime":"s","shared":true,
            "owners":[{"displayName":"Me","emailAddress":"me@x","me":true}],
            "sharingUser":{"displayName":"Bo"},"lastModifyingUser":{"emailAddress":"l@x"},
            "shortcutDetails":{"targetId":"t","targetMimeType":"application/pdf"},
            "capabilities":{"canAddChildren":true,"canRename":true,"canTrash":true,
            "canDelete":false,"canUntrash":true,"canDownload":true,"canShare":true,"canCopy":true}}"#,
        )
        .unwrap();
        assert_eq!(full.parents, ["p"]);
        assert!(full.starred && full.trashed && full.explicitly_trashed && full.shared);
        assert_eq!(full.drive_id.as_deref(), Some("d"));
        assert!(full.owners[0].me);
        assert_eq!(
            full.sharing_user.unwrap().display_name.as_deref(),
            Some("Bo")
        );
        assert_eq!(full.shortcut_details.unwrap().target_id, "t");
        let caps = full.capabilities.unwrap();
        assert!(caps.can_rename && caps.can_untrash && !caps.can_delete);
        // A round trip keeps the new fields for the folder cache.
        let again: File = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(again.id, "a");
    }
    #[test]
    fn about_reads_string_numbers() {
        let about = parse_about(&serde_json::json!({
            "user": {"emailAddress": "a@x"},
            "storageQuota": {"limit": "16106127360", "usage": "1024",
                "usageInDrive": "512", "usageInDriveTrash": "2"}
        }))
        .unwrap();
        assert_eq!(about.email, "a@x");
        assert_eq!(
            about.storage,
            Quota {
                limit: Some(16106127360),
                usage: 1024,
                usage_in_drive: 512,
                usage_in_trash: 2,
            }
        );
        let unlimited = parse_about(&serde_json::json!({
            "user": {"emailAddress": "a@x"}, "storageQuota": {"usage": "5"}
        }))
        .unwrap();
        assert_eq!(unlimited.storage.limit, None);
        assert_eq!(unlimited.storage.usage, 5);
        assert!(parse_about(&serde_json::json!({})).is_err());
    }
    #[test]
    fn names_cannot_escape() {
        assert_eq!(safe_name("../a\n"), ".._a_");
        assert_eq!(safe_name(".."), "download");
    }
}
