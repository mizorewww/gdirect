//! The download and upload queues: each runs one item at a time on its own
//! thread, as `gdirect browse` does, and reports through the app's channel.
use crate::format;
use chrono::{DateTime, Local};
use gdirect::{
    api::{Api, File},
    download::{self, Stats},
    upload::TreeProgress,
};
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Queued,
    Running,
    /// Cancelled while running; the partial file is kept and resumes.
    Paused,
    Done {
        at: DateTime<Local>,
        note: String,
    },
    Failed(String),
}

impl State {
    pub fn finished(&self) -> bool {
        matches!(self, Self::Done { .. })
    }
    pub fn pending(&self) -> bool {
        !self.finished()
    }
}

pub struct Download {
    pub id: u64,
    /// The account that queued it; it runs only while that account is active.
    pub account: String,
    pub file: File,
    pub output: PathBuf,
    /// The export MIME type of a Google document.
    pub export: Option<String>,
    pub size: Option<u64>,
    pub state: State,
    pub stats: Stats,
    pub started: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Download {
    pub fn fraction(&self) -> f32 {
        let total = if self.stats.total > 0 {
            self.stats.total
        } else {
            self.size.unwrap_or(0)
        };
        match (&self.state, total) {
            (State::Done { .. }, _) => 1.,
            (_, 0) => 0.,
            _ => (self.stats.done as f64 / total as f64).clamp(0., 1.) as f32,
        }
    }
}

pub struct Upload {
    pub id: u64,
    pub account: String,
    pub path: PathBuf,
    pub folder: bool,
    /// Destination folder id and name.
    pub parent: (String, String),
    pub state: State,
    pub progress: TreeProgress,
    /// Bytes per second, smoothed.
    pub speed: u64,
    pub sampled: Option<(Instant, u64)>,
    /// The Drive id of the finished upload.
    pub result: Option<String>,
}

impl Upload {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string())
    }
    pub fn fraction(&self) -> f32 {
        let p = &self.progress;
        if self.state.finished() {
            1.
        } else if p.bytes_total > 0 {
            (p.bytes_done as f64 / p.bytes_total as f64) as f32
        } else if p.files_total > 0 {
            p.files_done as f32 / p.files_total as f32
        } else {
            0.
        }
    }
    pub fn sample(&mut self, progress: TreeProgress) {
        let now = Instant::now();
        match self.sampled {
            Some((at, bytes)) if now.duration_since(at).as_millis() >= 500 => {
                let elapsed = now.duration_since(at).as_secs_f64();
                let rate = (progress.bytes_done.saturating_sub(bytes)) as f64 / elapsed;
                self.speed = if self.speed == 0 {
                    rate as u64
                } else {
                    (self.speed as f64 * 0.6 + rate * 0.4) as u64
                };
                self.sampled = Some((now, progress.bytes_done));
            }
            None => self.sampled = Some((now, progress.bytes_done)),
            _ => {}
        }
        self.progress = progress;
    }
}

/// How the download thread was configured, from the settings at its start.
pub struct Options {
    pub connections: u8,
    pub md5: bool,
    /// Explicit identity: see `settings::Identity::sni`.
    pub sni: Option<String>,
    pub ca_cert: Option<PathBuf>,
}

#[derive(Default)]
pub struct Queues {
    pub downloads: Vec<Download>,
    pub uploads: Vec<Upload>,
    next: u64,
}

impl Queues {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Drive allows duplicate names; give each queued download its own output
    /// path. Unrelated files already on disk are avoided when it starts.
    fn output_for(&self, dir: &Path, name: &str, file_id: &str) -> PathBuf {
        let taken = |p: &Path| {
            self.downloads
                .iter()
                .any(|d| d.file.id != file_id && d.output == p)
        };
        let first = dir.join(name);
        if !taken(&first) {
            return first;
        }
        let path = Path::new(name);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ext = path.extension().map(|e| e.to_string_lossy());
        (2..)
            .map(|n| match &ext {
                Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
                None => dir.join(format!("{stem} ({n})")),
            })
            .find(|p| !taken(p))
            .unwrap()
    }

    /// Queues files into their directories; returns how many were added and skipped.
    pub fn enqueue(
        &mut self,
        files: Vec<(File, PathBuf)>,
        office: bool,
        account: &str,
    ) -> (usize, usize) {
        let (mut added, mut skipped) = (0, 0);
        for (file, dir) in files {
            if file.folder() || !file.downloadable() {
                skipped += 1;
                continue;
            }
            if let Some(i) = self
                .downloads
                .iter()
                .position(|d| d.file.id == file.id && d.account == account)
            {
                if matches!(self.downloads[i].state, State::Queued | State::Running) {
                    skipped += 1;
                    continue;
                }
                // Finished, paused or failed earlier: queue it again at the end.
                let mut job = self.downloads.remove(i);
                job.state = State::Queued;
                self.downloads.push(job);
                added += 1;
                continue;
            }
            let export = format::export(&file, office);
            let mut name = gdirect::api::safe_name(&file.name);
            if let Some((_, ext)) = export {
                name = format!("{name}.{ext}");
            }
            let id = self.id();
            self.downloads.push(Download {
                id,
                account: account.to_owned(),
                output: self.output_for(&dir, &name, &file.id),
                export: export.map(|(mime, _)| mime.to_owned()),
                size: file.size.as_ref().and_then(|s| s.parse().ok()),
                state: State::Queued,
                stats: Stats::default(),
                started: None,
                cancel: None,
                file,
            });
            added += 1;
        }
        (added, skipped)
    }

    pub fn enqueue_upload(&mut self, path: PathBuf, parent: (String, String), account: &str) {
        let id = self.id();
        self.uploads.push(Upload {
            id,
            account: account.to_owned(),
            folder: path.is_dir(),
            path,
            parent,
            state: State::Queued,
            progress: TreeProgress::default(),
            speed: 0,
            sampled: None,
            result: None,
        });
    }

    pub fn download(&mut self, id: u64) -> Option<&mut Download> {
        self.downloads.iter_mut().find(|d| d.id == id)
    }

    pub fn upload(&mut self, id: u64) -> Option<&mut Upload> {
        self.uploads.iter_mut().find(|u| u.id == id)
    }

    pub fn active_download(&self) -> Option<&Download> {
        self.downloads.iter().find(|d| d.state == State::Running)
    }

    pub fn active_upload(&self) -> Option<&Upload> {
        self.uploads.iter().find(|u| u.state == State::Running)
    }

    /// The state the file list shows next to a file.
    pub fn file_state(&self, file_id: &str) -> Option<&State> {
        self.downloads
            .iter()
            .rev()
            .find(|d| d.file.id == file_id)
            .map(|d| &d.state)
    }

    /// Items not yet finished, for the navigation badge.
    pub fn pending(&self) -> usize {
        self.downloads
            .iter()
            .filter(|d| matches!(d.state, State::Queued | State::Running))
            .count()
            + self
                .uploads
                .iter()
                .filter(|u| matches!(u.state, State::Queued | State::Running))
                .count()
    }

    pub fn retry_failed(&mut self) -> usize {
        let mut n = 0;
        for d in &mut self.downloads {
            if matches!(d.state, State::Failed(_)) {
                d.state = State::Queued;
                n += 1;
            }
        }
        for u in &mut self.uploads {
            if matches!(u.state, State::Failed(_)) {
                u.state = State::Queued;
                n += 1;
            }
        }
        n
    }

    pub fn clear_queued(&mut self, uploads: bool) {
        if uploads {
            self.uploads.retain(|u| u.state != State::Queued);
        } else {
            self.downloads.retain(|d| d.state != State::Queued);
        }
    }

    pub fn clear_finished(&mut self, uploads: bool) {
        if uploads {
            self.uploads.retain(|u| !u.state.finished());
        } else {
            self.downloads.retain(|d| !d.state.finished());
        }
    }

    /// The account's next queued download, marked running; the caller starts it.
    pub fn next_download(&mut self, account: &str) -> Option<&mut Download> {
        if self.active_download().is_some() {
            return None;
        }
        let job = self
            .downloads
            .iter_mut()
            .find(|d| d.state == State::Queued && d.account == account)?;
        job.state = State::Running;
        job.stats = Stats::default();
        job.started = Some(Instant::now());
        Some(job)
    }

    pub fn next_upload(&mut self, account: &str) -> Option<&mut Upload> {
        if self.active_upload().is_some() {
            return None;
        }
        let job = self
            .uploads
            .iter_mut()
            .find(|u| u.state == State::Queued && u.account == account)?;
        job.state = State::Running;
        job.speed = 0;
        job.sampled = None;
        Some(job)
    }
}

fn control(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.aria2", path.display()))
}

/// Where a download goes on disk: its queued path, unless an unrelated file
/// is already there. A finished copy with the same MD5 stays (the engine
/// skips it), and a partial one with its aria2 control file resumes.
fn output_on_disk(path: &Path, md5: Option<&str>) -> anyhow::Result<PathBuf> {
    let usable = |p: &Path| -> anyhow::Result<bool> {
        Ok(!p.exists() || control(p).exists() || download::matches_md5(p, md5)?)
    };
    if usable(path)? {
        return Ok(path.to_owned());
    }
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path.extension().map(|e| e.to_string_lossy());
    for n in 2.. {
        let candidate = match &ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        };
        if usable(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

/// Runs one download to completion; `progress` gets live statistics. Returns
/// a note for the finished list and the path the file was saved to.
pub fn run_download(
    api: &Api,
    job: &Download,
    options: &Options,
    cancel: Arc<AtomicBool>,
    progress: std::sync::mpsc::Sender<Stats>,
) -> anyhow::Result<(String, PathBuf)> {
    if let Some(dir) = job.output.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let md5 = if options.md5 && job.export.is_none() {
        job.file.md5_checksum.clone()
    } else {
        None
    };
    let output = output_on_disk(&job.output, md5.as_deref())?;
    let token = api.download_token()?;
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "已取消");
    // The API client may already have failed over; downloads follow it.
    let (ip, sni, backups) = api.route();
    let existed = output.exists() && !control(&output).exists();
    let result = download::run(
        download::Args {
            file: job.file.id.clone(),
            output: output.clone(),
            ip,
            sni: options.sni.clone().or(sni),
            auto: options.sni.is_none(),
            backups,
            db: Some(api.db.clone()),
            token: Some(token),
            export: job.export.clone(),
            md5,
            connections: options.connections,
            ca_cert: options.ca_cert.clone(),
        },
        Some(&mut download::Progress {
            tx: progress,
            cancel: cancel.clone(),
        }),
    );
    anyhow::ensure!(
        !cancel.load(Ordering::Relaxed),
        "已取消，已下载的部分会保留"
    );
    result?;
    let size = output
        .metadata()
        .map(|m| format::bytes(m.len()))
        .unwrap_or_default();
    let note = if existed {
        "已存在且校验和一致，跳过下载".into()
    } else if job.export.is_some() {
        format!("{size} · 由 Google 文档导出")
    } else if options.md5 && job.file.md5_checksum.is_some() {
        format!("{size} · MD5 校验通过")
    } else {
        size
    };
    Ok((note, output))
}

/// Lists every file below `folder`, paired with its directory relative to it.
pub fn walk(
    api: &Api,
    folder: &str,
    progress: impl Fn(usize),
) -> anyhow::Result<Vec<(File, PathBuf)>> {
    let mut files = Vec::new();
    let mut pending = VecDeque::from([(folder.to_owned(), PathBuf::new())]);
    // A folder can have several parents; list each one only once.
    let mut seen = HashSet::new();
    while let Some((id, dir)) = pending.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for file in api.list(&id)? {
            if file.folder() {
                let sub = dir.join(gdirect::api::safe_name(&file.name));
                pending.push_back((file.id.clone(), sub));
            } else {
                files.push((file, dir.clone()));
            }
        }
        progress(files.len());
    }
    Ok(files)
}

/// An engine error in the client's words: its messages are written for the
/// command line, and name flags and commands the client does not have.
pub fn explain(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if let Some(code) = text
        .split("libaria2 code ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .and_then(|code| code.parse::<i32>().ok())
    {
        let reason = match code {
            1 => "连接或 TLS 握手失败，可在“线路”页换一个地址",
            2 => "连接超时，可在“线路”页重新扫描",
            3 => "文件不存在，或没有与此账号共享",
            6 => "网络问题",
            9 => "磁盘空间不足",
            13 => "目标文件已存在",
            15..=18 => "无法写入文件，请检查下载目录的权限和剩余空间",
            19 => "域名解析失败",
            22 => "Google 云盘拒绝了下载（配额、权限或下载次数限制）",
            24 => "授权被拒绝，请在“设置 › 账号”重新登录",
            _ => "下载出错",
        };
        return format!("{reason}（libaria2 错误码 {code}）· 已下载的部分会保留，重试即可续传");
    }
    if text.contains("MD5 does not match") {
        return "下载完成，但 MD5 与云盘记录不一致；文件已保留，以便检查".into();
    }
    if text.contains("login/quota error page") {
        return "Google 云盘返回了登录或配额错误页".into();
    }
    if text.contains("local file changed during upload") {
        return "上传期间本地文件被修改，请在文件稳定后重试".into();
    }
    text
}

/// A finished folder upload in the client's words.
pub fn folder_summary(result: &gdirect::upload::TreeUpload) -> String {
    let mut parts = vec![if result.uploaded > 0 {
        format!(
            "已上传 {} 个文件（{}）",
            result.uploaded,
            format::bytes(result.bytes)
        )
    } else if result.unchanged > 0 {
        "没有新文件".to_owned()
    } else {
        format!("已建立 {} 个文件夹", result.folders.max(1))
    }];
    if result.unchanged > 0 {
        parts.push(format!("{} 个此前已上传", result.unchanged));
    }
    if result.skipped > 0 {
        parts.push(format!("跳过 {} 个本地项（如符号链接）", result.skipped));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_on_disk_avoids_unrelated_files() {
        let dir = std::env::temp_dir().join(format!("gdirect-desktop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("report.pdf");
        // Nothing there yet: the queued path.
        assert_eq!(output_on_disk(&path, None).unwrap(), path);
        // An unrelated file: the next free name.
        std::fs::write(&path, b"other").unwrap();
        assert_eq!(
            output_on_disk(&path, Some("00000000000000000000000000000000")).unwrap(),
            dir.join("report (2).pdf")
        );
        // The same file (MD5 of "other"): kept, so the engine skips it.
        assert_eq!(
            output_on_disk(&path, Some("795f3202b17cb6bc3d4b771d8c6c9eaf")).unwrap(),
            path
        );
        // A partial download resumes in place.
        std::fs::write(control(&path), b"").unwrap();
        assert_eq!(output_on_disk(&path, None).unwrap(), path);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
