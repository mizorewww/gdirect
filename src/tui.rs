mod draw;

use crate::{
    api::{Api, File},
    download::{self, Stats},
    store::Store,
    upload::{self, TreeProgress},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::widgets::ListState;
use std::{
    collections::{HashSet, VecDeque},
    io::IsTerminal,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub struct Options {
    pub folder: String,
    pub output: PathBuf,
    /// --no-sni: downloads send no SNI (Drive API requests keep the real name).
    pub no_sni: bool,
    pub auto: bool,
    pub connections: u8,
    pub ca_cert: Option<PathBuf>,
}
enum Message {
    Folder(String, Result<Vec<File>, String>),
    Preview(String, Result<Vec<File>, String>),
    Finished(String, Result<(), String>),
    /// Progress of a folder download: folder name and files found so far.
    Listing(String, usize),
    /// Every file below a folder, with the directory to save it in.
    Listed(String, Result<Vec<(File, PathBuf)>, String>),
    /// A file operation finished: its status line and the file to select afterwards.
    Operated(Result<(String, Option<String>), String>),
    /// Live progress of an upload, shown in the status line.
    Uploading(String),
}
#[derive(Clone, Debug, PartialEq)]
enum State {
    Queued,
    Active,
    Done,
    Failed(String),
}
struct Job {
    file: File,
    output: PathBuf,
    size: Option<u64>,
    state: State,
}
struct Active {
    id: String,
    stats: Stats,
    rx: Receiver<Stats>,
    cancel: Arc<AtomicBool>,
    worker: thread::JoinHandle<()>,
}
/// Files yanked with y (copy) or x (cut), pasted into another folder with p.
struct Clipboard {
    files: Vec<File>,
    cut: bool,
}
enum Prompt {
    /// A name ending in / creates a folder.
    Create,
    Rename(File),
    Trash(Vec<File>),
    Upload,
    Filter,
}
struct Input {
    prompt: Prompt,
    text: String,
    /// Cursor position in characters.
    cursor: usize,
}
impl Input {
    fn new(prompt: Prompt, text: String) -> Self {
        let cursor = text.chars().count();
        Self {
            prompt,
            text,
            cursor,
        }
    }
    fn byte(&self, cursor: usize) -> usize {
        self.text
            .char_indices()
            .nth(cursor)
            .map_or(self.text.len(), |(i, _)| i)
    }
    /// Edits the text; returns false for keys that are not editing keys.
    fn edit(&mut self, key: KeyEvent) -> bool {
        let len = self.text.chars().count();
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let at = self.byte(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Delete if self.cursor < len => {
                let at = self.byte(self.cursor);
                self.text.remove(at);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            KeyCode::Backspace | KeyCode::Delete => {}
            _ => return false,
        }
        true
    }
}
struct View {
    path: Vec<(String, String)>,
    /// Everything in the current folder.
    entries: Vec<File>,
    /// The entries matching the filter, as shown.
    files: Vec<File>,
    filter: String,
    /// The folder above, from the cache, for the parent column.
    parent: Vec<File>,
    selection: ListState,
    /// Rows in the file list, for page up/down.
    page: usize,
    preview: Vec<File>,
    preview_id: Option<String>,
    status: String,
    /// The status is a listing error that a successful refresh clears.
    stale: bool,
    loading: bool,
    connections: u8,
    /// Files marked with space, kept across folders until used.
    marked: Vec<File>,
    clipboard: Option<Clipboard>,
    input: Option<Input>,
    help: bool,
    /// File operations still running.
    busy: usize,
    started: Instant,
    /// Every download of this session, in queue order.
    jobs: Vec<Job>,
    active: Option<Active>,
    /// Folder downloads still being listed.
    listing: usize,
    quitting: bool,
    /// The signed-in account being browsed, shown in the header.
    account: Option<String>,
}
impl View {
    fn new(path: (String, String), files: Vec<File>, connections: u8) -> Self {
        let mut view = Self {
            path: vec![path],
            entries: vec![],
            files: vec![],
            filter: String::new(),
            parent: vec![],
            selection: ListState::default(),
            page: 10,
            preview: vec![],
            preview_id: None,
            status: String::new(),
            stale: false,
            loading: true,
            connections,
            marked: vec![],
            clipboard: None,
            input: None,
            help: false,
            busy: 0,
            started: Instant::now(),
            jobs: vec![],
            active: None,
            listing: 0,
            quitting: false,
            account: None,
        };
        view.set_entries(files, None);
        view
    }
    fn folder(&self) -> &str {
        &self.path.last().unwrap().0
    }
    fn selected(&self) -> Option<&File> {
        self.selection.selected().and_then(|i| self.files.get(i))
    }
    fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.file.id == id)
    }
    fn job_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.file.id == id)
    }
    fn marked(&self, file: &File) -> bool {
        self.marked.iter().any(|m| m.id == file.id)
    }
    /// Replaces the folder listing, keeping the selection on `keep` or the current file.
    fn set_entries(&mut self, files: Vec<File>, keep: Option<String>) {
        let keep = keep.or_else(|| self.selected().map(|f| f.id.clone()));
        self.entries = files;
        self.apply_filter(keep);
    }
    fn apply_filter(&mut self, keep: Option<String>) {
        let needle = self.filter.to_lowercase();
        self.files = self
            .entries
            .iter()
            .filter(|f| needle.is_empty() || f.name.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let found = keep.and_then(|id| self.files.iter().position(|f| f.id == id));
        let index = found.unwrap_or_else(|| self.selection.selected().unwrap_or(0));
        self.selection.select(if self.files.is_empty() {
            None
        } else {
            Some(index.min(self.files.len() - 1))
        });
    }
    fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.selection.select(Some(0));
        self.apply_filter(None);
    }
    /// Opens a subfolder whose cached listing is `files`.
    fn enter(&mut self, folder: File, files: Vec<File>) {
        self.path.push((folder.id, folder.name));
        self.parent = std::mem::take(&mut self.entries);
        self.filter.clear();
        self.selection.select(Some(0));
        self.set_entries(files, None);
        self.loading = true;
    }
    /// Returns to the parent folder, selecting the folder just left.
    fn leave(&mut self, files: Vec<File>, parent: Vec<File>) {
        let (left, _) = self.path.pop().unwrap();
        self.parent = parent;
        self.filter.clear();
        self.selection.select(Some(0));
        self.set_entries(files, Some(left));
        self.loading = true;
    }
    fn move_to(&mut self, index: isize) {
        if self.files.is_empty() {
            self.selection.select(None);
            return;
        }
        let next = index.clamp(0, self.files.len() as isize - 1) as usize;
        self.selection.select(Some(next));
    }
    fn move_by(&mut self, delta: isize) {
        self.move_to(self.selection.selected().unwrap_or(0) as isize + delta);
    }
    fn toggle_mark(&mut self) {
        let Some(file) = self.selected().cloned() else {
            return;
        };
        if let Some(i) = self.marked.iter().position(|f| f.id == file.id) {
            self.marked.remove(i);
        } else {
            self.marked.push(file);
        }
        self.move_by(1);
    }
    /// Marks everything shown, or unmarks it when all of it is marked.
    fn toggle_all(&mut self) {
        let files = self.files.clone();
        if files.iter().all(|f| self.marked(f)) {
            self.marked.retain(|m| !files.iter().any(|f| f.id == m.id));
        } else {
            for file in files {
                if !self.marked(&file) {
                    self.marked.push(file);
                }
            }
        }
    }
    /// The marked files, or the selected one; marks are used up.
    fn take_picked(&mut self) -> Vec<File> {
        if self.marked.is_empty() {
            self.selected().cloned().into_iter().collect()
        } else {
            std::mem::take(&mut self.marked)
        }
    }
    fn yank(&mut self, cut: bool) {
        let files = self.take_picked();
        if files.is_empty() {
            return;
        }
        self.status = format!(
            "{} {} · p pastes into the current folder",
            if cut { "Cut" } else { "Copied" },
            items(files.len())
        );
        self.clipboard = Some(Clipboard { files, cut });
    }
    /// Esc steps back: filter first, then marks, then the clipboard.
    fn escape(&mut self) {
        if !self.filter.is_empty() {
            self.set_filter(String::new());
        } else if !self.marked.is_empty() {
            self.marked.clear();
        } else {
            self.clipboard = None;
        }
    }
    /// Drive allows duplicate names; give each job of this session its own output path.
    fn output_for(&self, dir: &Path, file: &File) -> PathBuf {
        let name = crate::api::safe_name(&file.name);
        let name = if file.workspace() {
            format!("{name}.pdf")
        } else {
            name
        };
        let taken = |p: &Path| {
            self.jobs
                .iter()
                .any(|j| j.file.id != file.id && j.output == p)
        };
        let first = dir.join(&name);
        if !taken(&first) {
            return first;
        }
        let path = Path::new(&name);
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
    /// Queues each file into its directory; returns the queued names and the number skipped.
    fn enqueue(&mut self, files: Vec<(File, PathBuf)>) -> (Vec<String>, usize) {
        let (mut added, mut skipped) = (Vec::new(), 0);
        for (file, dir) in files {
            if file.folder() || !file.downloadable() {
                skipped += 1;
                continue;
            }
            match self.jobs.iter().position(|j| j.file.id == file.id) {
                Some(i) if matches!(self.jobs[i].state, State::Queued | State::Active) => {
                    skipped += 1;
                    continue;
                }
                // Finished or failed earlier: queue it again at the end.
                Some(i) => {
                    let mut job = self.jobs.remove(i);
                    job.state = State::Queued;
                    self.jobs.push(job);
                }
                None => self.jobs.push(Job {
                    output: self.output_for(&dir, &file),
                    size: file.size.as_ref().and_then(|s| s.parse().ok()),
                    state: State::Queued,
                    file: file.clone(),
                }),
            }
            added.push(file.name);
        }
        (added, skipped)
    }
    fn retry_failed(&mut self) {
        let failed: Vec<_> = self
            .jobs
            .iter()
            .filter(|j| matches!(j.state, State::Failed(_)))
            .map(|j| (j.file.clone(), PathBuf::new()))
            .collect();
        self.status = if failed.is_empty() {
            "No failed downloads".into()
        } else {
            // Outputs are already assigned, so the directory is not consulted again.
            let (added, skipped) = self.enqueue(failed);
            queued(&added, skipped)
        };
    }
    fn clear_queue(&mut self) {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.state != State::Queued);
        self.status = format!("Removed {} queued downloads", before - self.jobs.len());
    }
    fn count(&self, state: fn(&State) -> bool) -> usize {
        self.jobs.iter().filter(|j| state(&j.state)).count()
    }
    fn summary(&self) -> String {
        let done = self.count(|s| *s == State::Done);
        let failed = self.count(|s| matches!(s, State::Failed(_)));
        let mut text = format!("{done}/{} done", self.jobs.len());
        if failed > 0 {
            text += &format!(" · {failed} failed");
        }
        let total: u64 = self
            .jobs
            .iter()
            .filter(|j| !matches!(j.state, State::Failed(_)))
            .filter_map(|j| j.size)
            .sum();
        if total > 0 {
            let saved: u64 = self
                .jobs
                .iter()
                .filter(|j| j.state == State::Done)
                .filter_map(|j| j.size)
                .sum::<u64>()
                + self.active.as_ref().map_or(0, |a| a.stats.done);
            text += &format!(" · {} / {}", bytes(saved.min(total)), bytes(total));
        }
        text
    }
}
fn queued(added: &[String], skipped: usize) -> String {
    match (added, skipped) {
        ([], 0) => "Nothing to download".into(),
        ([], _) => "Nothing to download: already queued, or not downloadable".into(),
        ([name], 0) => format!("Queued {name}"),
        (names, 0) => format!("Queued {} files", names.len()),
        (names, skipped) => format!("Queued {} files, skipped {skipped}", names.len()),
    }
}
fn items(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}
/// Lists every file below `folder`, paired with its directory relative to `folder`.
fn walk(
    list: impl Fn(&str) -> Result<Vec<File>>,
    folder: &str,
    progress: impl Fn(usize),
) -> Result<Vec<(File, PathBuf)>> {
    let mut files = Vec::new();
    let mut pending = VecDeque::from([(folder.to_owned(), PathBuf::new())]);
    // A folder can have several parents; list each one only once.
    let mut seen = HashSet::new();
    while let Some((id, dir)) = pending.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        for file in list(&id)? {
            if file.folder() {
                let sub = dir.join(crate::api::safe_name(&file.name));
                pending.push_back((file.id.clone(), sub));
            } else {
                files.push((file, dir.clone()));
            }
        }
        progress(files.len());
    }
    Ok(files)
}
fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}
fn bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}
/// A local path as typed or dropped into the terminal: quoted, escaped, or starting with ~.
fn local_path(text: &str) -> PathBuf {
    let text = text.trim();
    let text = ['\'', '"']
        .iter()
        .find_map(|q| text.strip_prefix(*q)?.strip_suffix(*q))
        .map(str::to_owned)
        .unwrap_or_else(|| text.replace("\\ ", " "));
    match text.strip_prefix("~/").zip(dirs::home_dir()) {
        Some((rest, home)) => home.join(rest),
        None => PathBuf::from(text),
    }
}
fn upload_status(progress: &TreeProgress) -> String {
    // A folder of empty files has no byte total; count files instead.
    let percent = if progress.bytes_total > 0 {
        upload::percent(progress.bytes_done, progress.bytes_total)
    } else {
        upload::percent(progress.files_done as u64, progress.files_total as u64)
    };
    format!(
        "Uploading {} · {}/{} files · {percent}%",
        progress.path, progress.files_done, progress.files_total
    )
}
fn fetch(api: &Api, tx: &Sender<Message>, id: String, preview: bool) {
    let api = api.clone();
    let tx = tx.clone();
    thread::spawn(move || {
        let result = api.list(&id).map_err(|e| format!("{e:#}"));
        let _ = tx.send(if preview {
            Message::Preview(id, result)
        } else {
            Message::Folder(id, result)
        });
    });
}
/// Runs a file operation in the background; the current folder is refreshed when it ends.
fn operate(
    api: &Api,
    tx: &Sender<Message>,
    work: impl FnOnce(&Api) -> Result<(String, Option<String>)> + Send + 'static,
) {
    operate_with(api, tx, move |api, _| work(api));
}
/// `operate` for work that reports progress through the message channel.
fn operate_with(
    api: &Api,
    tx: &Sender<Message>,
    work: impl FnOnce(&Api, Sender<Message>) -> Result<(String, Option<String>)> + Send + 'static,
) {
    let api = api.clone();
    let tx = tx.clone();
    thread::spawn(move || {
        let result = work(&api, tx.clone()).map_err(|e| format!("{e:#}"));
        let _ = tx.send(Message::Operated(result));
    });
}
/// Applies `each` to every file in turn, stopping at the first error.
fn each(
    files: &[File],
    done: &str,
    mut each: impl FnMut(&File) -> Result<bool>,
) -> Result<(String, Option<String>)> {
    let (mut count, mut skipped) = (0, 0);
    for file in files {
        match each(file) {
            Ok(true) => count += 1,
            Ok(false) => skipped += 1,
            Err(e) if count > 0 => anyhow::bail!("{} {done}; {}: {e:#}", items(count), file.name),
            Err(e) => anyhow::bail!("{}: {e:#}", file.name),
        }
    }
    let mut status = format!("{} {done}", items(count));
    if skipped > 0 {
        status += &format!(", skipped {skipped}");
    }
    Ok((status, files.first().map(|f| f.id.clone())))
}
/// Lists a folder tree in the background; its files are queued under `dir/<folder name>`.
fn download_folder(api: &Api, tx: &Sender<Message>, folder: File, dir: &Path) {
    let api = api.clone();
    let tx = tx.clone();
    let root = dir.join(crate::api::safe_name(&folder.name));
    thread::spawn(move || {
        let name = folder.name;
        let result = walk(
            |id| api.list(id),
            &folder.id,
            |count| {
                let _ = tx.send(Message::Listing(name.clone(), count));
            },
        )
        .map(|files| {
            files
                .into_iter()
                .map(|(file, sub)| (file, root.join(sub)))
                .collect()
        })
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Message::Listed(name, result));
    });
}
fn start(api: &Api, options: &Options, tx: &Sender<Message>, job: &Job, connections: u8) -> Active {
    let (progress_tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let api = api.clone();
    let tx = tx.clone();
    let id = job.file.id.clone();
    // The API client may already have failed over; downloads follow it.
    let (ip, sni, backups) = api.route();
    let mut args = download::Args {
        file: id.clone(),
        output: job.output.clone(),
        ip,
        sni: if options.no_sni {
            Some(String::new())
        } else {
            sni
        },
        auto: options.auto,
        backups,
        db: Some(api.db.clone()),
        token: None,
        export: job.file.workspace().then(|| "application/pdf".into()),
        md5: job.file.md5_checksum.clone(),
        connections,
        ca_cert: options.ca_cert.clone(),
    };
    let worker_id = id.clone();
    let worker = thread::spawn(move || {
        let result = (|| -> Result<()> {
            args.token = Some(api.download_token()?);
            if worker_cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Cancelled");
            }
            let result = download::run(
                args,
                Some(&mut download::Progress {
                    tx: progress_tx,
                    cancel: worker_cancel.clone(),
                }),
            );
            if worker_cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Cancelled; partial download saved");
            }
            result
        })()
        .map_err(|e| format!("{e:#}"));
        let _ = tx.send(Message::Finished(worker_id, result));
    });
    Active {
        id,
        stats: Stats::default(),
        rx,
        cancel,
        worker,
    }
}
pub fn run(api: Api, options: Options) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "browse needs a terminal; use gdirect ls --json for scripts"
    );
    api.token()?;
    let mut terminal = ratatui::try_init()?;
    let result = event_loop(&mut terminal, api, options);
    ratatui::restore();
    result
}
/// Handles a key while a prompt is open.
fn prompt_key(view: &mut View, api: &Api, tx: &Sender<Message>, key: KeyEvent) {
    let Some(input) = view.input.as_mut() else {
        return;
    };
    if let Prompt::Trash(_) = input.prompt {
        let Some(Input {
            prompt: Prompt::Trash(files),
            ..
        }) = view.input.take()
        else {
            unreachable!()
        };
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            view.marked.retain(|m| !files.iter().any(|f| f.id == m.id));
            view.status = format!("Moving {} to the trash…", items(files.len()));
            view.busy += 1;
            operate(api, tx, move |api| {
                each(&files, "moved to the trash", |f| {
                    api.trash(&f.id).map(|_| true)
                })
            });
        } else {
            view.status = "Nothing moved to the trash".into();
        }
        return;
    }
    match key.code {
        KeyCode::Esc => {
            if let Some(Input {
                prompt: Prompt::Filter,
                ..
            }) = view.input.take()
            {
                view.set_filter(String::new());
            }
        }
        KeyCode::Enter => {
            let input = view.input.take().unwrap();
            let text = input.text;
            let folder = view.folder().to_owned();
            match input.prompt {
                Prompt::Filter => {}
                Prompt::Create => {
                    let (name, dir) = match text.trim().strip_suffix('/') {
                        Some(name) => (name.to_owned(), true),
                        None => (text.trim().to_owned(), false),
                    };
                    if name.is_empty() {
                        return;
                    }
                    view.busy += 1;
                    view.status = format!("Creating {name}…");
                    operate(api, tx, move |api| {
                        let file = api.create(&folder, &name, dir)?;
                        let kind = if dir { "folder" } else { "file" };
                        Ok((format!("Created {kind} {}", file.name), Some(file.id)))
                    });
                }
                Prompt::Rename(file) => {
                    if text.trim().is_empty() || text.trim() == file.name {
                        return;
                    }
                    view.busy += 1;
                    view.status = format!("Renaming {}…", file.name);
                    operate(api, tx, move |api| {
                        let renamed = api.rename(&file.id, &text)?;
                        Ok((
                            format!("Renamed {} to {}", file.name, renamed.name),
                            Some(renamed.id),
                        ))
                    });
                }
                Prompt::Upload => {
                    if text.trim().is_empty() {
                        return;
                    }
                    let path = local_path(&text);
                    view.busy += 1;
                    view.status = format!("Uploading {}…", path.display());
                    operate_with(api, tx, move |api, tx| {
                        let metadata = std::fs::metadata(&path)
                            .with_context(|| format!("cannot open {}", path.display()))?;
                        if metadata.is_dir() {
                            let result =
                                api.upload_dir(&path, &folder, None, false, |progress| {
                                    let _ = tx.send(Message::Uploading(upload_status(&progress)));
                                })?;
                            Ok((result.summary(), Some(result.id)))
                        } else {
                            let label = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .map(str::to_owned)
                                .unwrap_or_else(|| path.display().to_string());
                            let file = api.upload_with(&path, &folder, None, |done, total| {
                                let percent = upload::percent(done, total);
                                let _ = tx.send(Message::Uploading(format!(
                                    "Uploading {label} · {percent}%"
                                )));
                            })?;
                            Ok((format!("Uploaded {}", file.name), Some(file.id)))
                        }
                    });
                }
                Prompt::Trash(_) => unreachable!(),
            }
        }
        _ => {
            if input.edit(key) && matches!(input.prompt, Prompt::Filter) {
                let text = input.text.clone();
                view.set_filter(text);
            }
        }
    }
}
fn paste(view: &mut View, api: &Api, tx: &Sender<Message>) {
    let Some(clipboard) = view.clipboard.take_if(|c| c.cut) else {
        let Some(clipboard) = &view.clipboard else {
            view.status = "Nothing to paste: y copies, x cuts".into();
            return;
        };
        // Drive copies files only; folders would need a recursive copy.
        let files: Vec<File> = clipboard
            .files
            .iter()
            .filter(|f| !f.folder())
            .cloned()
            .collect();
        let folders = clipboard.files.len() - files.len();
        if files.is_empty() {
            view.status = "Drive cannot copy folders; x then p moves them".into();
            return;
        }
        let folder = view.folder().to_owned();
        view.busy += 1;
        view.status = format!("Copying {}…", items(files.len()));
        operate(api, tx, move |api| {
            let (mut status, select) =
                each(&files, "copied", |f| api.copy(&f.id, &folder).map(|_| true))?;
            if folders > 0 {
                status += &format!("; folders cannot be copied ({folders} left out)");
            }
            Ok((status, select))
        });
        return;
    };
    let folder = view.folder().to_owned();
    // A folder cannot move into itself.
    if clipboard.files.iter().any(|f| f.id == folder) {
        view.status = "Cannot move a folder into itself".into();
        view.clipboard = Some(clipboard);
        return;
    }
    view.busy += 1;
    view.status = format!("Moving {}…", items(clipboard.files.len()));
    operate(api, tx, move |api| {
        each(&clipboard.files, "moved here", |f| {
            api.move_to(&f.id, &folder)
        })
    });
}
fn event_loop(terminal: &mut ratatui::DefaultTerminal, api: Api, options: Options) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let db = Store::open(&api.db)?;
    let initial = api.cached(&options.folder)?;
    let folder_name = if options.folder == "root" {
        "My Drive".to_owned()
    } else {
        api.metadata(&options.folder)?.name
    };
    let mut view = View::new(
        (options.folder.clone(), folder_name),
        initial,
        options.connections,
    );
    view.account = api.account.clone();
    fetch(&api, &tx, options.folder.clone(), false);
    // Selected after the next listing of the current folder arrives.
    let mut select: Option<String> = None;
    let mut changed = Instant::now();
    let result = (|| -> Result<()> {
        loop {
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Folder(id, result) if view.folder() == id => {
                        view.loading = false;
                        match result {
                            Ok(files) => {
                                view.set_entries(files, select.take());
                                if view.stale {
                                    view.status.clear();
                                    view.stale = false;
                                }
                                changed = Instant::now();
                            }
                            Err(e) => {
                                view.status = e;
                                view.stale = true;
                            }
                        }
                    }
                    Message::Preview(id, result) if view.selected().is_some_and(|f| f.id == id) => {
                        match result {
                            Ok(files) => view.preview = files,
                            Err(e) => view.status = e,
                        }
                    }
                    Message::Listing(name, count) => {
                        view.status = format!("Listing {name}… {count} files");
                    }
                    Message::Listed(name, result) => {
                        view.listing -= 1;
                        view.status = match result {
                            Ok(files) if files.is_empty() => format!("{name} has no files"),
                            Ok(files) => {
                                let (added, skipped) = view.enqueue(files);
                                format!("{name}: {}", queued(&added, skipped))
                            }
                            Err(e) => format!("Could not list {name}: {e}"),
                        };
                    }
                    Message::Uploading(text) => view.status = text,
                    Message::Operated(result) => {
                        view.busy -= 1;
                        view.status = match result {
                            Ok((status, id)) => {
                                select = id;
                                status
                            }
                            Err(e) => e,
                        };
                        view.loading = true;
                        // Cached previews may now be out of date.
                        view.preview_id = None;
                        fetch(&api, &tx, view.folder().to_owned(), false);
                    }
                    Message::Finished(id, result) => {
                        if let Some(active) = view.active.take_if(|a| a.id == id) {
                            let _ = active.worker.join();
                        }
                        if let Some(job) = view.job_mut(&id) {
                            let name = job.file.name.clone();
                            job.state = match result {
                                Ok(()) => State::Done,
                                Err(e) => State::Failed(e),
                            };
                            view.status = match &job.state {
                                State::Failed(e) => format!("{name}: {e}"),
                                _ => format!("Saved {name}"),
                            };
                        }
                        if !view.jobs.iter().any(|j| j.state == State::Queued) {
                            view.status = format!("Finished · {}", view.summary());
                        }
                    }
                    _ => {}
                }
            }
            if let Some(active) = &mut view.active {
                while let Ok(stats) = active.rx.try_recv() {
                    active.stats = stats;
                }
            }
            if view.active.is_none() {
                if view.quitting {
                    break;
                }
                if let Some(job) = view.jobs.iter_mut().find(|j| j.state == State::Queued) {
                    job.state = State::Active;
                    view.active = Some(start(&api, &options, &tx, job, view.connections));
                }
            }
            if changed.elapsed() > Duration::from_millis(200)
                && !view.loading
                && let Some(file) = view.selected().filter(|f| f.folder())
            {
                let id = file.id.clone();
                if view.preview_id.as_deref() != Some(&id) {
                    view.preview = api.cached(&id).unwrap_or_default();
                    view.preview_id = Some(id.clone());
                    fetch(&api, &tx, id, true);
                }
            }
            terminal.draw(|frame| view.draw(frame))?;
            if !event::poll(Duration::from_millis(80))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press || view.quitting {
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            if view.help {
                view.help = false;
                continue;
            }
            if view.input.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
                prompt_key(&mut view, &api, &tx, key);
                changed = Instant::now();
                continue;
            }
            let before = view.selection.selected();
            match key.code {
                KeyCode::Char('q') => view.quitting = true,
                KeyCode::Char('c') if ctrl => view.quitting = true,
                KeyCode::Char('c') => {
                    if let Some(active) = &view.active {
                        active.cancel.store(true, Ordering::Relaxed);
                        view.status = "Cancelling…".into();
                    }
                }
                KeyCode::Char('?') => view.help = true,
                KeyCode::Down | KeyCode::Char('j') => view.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => view.move_by(-1),
                KeyCode::Char('d') if ctrl => view.move_by(view.page as isize / 2),
                KeyCode::Char('u') if ctrl => view.move_by(-(view.page as isize) / 2),
                KeyCode::PageDown => view.move_by(view.page as isize),
                KeyCode::PageUp => view.move_by(-(view.page as isize)),
                KeyCode::Home | KeyCode::Char('g') => view.move_to(0),
                KeyCode::End | KeyCode::Char('G') => view.move_to(isize::MAX),
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    view.connections = (view.connections + 1).min(16);
                    db.set("connections", &view.connections.to_string())?;
                }
                KeyCode::Char('-') => {
                    view.connections = view.connections.saturating_sub(1).max(1);
                    db.set("connections", &view.connections.to_string())?;
                }
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    if let Some(file) = view.selected().filter(|f| f.folder()).cloned() {
                        let files = api.cached(&file.id).unwrap_or_default();
                        let id = file.id.clone();
                        view.enter(file, files);
                        fetch(&api, &tx, id, false);
                    }
                }
                KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                    if view.path.len() > 1 {
                        let id = view.path[view.path.len() - 2].0.clone();
                        let files = api.cached(&id).unwrap_or_default();
                        let parent = view
                            .path
                            .len()
                            .checked_sub(3)
                            .map(|i| api.cached(&view.path[i].0).unwrap_or_default())
                            .unwrap_or_default();
                        view.leave(files, parent);
                        fetch(&api, &tx, id, false);
                    }
                }
                KeyCode::Char('r') if ctrl => {
                    view.loading = true;
                    fetch(&api, &tx, view.folder().to_owned(), false);
                }
                KeyCode::F(5) => {
                    view.loading = true;
                    fetch(&api, &tx, view.folder().to_owned(), false);
                }
                KeyCode::Char(' ') => view.toggle_mark(),
                KeyCode::Char('a') if ctrl => view.toggle_all(),
                KeyCode::Esc => view.escape(),
                KeyCode::Char('/') => {
                    view.input = Some(Input::new(Prompt::Filter, view.filter.clone()));
                }
                KeyCode::Char('a') => view.input = Some(Input::new(Prompt::Create, String::new())),
                KeyCode::Char('u') => view.input = Some(Input::new(Prompt::Upload, String::new())),
                KeyCode::Char('r') => {
                    if let Some(file) = view.selected().cloned() {
                        let mut input = Input::new(Prompt::Rename(file.clone()), file.name.clone());
                        // Start before the extension, as most renames keep it.
                        if let Some((stem, _)) = file.name.rsplit_once('.')
                            && !file.folder()
                            && !stem.is_empty()
                        {
                            input.cursor = stem.chars().count();
                        }
                        view.input = Some(input);
                    }
                }
                KeyCode::Char('D') | KeyCode::Delete => {
                    let files = if view.marked.is_empty() {
                        view.selected().cloned().into_iter().collect()
                    } else {
                        view.marked.clone()
                    };
                    if !files.is_empty() {
                        view.input = Some(Input::new(Prompt::Trash(files), String::new()));
                    }
                }
                KeyCode::Char('y') => view.yank(false),
                KeyCode::Char('x') => view.yank(true),
                KeyCode::Char('p') => paste(&mut view, &api, &tx),
                KeyCode::Char('d') => {
                    let picked = view.take_picked();
                    let (folders, files): (Vec<_>, Vec<_>) =
                        picked.into_iter().partition(|f| f.folder());
                    if !files.is_empty() || folders.is_empty() {
                        let files = files
                            .into_iter()
                            .map(|f| (f, options.output.clone()))
                            .collect();
                        let (added, skipped) = view.enqueue(files);
                        view.status = queued(&added, skipped);
                    }
                    for folder in folders {
                        view.status = format!("Listing {}…", folder.name);
                        view.listing += 1;
                        download_folder(&api, &tx, folder, &options.output);
                    }
                }
                KeyCode::Char('R') => view.retry_failed(),
                KeyCode::Char('X') => view.clear_queue(),
                _ => {}
            }
            if view.quitting {
                view.jobs.retain(|j| j.state != State::Queued);
                view.status = "Stopping download…".into();
                if let Some(active) = &view.active {
                    active.cancel.store(true, Ordering::Relaxed);
                }
            }
            if view.selection.selected() != before {
                changed = Instant::now();
            }
        }
        Ok(())
    })();
    // Always stop/join the native worker before restoring the terminal, even on I/O failure.
    if let Some(active) = view.active.take() {
        active.cancel.store(true, Ordering::Relaxed);
        let _ = active.worker.join();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(id: &str, name: &str, mime_type: &str) -> File {
        File {
            id: id.into(),
            name: name.into(),
            mime_type: mime_type.into(),
            size: Some("1048576".into()),
            modified_time: None,
            md5_checksum: None,
            resource_key: None,
        }
    }
    const DIR: &str = "application/vnd.google-apps.folder";
    fn render(view: &mut View, w: u16, h: u16) -> Vec<String> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| view.draw(f)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn layout_has_browser_above_downloads() {
        let folder = file("folder", "Documents", DIR);
        let mut view = View::new(("root".into(), "My Drive".into()), vec![folder], 8);
        view.loading = false;
        for (w, h) in [(120, 30), (100, 30), (40, 15)] {
            let rows = render(&mut view, w, h);
            let files = rows.iter().position(|s| s.contains("Documents")).unwrap();
            let downloads = rows.iter().position(|s| s.contains("Downloads")).unwrap();
            assert!(downloads > files);
            assert!(rows[0].contains("My Drive"));
            assert!(rows.last().unwrap().contains("NORMAL"));
        }
        view.help = true;
        let rows = render(&mut view, 100, 30);
        assert!(rows.iter().any(|s| s.contains("Move to the trash")));
        assert!(rows.iter().any(|s| s.contains("Upload a file or folder")));
    }
    #[test]
    fn upload_prompt_accepts_files_and_folders() {
        let mut view = View::new(("root".into(), "My Drive".into()), vec![], 8);
        view.input = Some(Input::new(Prompt::Upload, String::new()));
        let rows = render(&mut view, 100, 30);
        assert!(rows.iter().any(|s| s.contains("local file or folder path")));
    }
    #[test]
    fn upload_progress_reports_files() {
        let progress = TreeProgress {
            path: "a/b.txt".into(),
            files_done: 2,
            files_total: 10,
            bytes_done: 3,
            bytes_total: 9,
            file_done: 1,
            file_total: 4,
        };
        let status = upload_status(&progress);
        assert!(status.contains("a/b.txt"));
        assert!(status.contains("2/10 files"));
        assert!(status.contains("33%"));
        let empty = TreeProgress {
            path: "empty".into(),
            files_done: 1,
            files_total: 1,
            bytes_done: 0,
            bytes_total: 0,
            file_done: 0,
            file_total: 0,
        };
        assert!(upload_status(&empty).contains("100%"));
    }
    #[test]
    fn marked_files_are_queued_together() {
        let files = vec![
            file("a", "a.bin", "application/octet-stream"),
            file("dir", "dir", DIR),
            file("b", "b.bin", "application/octet-stream"),
            file("form", "Form", "application/vnd.google-apps.form"),
        ];
        let mut view = View::new(("root".into(), "My Drive".into()), files, 8);
        view.toggle_all();
        assert_eq!(view.marked.len(), 4);
        view.toggle_all();
        assert!(view.marked.is_empty());
        view.toggle_all();
        let marked = std::mem::take(&mut view.marked);
        let (added, skipped) =
            view.enqueue(marked.into_iter().map(|f| (f, "out".into())).collect());
        assert_eq!(queued(&added, skipped), "Queued 2 files, skipped 2");
        let (added, skipped) = view.enqueue(vec![(view.files[0].clone(), "out".into())]);
        assert_eq!(
            (added.len(), skipped),
            (0, 1),
            "queued files are not added twice"
        );
        assert_eq!(view.jobs.len(), 2);
        view.jobs[0].state = State::Failed("boom".into());
        view.jobs[1].state = State::Done;
        assert!(view.summary().starts_with("1/2 done · 1 failed"));
        let rows = render(&mut view, 100, 30);
        assert!(rows.iter().any(|s| s.contains("✗ a.bin  boom")));
        assert!(rows.iter().any(|s| s.contains("✓ b.bin")));
        view.retry_failed();
        assert_eq!(view.jobs.last().unwrap().file.id, "a");
        assert_eq!(view.jobs.last().unwrap().state, State::Queued);
        view.clear_queue();
        assert_eq!(view.jobs.len(), 1);
    }
    #[test]
    fn duplicate_names_get_distinct_outputs() {
        let mut view = View::new(("root".into(), "My Drive".into()), vec![], 8);
        view.enqueue(
            [
                file("1", "report.pdf", "application/pdf"),
                file("2", "report.pdf", "application/pdf"),
                file("3", "report.pdf", "application/pdf"),
                file("4", "notes", "application/vnd.google-apps.document"),
                file("5", "notes.pdf", "application/pdf"),
            ]
            .into_iter()
            .map(|f| (f, "out".into()))
            .collect(),
        );
        let outputs: Vec<_> = view.jobs.iter().map(|j| j.output.clone()).collect();
        assert_eq!(
            outputs,
            [
                "report.pdf",
                "report (2).pdf",
                "report (3).pdf",
                "notes.pdf",
                "notes (2).pdf"
            ]
            .map(|name| Path::new("out").join(name))
        );
    }
    #[test]
    fn folders_are_walked_recursively() {
        let tree = |id: &str| -> Result<Vec<File>> {
            Ok(match id {
                "root" => vec![
                    file("sub", "Sub/Dir", DIR),
                    file("a", "a.bin", "application/octet-stream"),
                ],
                // A folder reachable twice is listed once.
                "sub" => vec![
                    file("deep", "Deep", DIR),
                    file("again", "Deep", DIR),
                    file("b", "b.bin", "application/octet-stream"),
                ],
                "deep" | "again" => vec![
                    file("c", "c.bin", "application/octet-stream"),
                    file("sub", "Sub/Dir", DIR),
                ],
                _ => anyhow::bail!("unknown folder {id}"),
            })
        };
        let counts = std::cell::RefCell::new(vec![]);
        let files = walk(tree, "root", |n| counts.borrow_mut().push(n)).unwrap();
        let found: Vec<_> = files
            .iter()
            .map(|(f, d)| (f.id.as_str(), d.clone()))
            .collect();
        let sub = PathBuf::from("Sub_Dir");
        assert_eq!(
            found,
            [
                ("a", PathBuf::new()),
                ("b", sub.clone()),
                ("c", sub.join("Deep")),
                ("c", sub.join("Deep")),
            ]
        );
        assert_eq!(counts.into_inner(), [1, 2, 3, 4]);
        assert!(walk(|_| anyhow::bail!("offline"), "root", |_| {}).is_err());
        // The same file found twice is queued once.
        let mut view = View::new(("root".into(), "My Drive".into()), vec![], 8);
        let (added, skipped) = view.enqueue(files);
        assert_eq!((added.len(), skipped), (3, 1));
    }
    #[test]
    fn filter_keeps_selection_and_navigation_restores_it() {
        let files = vec![
            file("p", "Photos", DIR),
            file("r", "report.pdf", "application/pdf"),
            file("n", "notes.txt", "text/plain"),
        ];
        let mut view = View::new(("root".into(), "My Drive".into()), files.clone(), 8);
        view.set_filter("RE".into());
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.selected().unwrap().id, "r");
        view.escape();
        assert_eq!(view.files.len(), 3);
        // A refresh keeps the selected file even when its position changes.
        view.move_to(2);
        let mut reordered = files.clone();
        reordered.reverse();
        view.set_entries(reordered, None);
        assert_eq!(view.selected().unwrap().id, "n");
        view.move_to(2);
        view.enter(files[0].clone(), vec![file("x", "x.jpg", "image/jpeg")]);
        assert_eq!(view.parent.len(), 3);
        assert_eq!(view.folder(), "p");
        let rows = render(&mut view, 120, 30);
        assert!(rows[0].contains("My Drive") && rows[0].contains("Photos"));
        view.leave(files, vec![]);
        assert_eq!(view.selected().unwrap().id, "p");
    }
    #[test]
    fn yank_uses_marks_and_escape_steps_back() {
        let files = vec![
            file("a", "a.bin", "application/octet-stream"),
            file("b", "b.bin", "application/octet-stream"),
        ];
        let mut view = View::new(("root".into(), "My Drive".into()), files, 8);
        view.yank(false);
        assert_eq!(view.clipboard.as_ref().unwrap().files[0].id, "a");
        view.toggle_all();
        view.yank(true);
        let clipboard = view.clipboard.as_ref().unwrap();
        assert!(clipboard.cut && clipboard.files.len() == 2);
        assert!(view.marked.is_empty());
        let rows = render(&mut view, 100, 30);
        assert!(rows.last().unwrap().contains("2 cut"));
        view.escape();
        assert!(view.clipboard.is_none());
    }
    #[test]
    fn input_edits_by_character() {
        let mut input = Input::new(Prompt::Create, "héllo.txt".into());
        input.cursor = 5;
        input.edit(key(KeyCode::Char('!')));
        assert_eq!(input.text, "héllo!.txt");
        input.edit(key(KeyCode::Backspace));
        input.edit(key(KeyCode::Backspace));
        assert_eq!(input.text, "héll.txt");
        input.edit(key(KeyCode::Home));
        input.edit(key(KeyCode::Delete));
        assert_eq!(input.text, "éll.txt");
        assert!(!input.edit(key(KeyCode::Enter)));
        input.edit(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(input.text.is_empty() && input.cursor == 0);
    }
    #[test]
    fn local_paths_accept_terminal_drops() {
        assert_eq!(local_path(" 'a b.zip' "), PathBuf::from("a b.zip"));
        assert_eq!(local_path("a\\ b.zip"), PathBuf::from("a b.zip"));
        assert_eq!(local_path("~/x"), dirs::home_dir().unwrap().join("x"));
    }
    #[test]
    fn batch_operations_report_progress() {
        let files = [
            file("a", "a", "text/plain"),
            file("b", "b", "text/plain"),
            file("c", "c", "text/plain"),
        ];
        let (status, select) = each(&files, "moved here", |f| Ok(f.id != "b")).unwrap();
        assert_eq!(status, "2 items moved here, skipped 1");
        assert_eq!(select.as_deref(), Some("a"));
        let error = each(&files, "copied", |f| {
            if f.id == "c" {
                anyhow::bail!("quota")
            }
            Ok(true)
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "2 items copied; c: quota");
    }
}
