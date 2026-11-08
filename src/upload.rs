use crate::{
    api::{Api, File},
    store::Store,
};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, blocking::Response};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

const CHUNK: u64 = 32 * 1024 * 1024;
const FIELDS: &str = "id,name,mimeType,size,md5Checksum";
#[derive(Serialize, Deserialize)]
struct Session {
    size: u64,
    modified: u128,
    url: String,
}

// Session URLs are credentials: keep them in the private SQLite database and
// never print them or follow a redirect with the user's bearer token.
fn session_url(raw: &str) -> Result<url::Url> {
    let url = url::Url::parse(raw).context("invalid upload session URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("www.googleapis.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path().starts_with("/upload/drive/v3/files"),
        "untrusted upload session URL"
    );
    Ok(url)
}
fn offset(response: &Response, size: u64) -> Result<u64> {
    let Some(range) = response.headers().get("range") else {
        return Ok(0);
    };
    acknowledged(range.to_str()?, size)
}
fn acknowledged(range: &str, size: u64) -> Result<u64> {
    let end: u64 = range
        .strip_prefix("bytes=0-")
        .context("invalid upload acknowledgement")?
        .parse()?;
    let next = end.checked_add(1).context("invalid upload offset")?;
    ensure!(next <= size, "upload acknowledgement exceeds file size");
    Ok(next)
}
fn failure(response: Response) -> anyhow::Error {
    // Do not include a response body: it can contain the secret session URI.
    anyhow::anyhow!(
        "Drive upload returned {}; rerun the same upload to resume",
        response.status()
    )
}
impl Api {
    /// Uploads one file, printing byte progress to stderr.
    pub fn upload(&self, path: &Path, parent: &str, name: Option<&str>) -> Result<File> {
        self.upload_with(path, parent, name, |done, total| {
            if done < total {
                eprint!("\rUploading {done}/{total} bytes");
            } else {
                eprintln!("\rUploaded {total}/{total} bytes");
            }
        })
    }
    /// Asks Drive how much of a resumable session it already has.
    fn upload_status(&self, url: &url::Url, token: &str, size: u64) -> Result<Response> {
        self.deliver(|client, forged| {
            client
                .put(crate::api::gate(forged, url))
                .bearer_auth(token)
                .header("Content-Range", format!("bytes */{size}"))
                .header("Content-Length", 0)
        })
    }
    /// Starts a new resumable upload session.
    fn upload_session(&self, token: &str, size: u64, name: &str, parent: &str) -> Result<Response> {
        self.deliver(|client, forged| {
            client
                .post(crate::api::url_for(forged, "upload/drive/v3/files"))
                .query(&[
                    ("uploadType", "resumable"),
                    ("supportsAllDrives", "true"),
                    ("fields", FIELDS),
                ])
                .bearer_auth(token)
                .header("X-Upload-Content-Type", "application/octet-stream")
                .header("X-Upload-Content-Length", size)
                .json(&serde_json::json!({"name": name, "parents": [parent]}))
        })
    }
    /// Refreshes a token rejected with 401; false when the environment fixes it.
    fn refresh_denied(&self, token: &mut String) -> Result<bool> {
        if std::env::var_os("GDIRECT_ACCESS_TOKEN").is_some() {
            return Ok(false);
        }
        *token = self.download_token()?;
        Ok(true)
    }
    /// Uploads one file; `progress` receives the bytes sent and the file size.
    pub fn upload_with(
        &self,
        path: &Path,
        parent: &str,
        name: Option<&str>,
        progress: impl FnMut(u64, u64),
    ) -> Result<File> {
        self.upload_with_token(path, parent, name, &mut String::new(), progress)
    }
    /// `upload_with` with a token shared by every file of a folder upload.
    fn upload_with_token(
        &self,
        path: &Path,
        parent: &str,
        name: Option<&str>,
        token: &mut String,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<File> {
        let path = path.canonicalize().context("cannot open upload file")?;
        let mut file = fs::File::open(&path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "upload expects a regular file");
        let size = metadata.len();
        let modified = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
        let name = name
            .or_else(|| path.file_name().and_then(|s| s.to_str()))
            .context("file name must be UTF-8; use --name")?;
        ensure!(!name.is_empty(), "file name cannot be empty");
        ensure!(crate::drive::valid_id(parent), "invalid parent folder ID");
        let db = Store::open(&self.db)?;
        let key = Store::upload_key(
            self.account.as_deref().unwrap_or_default(),
            &serde_json::to_string(&(path.to_string_lossy(), parent, name))?,
        );
        let saved = db
            .setting(&key)?
            .map(|s| serde_json::from_str::<Session>(&s))
            .transpose()?;
        let mut start = 0;
        let mut session = saved.filter(|s| s.size == size && s.modified == modified);
        if token.is_empty() {
            *token = self.download_token()?;
        }
        if let Some(s) = &session {
            let url = session_url(&s.url)?;
            let mut response = self.upload_status(&url, token, size)?;
            if response.status() == StatusCode::UNAUTHORIZED && self.refresh_denied(token)? {
                response = self.upload_status(&url, token, size)?;
            }
            match response.status() {
                StatusCode::OK | StatusCode::CREATED => {
                    let result = response.json()?;
                    db.unset(&key)?;
                    progress(size, size);
                    return Ok(result);
                }
                StatusCode::PERMANENT_REDIRECT => start = offset(&response, size)?,
                StatusCode::NOT_FOUND | StatusCode::GONE => session = None,
                _ => return Err(failure(response)),
            }
        }
        let session = match session {
            Some(s) => s,
            None => {
                let mut response = self.upload_session(token, size, name, parent)?;
                if response.status() == StatusCode::UNAUTHORIZED && self.refresh_denied(token)? {
                    response = self.upload_session(token, size, name, parent)?;
                }
                if !response.status().is_success() {
                    return Err(failure(response));
                }
                let raw = response
                    .headers()
                    .get("location")
                    .context("missing upload session")?
                    .to_str()?;
                let s = Session {
                    size,
                    modified,
                    url: session_url(raw)?.into(),
                };
                db.set(&key, &serde_json::to_string(&s)?)?;
                s
            }
        };
        let url = session_url(&session.url)?;
        let mut failures = 0;
        let mut probe = false;
        loop {
            ensure!(!self.cancelled(), "upload cancelled; rerun to resume");
            let current = file.metadata()?;
            ensure!(
                current.len() == size
                    && current.modified()?.duration_since(UNIX_EPOCH)?.as_nanos() == modified,
                "local file changed during upload; rerun with a stable file"
            );
            if start < size {
                progress(start, size);
            }
            let mut body = None;
            let count = (size - start).min(CHUNK);
            if !(probe || size == 0 || start == size) {
                file.seek(SeekFrom::Start(start))?;
                let mut bytes = vec![0; count as usize];
                file.read_exact(&mut bytes)?;
                body = Some(bytes);
            }
            let response = self.deliver(|client, forged| {
                let mut request = client
                    .put(crate::api::gate(forged, &url))
                    .bearer_auth(token.as_str())
                    .timeout(Duration::from_secs(180));
                match &body {
                    None => {
                        request = request
                            .header("Content-Range", format!("bytes */{size}"))
                            .header("Content-Length", 0);
                    }
                    Some(bytes) => {
                        request = request
                            .header("Content-Type", "application/octet-stream")
                            .header(
                                "Content-Range",
                                format!("bytes {}-{}/{size}", start, start + count - 1),
                            )
                            .body(bytes.clone());
                    }
                }
                request
            });
            match response {
                Ok(r) if r.status() == StatusCode::OK || r.status() == StatusCode::CREATED => {
                    let result: File = r.json()?;
                    ensure!(
                        result.size.as_deref() == Some(&size.to_string()),
                        "uploaded size mismatch"
                    );
                    db.unset(&key)?;
                    progress(size, size);
                    return Ok(result);
                }
                Ok(r) if r.status() == StatusCode::PERMANENT_REDIRECT => {
                    let next = offset(&r, size)?;
                    if !probe && next <= start {
                        failures += 1;
                    } else if next > start {
                        failures = 0;
                    }
                    start = next;
                    probe = false;
                }
                Ok(r) if r.status() == StatusCode::UNAUTHORIZED => {
                    *token = self.download_token()?;
                    failures += 1;
                    probe = true;
                }
                Ok(r)
                    if r.status().is_server_error()
                        || r.status() == StatusCode::TOO_MANY_REQUESTS =>
                {
                    failures += 1;
                    probe = true;
                }
                Err(_) => {
                    failures += 1;
                    probe = true;
                }
                Ok(r) => return Err(failure(r)),
            }
            if failures >= 4 {
                bail!("upload interrupted; rerun the same command to resume");
            }
            if failures > 0 {
                std::thread::sleep(Duration::from_secs(1 << failures));
            }
        }
    }
}
/// One local entry to upload. The folder structure is mirrored in Drive.
#[derive(Debug)]
enum Node {
    Dir {
        name: String,
        children: Vec<Node>,
    },
    File {
        name: String,
        path: PathBuf,
        size: u64,
        modified: u128,
    },
}
#[derive(Default)]
struct Scanned {
    files: usize,
    bytes: u64,
    skipped: usize,
}
#[derive(Debug)]
struct Plan {
    name: String,
    children: Vec<Node>,
    files: usize,
    bytes: u64,
    skipped: usize,
}
/// Whether an earlier run already finished this exact file.
fn is_uploaded(saved: Option<&SavedFile>, size: u64, modified: u128) -> bool {
    saved.is_some_and(|saved| saved.size == size && saved.modified == modified)
}
/// Progress of a recursive upload.
#[derive(Clone, Debug, Default)]
pub struct TreeProgress {
    /// Path relative to the folder being uploaded.
    pub path: String,
    /// Files finished in this run, including files skipped as already uploaded.
    pub files_done: usize,
    pub files_total: usize,
    /// Bytes finished in this run, including the current file's progress.
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub file_done: u64,
    pub file_total: u64,
}
/// Result of a recursive upload.
#[derive(Debug)]
pub struct TreeUpload {
    pub id: String,
    pub name: String,
    /// Files uploaded in this run, and their bytes.
    pub uploaded: usize,
    pub bytes: u64,
    /// Files a previous run had already uploaded.
    pub unchanged: usize,
    pub folders: usize,
    /// Local items left out (symbolic links, unreadable entries).
    pub skipped: usize,
}
impl TreeUpload {
    /// A one-line result suitable for a terminal or the browse status bar.
    pub fn summary(&self) -> String {
        let mut parts = vec![if self.uploaded > 0 {
            format!(
                "Uploaded {} ({}) to {}",
                count(self.uploaded),
                human_bytes(self.bytes),
                self.name
            )
        } else if self.unchanged > 0 {
            format!("Nothing new for {}", self.name)
        } else if self.folders > 1 {
            format!("Created {} ({} folders)", self.name, self.folders)
        } else if self.folders == 1 {
            format!("Created {}", self.name)
        } else {
            format!("{} is ready", self.name)
        }];
        if self.unchanged > 0 {
            parts.push(format!("{} already uploaded", count(self.unchanged)));
        }
        if self.skipped > 0 {
            parts.push(format!(
                "skipped {} local {}",
                self.skipped,
                if self.skipped == 1 { "item" } else { "items" }
            ));
        }
        parts.join(" · ")
    }
}
fn count(n: usize) -> String {
    if n == 1 {
        "1 file".into()
    } else {
        format!("{n} files")
    }
}
/// A whole percentage; an empty total counts as complete.
pub fn percent(done: u64, total: u64) -> u64 {
    (u128::from(done) * 100)
        .checked_div(u128::from(total))
        .map_or(100, |value| value.min(100) as u64)
}
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
/// What a previous, interrupted run of the same folder upload already finished.
#[derive(Deserialize, Serialize)]
struct SavedFile {
    size: u64,
    modified: u128,
}
fn scan_root(root: &Path, name: String) -> Result<Plan> {
    let (children, scanned) = scan_dir(root)?;
    Ok(Plan {
        name,
        children,
        files: scanned.files,
        bytes: scanned.bytes,
        skipped: scanned.skipped,
    })
}
fn scan_dir(dir: &Path) -> Result<(Vec<Node>, Scanned)> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("cannot read folder {}", dir.display()))?
    {
        entries.push(entry.with_context(|| format!("cannot read folder {}", dir.display()))?);
    }
    entries.sort_by_cached_key(|entry| entry.file_name());
    let mut nodes = Vec::new();
    let mut scanned = Scanned::default();
    for entry in entries {
        let path = entry.path();
        let kind = entry
            .file_type()
            .with_context(|| format!("cannot inspect {}", path.display()))?;
        // Symlinks could leave the folder or form a cycle; special files have no content to send.
        if kind.is_symlink() || !(kind.is_dir() || kind.is_file()) {
            scanned.skipped += 1;
            continue;
        }
        let name = local_name(&path)?;
        if kind.is_dir() {
            let (children, sub) = scan_dir(&path)?;
            scanned.files += sub.files;
            scanned.bytes += sub.bytes;
            scanned.skipped += sub.skipped;
            nodes.push(Node::Dir { name, children });
        } else {
            let metadata = entry
                .metadata()
                .with_context(|| format!("cannot inspect {}", path.display()))?;
            let size = metadata.len();
            let modified = modified(&metadata, &path)?;
            scanned.files += 1;
            scanned.bytes += size;
            nodes.push(Node::File {
                name,
                path,
                size,
                modified,
            });
        }
    }
    Ok((nodes, scanned))
}
fn local_name(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("the file name is not valid UTF-8: {}", path.display()))?;
    ensure!(!name.is_empty(), "empty file name: {}", path.display());
    ensure!(
        !name.chars().any(char::is_control),
        "file name contains a control character: {}",
        path.display()
    );
    Ok(name.to_owned())
}
fn root_name(root: &Path, name: Option<&str>) -> Result<String> {
    let name = match name {
        Some(name) => {
            let name = name.trim();
            ensure!(!name.is_empty(), "--name cannot be empty");
            name
        }
        None => {
            let name = root
                .file_name()
                .and_then(|name| name.to_str())
                .with_context(|| {
                    format!(
                        "cannot derive a folder name from {}; use --name",
                        root.display()
                    )
                })?;
            ensure!(
                !name.is_empty(),
                "cannot derive a folder name from {}; use --name",
                root.display()
            );
            name
        }
    };
    ensure!(
        !name.chars().any(char::is_control),
        "folder name cannot contain control characters"
    );
    Ok(name.to_owned())
}
fn modified(metadata: &fs::Metadata, path: &Path) -> Result<u128> {
    Ok(metadata
        .modified()
        .with_context(|| format!("cannot read the modification time of {}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("{} was modified before 1970", path.display()))?
        .as_nanos())
}
fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_owned()
    } else {
        format!("{rel}/{name}")
    }
}
/// Private settings prefix for one resumable folder upload.
fn tree_prefix(email: &str, root: &Path, parent: &str, name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gdirect-tree-v1");
    for field in [
        path_bytes(root),
        parent.as_bytes().to_vec(),
        name.as_bytes().to_vec(),
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(&field);
    }
    let hash: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}\n", Store::upload_key(email, &format!("tree:{hash}")))
}
fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str()
            .encode_wide()
            .flat_map(|unit| unit.to_le_bytes())
            .collect()
    }
    #[cfg(not(any(unix, windows)))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}
struct Uploader<'a, F> {
    api: &'a Api,
    db: &'a Store,
    prefix: &'a str,
    dirs: HashMap<String, String>,
    files: HashMap<String, SavedFile>,
    progress: F,
    /// Fetched once for all files; the empty string means not fetched yet.
    token: String,
    files_total: usize,
    bytes_total: u64,
    files_done: usize,
    bytes_done: u64,
    folders: usize,
    uploaded: usize,
    unchanged: usize,
    uploaded_bytes: u64,
}
impl<F: FnMut(TreeProgress)> Uploader<'_, F> {
    fn save_dir(&self, rel: &str, id: &str) -> Result<()> {
        self.db.set(&format!("{}dir:{rel}", self.prefix), id)
    }
    fn save_file(&self, rel: &str, file: &SavedFile) -> Result<()> {
        self.db.set(
            &format!("{}file:{rel}", self.prefix),
            &serde_json::to_string(file)?,
        )
    }
    fn folder(&mut self, rel: &str, name: &str, parent: &str) -> Result<String> {
        if let Some(id) = self.dirs.get(rel) {
            return Ok(id.clone());
        }
        let folder = self
            .api
            .create(parent, name, true)
            .with_context(|| format!("cannot create folder '{name}'"))?;
        self.save_dir(rel, &folder.id)?;
        self.dirs.insert(rel.to_owned(), folder.id.clone());
        self.folders += 1;
        Ok(folder.id)
    }
    fn children(&mut self, children: &[Node], rel: &str, parent: &str) -> Result<()> {
        for node in children {
            match node {
                Node::Dir { name, children } => {
                    let child = join(rel, name);
                    let id = self.folder(&child, name, parent)?;
                    self.children(children, &child, &id)?;
                }
                Node::File {
                    name,
                    path,
                    size,
                    modified,
                } => {
                    let child = join(rel, name);
                    if is_uploaded(self.files.get(&child), *size, *modified) {
                        self.files_done += 1;
                        self.bytes_done += *size;
                        self.unchanged += 1;
                        (self.progress)(TreeProgress {
                            path: child,
                            files_done: self.files_done,
                            files_total: self.files_total,
                            bytes_done: self.bytes_done,
                            bytes_total: self.bytes_total,
                            file_done: *size,
                            file_total: *size,
                        });
                        continue;
                    }
                    if self.token.is_empty() {
                        self.token = self.api.download_token()?;
                    }
                    let before = (self.files_done, self.bytes_done);
                    let totals = (self.files_total, self.bytes_total);
                    let path_for_progress = child.clone();
                    let api = self.api;
                    let token = &mut self.token;
                    let progress = &mut self.progress;
                    api.upload_with_token(path, parent, Some(name), token, |sent, file_total| {
                        let finished = sent >= file_total;
                        progress(TreeProgress {
                            path: path_for_progress.clone(),
                            files_done: before.0 + usize::from(finished),
                            files_total: totals.0,
                            bytes_done: before.1 + sent,
                            bytes_total: totals.1,
                            file_done: sent,
                            file_total,
                        });
                    })?;
                    let saved = SavedFile {
                        size: *size,
                        modified: *modified,
                    };
                    self.save_file(&child, &saved)?;
                    self.files.insert(child, saved);
                    self.files_done += 1;
                    self.bytes_done += *size;
                    self.uploaded += 1;
                    self.uploaded_bytes += *size;
                }
            }
        }
        Ok(())
    }
}
impl Api {
    /// Uploads a local folder and everything inside it, resuming an interrupted
    /// run of the same folder, parent and name. `progress` reports file progress.
    pub fn upload_dir(
        &self,
        path: &Path,
        parent: &str,
        name: Option<&str>,
        restart: bool,
        progress: impl FnMut(TreeProgress),
    ) -> Result<TreeUpload> {
        ensure!(crate::drive::valid_id(parent), "invalid parent folder ID");
        let root = path
            .canonicalize()
            .with_context(|| format!("cannot open upload folder {}", path.display()))?;
        ensure!(root.is_dir(), "{} is not a folder", path.display());
        let name = root_name(&root, name)?;
        let plan = scan_root(&root, name)?;
        let db = Store::open(&self.db)?;
        let prefix = tree_prefix(
            self.account.as_deref().unwrap_or_default(),
            &root,
            parent,
            &plan.name,
        );
        if restart {
            db.remove_settings_with_prefix(&prefix)?;
        }
        let mut dirs: HashMap<String, String> = HashMap::new();
        let mut files: HashMap<String, SavedFile> = HashMap::new();
        for (key, value) in db.settings_with_prefix(&prefix).with_context(
            || "cannot read the saved state of this folder upload; use --restart to start over",
        )? {
            let key = key.strip_prefix(&prefix).unwrap_or_default();
            if let Some(rel) = key.strip_prefix("dir:") {
                dirs.insert(rel.to_owned(), value);
            } else if let Some(rel) = key.strip_prefix("file:") {
                let saved = serde_json::from_str(&value).with_context(|| {
                    format!("saved upload state for {rel} is invalid; use --restart to start over")
                })?;
                files.insert(rel.to_owned(), saved);
            }
        }
        let (root_id, created) = match dirs.get("").cloned() {
            Some(id) => (id, false),
            None => {
                let folder = self
                    .create(parent, &plan.name, true)
                    .with_context(|| format!("cannot create folder '{}'", plan.name))?;
                db.set(&format!("{prefix}dir:"), &folder.id)?;
                dirs.insert(String::new(), folder.id.clone());
                (folder.id, true)
            }
        };
        let mut uploader = Uploader {
            api: self,
            db: &db,
            prefix: &prefix,
            dirs,
            files,
            progress,
            token: String::new(),
            files_total: plan.files,
            bytes_total: plan.bytes,
            files_done: 0,
            bytes_done: 0,
            folders: usize::from(created),
            uploaded: 0,
            unchanged: 0,
            uploaded_bytes: 0,
        };
        if let Err(error) = uploader.children(&plan.children, "", &root_id) {
            return Err(error.context(format!(
                "uploaded {}/{} files from {}; rerun to resume, or use --restart to start over",
                uploader.files_done,
                plan.files,
                root.display()
            )));
        }
        db.remove_settings_with_prefix(&prefix)?;
        Ok(TreeUpload {
            id: root_id,
            name: plan.name,
            uploaded: uploader.uploaded,
            unchanged: uploader.unchanged,
            folders: uploader.folders,
            skipped: plan.skipped,
            bytes: uploader.uploaded_bytes,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgement_bounds() {
        assert_eq!(acknowledged("bytes=0-262143", 524288).unwrap(), 262144);
        for range in [
            "bytes=1-2",
            "bytes=0-524288",
            "bytes=0-18446744073709551615",
        ] {
            assert!(acknowledged(range, 524288).is_err());
        }
    }
    #[test]
    fn rejects_session_token_exfiltration() {
        for url in [
            "http://www.googleapis.com/upload/drive/v3/files",
            "https://evil.test/upload/drive/v3/files",
            "https://www.googleapis.com:444/upload/drive/v3/files",
            "https://user@www.googleapis.com/upload/drive/v3/files",
        ] {
            assert!(session_url(url).is_err());
        }
        assert!(
            session_url("https://www.googleapis.com/upload/drive/v3/files?upload_id=private")
                .is_ok()
        );
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "gdirect-upload-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn layout(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .map(|node| match node {
                Node::Dir { name, children } => format!("{name}/{}", layout(children).join(",")),
                Node::File { name, size, .. } => format!("{name}:{size}"),
            })
            .collect()
    }
    #[test]
    fn folder_trees_are_planned_in_order() {
        let dir = TempDir::new();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        fs::create_dir_all(dir.path().join("sub/deeper")).unwrap();
        fs::write(dir.path().join("sub/b.bin"), b"abc").unwrap();
        let plan = scan_root(dir.path(), "photos".into()).unwrap();
        assert_eq!(plan.name, "photos");
        assert_eq!((plan.files, plan.bytes, plan.skipped), (2, 8, 0));
        assert_eq!(
            layout(&plan.children),
            ["a.txt:5", "empty/", "sub/b.bin:3,deeper/"]
        );
        assert_eq!(join("", "a.txt"), "a.txt");
        assert_eq!(join("sub", "b.bin"), "sub/b.bin");
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        fs::write(dir.path().join("real.txt"), b"x").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();
        symlink(dir.path().join("sub"), dir.path().join("linkdir")).unwrap();
        let plan = scan_root(dir.path(), "root".into()).unwrap();
        assert_eq!((plan.files, plan.bytes, plan.skipped), (1, 1, 2));
        assert_eq!(layout(&plan.children), ["real.txt:1", "sub/"]);
    }
    #[cfg(unix)]
    #[test]
    fn unusual_local_names_are_rejected() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let dir = TempDir::new();
        let not_utf8 = dir.path().join(OsString::from_vec(vec![b'f', 0xff]));
        // APFS refuses names that are not UTF-8, so only some file systems can test them.
        if fs::write(&not_utf8, b"x").is_ok() {
            let error = scan_root(dir.path(), "root".into()).unwrap_err();
            assert!(format!("{error:#}").contains("not valid UTF-8"));
            fs::remove_file(&not_utf8).unwrap();
        }
        fs::write(dir.path().join("bad\nname"), b"x").unwrap();
        let error = scan_root(dir.path(), "root".into()).unwrap_err();
        assert!(format!("{error:#}").contains("control character"));
    }
    #[test]
    fn folder_uploads_reject_bad_paths_before_network() {
        let dir = TempDir::new();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"x").unwrap();
        let api = Api::new(dir.path().join("state.sqlite3"), None, None, None, false).unwrap();
        let error = api
            .upload_dir(&file, "root", None, false, |_| {})
            .unwrap_err();
        assert!(format!("{error:#}").contains("not a folder"));
        let error = api
            .upload_dir(&dir.path().join("missing"), "root", None, false, |_| {})
            .unwrap_err();
        assert!(format!("{error:#}").contains("cannot open upload folder"));
    }
    #[test]
    fn root_names_are_validated() {
        let dir = TempDir::new();
        let local = dir.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(root_name(dir.path(), None).unwrap(), local);
        assert_eq!(root_name(dir.path(), Some("  backup  ")).unwrap(), "backup");
        assert!(root_name(dir.path(), Some("   ")).is_err());
        assert!(root_name(dir.path(), Some("bad\nname")).is_err());
        #[cfg(unix)]
        assert!(root_name(Path::new("/"), None).is_err());
    }
    #[test]
    fn folder_upload_keys_are_scoped() {
        let root = Path::new("/tmp/photos");
        let key = tree_prefix("a@example.com", root, "root", "photos");
        assert_eq!(key, tree_prefix("a@example.com", root, "root", "photos"));
        assert!(key.starts_with("upload:a@example.com\ntree:"));
        assert!(key.ends_with('\n'));
        assert_ne!(key, tree_prefix("b@example.com", root, "root", "photos"));
        for other in [
            tree_prefix("a@example.com", Path::new("/tmp/other"), "root", "photos"),
            tree_prefix("a@example.com", root, "folder", "photos"),
            tree_prefix("a@example.com", root, "root", "other"),
        ] {
            assert_ne!(key, other);
        }
    }
    #[test]
    fn saved_file_states_round_trip() {
        let saved = SavedFile {
            size: 123_456_789,
            modified: 1_700_000_000_123_456_789,
        };
        let text = serde_json::to_string(&saved).unwrap();
        let back: SavedFile = serde_json::from_str(&text).unwrap();
        assert_eq!((back.size, back.modified), (saved.size, saved.modified));
    }
    #[test]
    fn finished_files_are_skipped_only_when_unchanged() {
        let saved = SavedFile {
            size: 5,
            modified: 7,
        };
        assert!(is_uploaded(Some(&saved), 5, 7));
        assert!(!is_uploaded(Some(&saved), 6, 7));
        assert!(!is_uploaded(Some(&saved), 5, 8));
        assert!(!is_uploaded(None, 5, 7));
    }
    #[test]
    fn folder_upload_summaries() {
        let result = |uploaded, unchanged, folders, skipped, bytes| TreeUpload {
            id: "id".into(),
            name: "photos".into(),
            uploaded,
            unchanged,
            folders,
            skipped,
            bytes,
        };
        assert_eq!(
            result(1, 0, 1, 0, 1024).summary(),
            "Uploaded 1 file (1.0 KiB) to photos"
        );
        assert_eq!(
            result(3, 2, 1, 0, 1536).summary(),
            "Uploaded 3 files (1.5 KiB) to photos · 2 files already uploaded"
        );
        assert_eq!(
            result(0, 2, 0, 0, 0).summary(),
            "Nothing new for photos · 2 files already uploaded"
        );
        assert_eq!(
            result(0, 0, 3, 0, 0).summary(),
            "Created photos (3 folders)"
        );
        assert_eq!(
            result(0, 0, 1, 2, 0).summary(),
            "Created photos · skipped 2 local items"
        );
        assert_eq!(result(0, 0, 0, 0, 0).summary(), "photos is ready");
    }
    #[test]
    fn percentages_stay_within_bounds() {
        assert_eq!(percent(0, 0), 100);
        assert_eq!(percent(0, 8), 0);
        assert_eq!(percent(1, 4), 25);
        assert_eq!(percent(5, 4), 100);
        assert_eq!(percent(u64::MAX, u64::MAX), 100);
    }
}
