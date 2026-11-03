//! Rendering for `browse`: a yazi-style column layout over a download panel.
use super::{Clipboard, Input, Job, Prompt, State, View, bytes, duration, items};
use crate::api::File;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, LineGauge, List, ListItem, ListState, Padding,
        Paragraph, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ACCENT: Color = Color::Blue;
const DIM: Color = Color::DarkGray;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The colour and kind shown for a file, chosen from its MIME type.
fn kind(file: &File) -> (Color, &'static str) {
    let mime = file.mime_type.as_str();
    if file.folder() {
        return (ACCENT, "Folder");
    }
    if let Some(app) = mime.strip_prefix("application/vnd.google-apps.") {
        return match app {
            "document" => (Color::Green, "Google Doc"),
            "spreadsheet" => (Color::Green, "Google Sheet"),
            "presentation" => (Color::Green, "Google Slides"),
            "drawing" => (Color::Green, "Google Drawing"),
            "shortcut" => (DIM, "Shortcut"),
            _ => (DIM, "Google item"),
        };
    }
    match mime.split('/').next().unwrap_or_default() {
        "image" => (Color::Yellow, "Image"),
        "video" => (Color::Magenta, "Video"),
        "audio" => (Color::Cyan, "Audio"),
        "text" => (Color::Reset, "Text"),
        _ if mime == "application/pdf" => (Color::LightRed, "PDF"),
        _ if [
            "zip", "x-tar", "gzip", "x-7z", "x-rar", "x-bzip", "x-xz", "zstd",
        ]
        .iter()
        .any(|a| mime.ends_with(a)) =>
        {
            (Color::Red, "Archive")
        }
        _ => (Color::Reset, "File"),
    }
}
fn size(file: &File) -> Option<u64> {
    file.size.as_ref().and_then(|s| s.parse().ok())
}
/// Cuts `text` to `width` columns, ending with … when shortened.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    if width > 0 {
        out.push('…');
    }
    out
}
/// Drive times are RFC 3339; show them as `YYYY-MM-DD HH:MM` (UTC).
fn time(rfc3339: &str) -> String {
    rfc3339.get(..16).unwrap_or(rfc3339).replace('T', " ")
}
fn bordered() -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
}
/// One file row, padded to `width`: marker, state, name and size.
struct Row<'a> {
    file: &'a File,
    selected: bool,
    /// Left marker colour: marked or in the clipboard.
    marker: Option<Color>,
    state: Option<Span<'static>>,
    size: bool,
}
impl Row<'_> {
    fn line(self, width: usize) -> Line<'static> {
        let (color, _) = kind(self.file);
        let mut name = self.file.name.clone();
        if self.file.folder() {
            name.push('/');
        }
        let size = if self.size {
            size(self.file).map(bytes).unwrap_or_default()
        } else {
            String::new()
        };
        let state = self.state.unwrap_or_else(|| " ".into());
        // marker(1) state(1) space(1) name … space size space
        let room = width.saturating_sub(3 + size.width() + 2);
        let name = fit(&name, room);
        let pad = room.saturating_sub(name.width());
        let marker = Span::styled(
            "▎",
            Style::default().fg(self.marker.unwrap_or(Color::Reset)),
        );
        let marker = if self.marker.is_some() {
            marker
        } else {
            " ".into()
        };
        let mut style = Style::default().fg(color);
        if self.file.folder() {
            style = style.bold();
        }
        if !self.file.folder() && !self.file.downloadable() {
            style = style.italic();
        }
        let mut spans = vec![
            marker,
            state,
            " ".into(),
            Span::styled(name, style),
            " ".repeat(pad + 1).into(),
            Span::styled(size, Style::default().fg(DIM)),
            " ".into(),
        ];
        if self.selected {
            // The hovered row is a solid bar in the file's colour, as in yazi.
            let bar = match color {
                Color::Reset | Color::DarkGray => Color::Gray,
                c => c,
            };
            let base = Style::default().fg(Color::Black).bg(bar).bold();
            for span in &mut spans[2..] {
                span.style = base;
            }
        }
        Line::from(spans)
    }
}
impl View {
    fn marker(&self, file: &File) -> Option<Color> {
        if self.marked(file) {
            return Some(Color::Yellow);
        }
        match &self.clipboard {
            Some(Clipboard { files, cut }) if files.iter().any(|f| f.id == file.id) => {
                Some(if *cut { Color::Red } else { Color::Green })
            }
            _ => None,
        }
    }
    fn state(&self, file: &File) -> Option<Span<'static>> {
        Some(match self.job(&file.id)?.state {
            State::Queued => "◦".gray(),
            State::Active => "↓".cyan().bold(),
            State::Done => "✓".green(),
            State::Failed(_) => "✗".red(),
        })
    }
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let waiting = self.jobs.len() - usize::from(self.active.is_some());
        let downloads = if self.jobs.is_empty() {
            3
        } else {
            5 + waiting.min(6) as u16
        };
        let [header, browser, progress, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(downloads),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        self.draw_header(frame, header);
        self.draw_browser(frame, browser);
        self.draw_downloads(frame, progress);
        self.draw_status(frame, status);
        if let Some(input) = &self.input {
            draw_input(frame, input);
        }
        if self.help {
            draw_help(frame);
        }
    }
    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![" gdirect ".fg(Color::Black).bg(ACCENT).bold(), " ".into()];
        let last = self.path.len() - 1;
        for (i, (_, name)) in self.path.iter().enumerate() {
            if i > 0 {
                spans.push(" › ".fg(DIM));
            }
            spans.push(if i == last {
                name.clone().fg(ACCENT).bold()
            } else {
                name.clone().into()
            });
        }
        if !self.filter.is_empty() {
            spans.push(format!("  /{}", self.filter).yellow());
        }
        let breadcrumb = Line::from(spans);
        // The account sits on the right while it fits beside the breadcrumb.
        if let Some(account) = &self.account {
            let room = (area.width as usize).saturating_sub(breadcrumb.width() + 2);
            if account.width() < room {
                frame.render_widget(
                    Paragraph::new(format!("{account} ").fg(DIM)).alignment(Alignment::Right),
                    area,
                );
            }
        }
        frame.render_widget(Paragraph::new(breadcrumb), area);
    }
    fn draw_browser(&mut self, frame: &mut Frame, area: Rect) {
        let mut title = vec![" ".into()];
        title.push(if self.loading {
            let frame = (self.started.elapsed().as_millis() / 80) as usize;
            format!("{} refreshing", SPINNER[frame % SPINNER.len()]).cyan()
        } else if self.filter.is_empty() {
            items(self.files.len()).into()
        } else {
            format!("{} of {}", self.files.len(), items(self.entries.len())).into()
        });
        title.push(" ".into());
        let block = bordered().title(Line::from(title));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        // Columns fold away on narrow terminals: parent first, then the preview.
        let parent = self.path.len() > 1 && area.width >= 90;
        let preview = area.width >= 50;
        let [left, middle, right] = Layout::horizontal(match (parent, preview) {
            (true, _) => [
                Constraint::Ratio(1, 8),
                Constraint::Ratio(4, 8),
                Constraint::Ratio(3, 8),
            ],
            (false, true) => [
                Constraint::Length(0),
                Constraint::Ratio(5, 9),
                Constraint::Ratio(4, 9),
            ],
            (false, false) => [
                Constraint::Length(0),
                Constraint::Min(0),
                Constraint::Length(0),
            ],
        })
        .areas(inner);
        if parent {
            self.draw_parent(frame, left);
        }
        self.draw_files(frame, middle);
        if preview {
            let block = Block::new()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(DIM));
            let area = block.inner(right);
            frame.render_widget(block, right);
            self.draw_preview(frame, area);
        }
    }
    fn draw_parent(&self, frame: &mut Frame, area: Rect) {
        let block = Block::new()
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(DIM));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let current = self.folder();
        let width = inner.width as usize;
        let rows: Vec<_> = self
            .parent
            .iter()
            .map(|file| {
                ListItem::new(
                    Row {
                        file,
                        selected: file.id == current,
                        marker: None,
                        state: None,
                        size: false,
                    }
                    .line(width),
                )
            })
            .collect();
        let mut state =
            ListState::default().with_selected(self.parent.iter().position(|f| f.id == current));
        frame.render_stateful_widget(List::new(rows), inner, &mut state);
    }
    fn draw_files(&mut self, frame: &mut Frame, area: Rect) {
        self.page = area.height.max(1) as usize;
        if self.files.is_empty() {
            let text = if self.loading {
                "Loading Drive…"
            } else if self.filter.is_empty() {
                "This folder is empty · a creates, u uploads"
            } else {
                "Nothing matches the filter · Esc clears it"
            };
            frame.render_widget(Paragraph::new(format!("  {text}")).fg(DIM), area);
            return;
        }
        let width = area.width as usize;
        let selected = self.selection.selected();
        let rows: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, file)| {
                ListItem::new(
                    Row {
                        file,
                        selected: Some(i) == selected,
                        marker: self.marker(file),
                        state: self.state(file),
                        size: true,
                    }
                    .line(width),
                )
            })
            .collect();
        frame.render_stateful_widget(List::new(rows).scroll_padding(2), area, &mut self.selection);
    }
    fn draw_preview(&self, frame: &mut Frame, area: Rect) {
        let area = Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(1),
            ..area
        };
        let Some(file) = self.selected() else {
            return;
        };
        let (color, kind_name) = kind(file);
        if file.folder() {
            let [head, list] =
                Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(fit(&file.name, area.width as usize))
                        .fg(color)
                        .bold(),
                    Line::from("Folder · Enter opens · d downloads all").fg(DIM),
                ]),
                head,
            );
            let loaded = self.preview_id.as_deref() == Some(&file.id);
            if !loaded || self.preview.is_empty() {
                let text = if loaded { "Empty folder" } else { "Loading…" };
                frame.render_widget(Paragraph::new(text).fg(DIM), list);
                return;
            }
            let width = list.width as usize;
            let rows: Vec<_> = self
                .preview
                .iter()
                .map(|f| {
                    ListItem::new(
                        Row {
                            file: f,
                            selected: false,
                            marker: self.marker(f),
                            state: self.state(f),
                            size: true,
                        }
                        .line(width),
                    )
                })
                .collect();
            frame.render_widget(List::new(rows), list);
            return;
        }
        let field = |label: &str, value: String| {
            Line::from(vec![format!("{label:<9}").fg(DIM), value.into()])
        };
        let mut lines = vec![
            Line::from(file.name.clone()).fg(color).bold(),
            Line::from(kind_name).fg(color),
            Line::from(""),
            field("Size", size(file).map(bytes).unwrap_or_else(|| "—".into())),
            field(
                "Modified",
                file.modified_time
                    .as_deref()
                    .map(time)
                    .unwrap_or_else(|| "—".into()),
            ),
            field("Type", file.mime_type.clone()),
        ];
        if let Some(md5) = &file.md5_checksum {
            lines.push(field("MD5", md5.clone()));
        }
        lines.push(Line::from(""));
        lines.push(match self.job(&file.id) {
            _ if !file.downloadable() => {
                Line::from("Cannot be downloaded (shortcut, form, site or map)").fg(DIM)
            }
            Some(job) => match &job.state {
                State::Queued => Line::from("◦ Queued").gray(),
                State::Active => Line::from("↓ Downloading…").cyan(),
                State::Done => Line::from(format!("✓ Saved {}", job.output.display())).green(),
                State::Failed(e) => Line::from(format!("✗ {e}")).red(),
            },
            None if file.workspace() => Line::from("d exports it as PDF").fg(DIM),
            None => Line::from("d downloads · space marks").fg(DIM),
        });
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
    fn draw_downloads(&self, frame: &mut Frame, area: Rect) {
        let mut title = vec![" Downloads ".bold()];
        if !self.jobs.is_empty() {
            title.push(format!("· {} ", self.summary()).into());
        }
        if self.listing > 0 {
            title.push(format!("· listing {} folders ", self.listing).cyan());
        }
        let block = bordered()
            .title(Line::from(title))
            .padding(Padding::horizontal(1));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [name, gauge, details, queue] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(inner);
        if let Some((active, job)) = self
            .active
            .as_ref()
            .and_then(|a| Some((a, self.job(&a.id)?)))
        {
            let s = active.stats;
            let ratio = if s.total > 0 {
                (s.done as f64 / s.total as f64).clamp(0., 1.)
            } else {
                0.
            };
            let position = self.jobs.iter().position(|j| j.file.id == active.id);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    "↓ ".cyan().bold(),
                    job.file.name.clone().bold(),
                    format!("  [{}/{}]", position.map_or(0, |i| i + 1), self.jobs.len()).fg(DIM),
                ])),
                name,
            );
            frame.render_widget(
                LineGauge::default()
                    .filled_style(Style::default().fg(Color::Cyan))
                    .unfilled_style(Style::default().fg(DIM))
                    .filled_symbol(symbols::line::THICK_HORIZONTAL)
                    .unfilled_symbol(symbols::line::THICK_HORIZONTAL)
                    .ratio(ratio)
                    .label(if s.total > 0 {
                        format!("{:>5.1}% ", ratio * 100.)
                    } else {
                        "  …    ".into()
                    }),
                gauge,
            );
            let eta = s
                .total
                .saturating_sub(s.done)
                .checked_div(s.speed)
                .map(duration)
                .unwrap_or_else(|| "—".into());
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    format!("{} / {}", bytes(s.done), bytes(s.total)).into(),
                    "  ·  ".fg(DIM),
                    format!("{}/s", bytes(s.speed)).cyan(),
                    "  ·  ".fg(DIM),
                    format!("{} connections", s.connections).into(),
                    "  ·  ".fg(DIM),
                    format!("ETA {eta}").into(),
                ])),
                details,
            );
        } else {
            frame.render_widget(
                Paragraph::new(if self.jobs.is_empty() {
                    "Ready · d downloads the selection, space marks several, ? shows all keys"
                } else {
                    "Idle · d adds more, R retries failed downloads"
                })
                .fg(DIM),
                name,
            );
        }
        // Waiting work first, then failures, then the most recent successes.
        let mut others: Vec<&Job> = self
            .jobs
            .iter()
            .filter(|j| j.state == State::Queued)
            .collect();
        others.extend(
            self.jobs
                .iter()
                .filter(|j| matches!(j.state, State::Failed(_))),
        );
        others.extend(self.jobs.iter().rev().filter(|j| j.state == State::Done));
        // Without an active download the first three rows are free for the list too.
        let queue = if self.active.is_some() {
            queue
        } else {
            Rect {
                y: gauge.y,
                height: queue.height + gauge.height + details.height,
                ..queue
            }
        };
        let rows = queue.height as usize;
        let shown = if others.len() > rows {
            rows.saturating_sub(1)
        } else {
            rows
        };
        let mut lines: Vec<Line> = others
            .iter()
            .take(shown)
            .map(|job| {
                let size = job.size.map(bytes).unwrap_or_default();
                match &job.state {
                    State::Failed(e) => Line::from(vec![
                        "✗ ".red(),
                        job.file.name.clone().into(),
                        format!("  {e}").red(),
                    ]),
                    State::Done => Line::from(vec![
                        "✓ ".green(),
                        job.file.name.clone().gray(),
                        format!("  {size}").fg(DIM),
                    ]),
                    _ => Line::from(vec![
                        "◦ ".gray(),
                        job.file.name.clone().into(),
                        format!("  {size}").fg(DIM),
                    ]),
                }
            })
            .collect();
        if others.len() > shown {
            lines.push(Line::from(format!("  … {} more", others.len() - shown)).fg(DIM));
        }
        frame.render_widget(Paragraph::new(lines), queue);
    }
    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let (mode, color) = match self.input.as_ref().map(|i| &i.prompt) {
            None => (" NORMAL ", ACCENT),
            Some(Prompt::Filter) => (" FILTER ", Color::Yellow),
            Some(Prompt::Trash(_)) => (" CONFIRM ", Color::Red),
            Some(_) => (" INPUT ", Color::Magenta),
        };
        let position = match self.selection.selected() {
            Some(i) if !self.files.is_empty() => format!(" {}/{} ", i + 1, self.files.len()),
            _ => " 0/0 ".into(),
        };
        let mut left = vec![
            mode.fg(Color::Black).bg(color).bold(),
            position.fg(color).bg(Color::Reset),
        ];
        if !self.marked.is_empty() {
            left.push(format!(" ● {} marked ", self.marked.len()).yellow().bold());
        }
        match &self.clipboard {
            Some(Clipboard { files, cut: true }) => {
                left.push(format!(" ✂ {} cut ", files.len()).red())
            }
            Some(Clipboard { files, .. }) => {
                left.push(format!(" ⧉ {} copied ", files.len()).green())
            }
            None => {}
        }
        if self.busy > 0 {
            let frame = (self.started.elapsed().as_millis() / 80) as usize;
            left.push(format!(" {} ", SPINNER[frame % SPINNER.len()]).cyan());
        }
        left.push(format!(" {}", self.status).yellow());
        let right = Line::from(vec![
            format!("⇣ {} conn ", self.connections).fg(DIM),
            " ? ".fg(Color::Black).bg(DIM),
            " help ".fg(DIM),
        ]);
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(right.width() as u16)])
                .areas(area);
        frame.render_widget(Paragraph::new(Line::from(left)), left_area);
        frame.render_widget(
            Paragraph::new(right).alignment(Alignment::Right),
            right_area,
        );
    }
}
fn popup(frame: &mut Frame, width: u16, height: u16) -> Rect {
    let area = frame.area();
    let area = area.centered(
        Constraint::Length(width.min(area.width.saturating_sub(4))),
        Constraint::Length(height.min(area.height)),
    );
    frame.render_widget(Clear, area);
    area
}
fn draw_input(frame: &mut Frame, input: &Input) {
    if let Prompt::Trash(files) = &input.prompt {
        let shown = files.len().min(6);
        let area = popup(frame, 56, shown as u16 + 5 + u16::from(files.len() > shown));
        let block = bordered()
            .border_style(Style::default().fg(Color::Red))
            .title(
                format!(" Move {} to the trash? ", items(files.len()))
                    .red()
                    .bold(),
            );
        let width = block.inner(area).width as usize;
        let mut lines: Vec<Line> = files[..shown]
            .iter()
            .map(|f| Line::from(fit(&format!(" {}", f.name), width)).fg(kind(f).0))
            .collect();
        if files.len() > shown {
            lines.push(Line::from(format!(" … {} more", files.len() - shown)).fg(DIM));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(" Items can be restored from the Drive trash.").fg(DIM));
        lines.push(Line::from(vec![
            " y".red().bold(),
            " confirm   ".into(),
            "any key".bold(),
            " cancel".into(),
        ]));
        frame.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }
    let (title, hint) = match &input.prompt {
        Prompt::Create => (" Create ", "name/ creates a folder"),
        Prompt::Rename(_) => (" Rename ", "Enter renames · Esc cancels"),
        Prompt::Upload => (
            " Upload to this folder ",
            "local file or folder path · Enter uploads",
        ),
        Prompt::Filter => (" Filter ", "Enter keeps · Esc clears"),
        Prompt::Trash(_) => unreachable!(),
    };
    let area = popup(frame, 60, 3);
    let block = bordered()
        .border_style(Style::default().fg(ACCENT))
        .title(title.fg(ACCENT).bold())
        .title_bottom(Line::from(format!(" {hint} ")).fg(DIM).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Scroll so the cursor stays visible.
    let width = inner.width.saturating_sub(1) as usize;
    let before: String = input.text.chars().take(input.cursor).collect();
    let mut skip = 0;
    while before.chars().skip(skip).collect::<String>().width() > width {
        skip += 1;
    }
    let visible: String = input.text.chars().skip(skip).collect();
    let column = before.chars().skip(skip).collect::<String>().width() as u16;
    frame.render_widget(Paragraph::new(visible), inner);
    frame.set_cursor_position((inner.x + column, inner.y));
}
const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigate",
        &[
            ("j/k ↑/↓", "Move"),
            ("l/→ Enter", "Open folder"),
            ("h/← Bksp", "Parent folder"),
            ("g/G", "Top / bottom"),
            ("^d/^u PgDn", "Half page / page"),
            ("/", "Filter this folder"),
            ("^r F5", "Refresh"),
        ],
    ),
    (
        "Select",
        &[
            ("space", "Mark and move down"),
            ("^a", "Mark all shown"),
            ("esc", "Clear filter/marks/clipboard"),
        ],
    ),
    (
        "Files",
        &[
            ("a", "Create (name/ for a folder)"),
            ("r", "Rename"),
            ("y / x", "Copy / cut"),
            ("p", "Paste here"),
            ("D Del", "Move to the trash"),
            ("u", "Upload a file or folder here"),
        ],
    ),
    (
        "Downloads",
        &[
            ("d", "Download (folders too)"),
            ("+/-", "Connections"),
            ("c", "Cancel current"),
            ("R", "Retry failed"),
            ("X", "Clear the queue"),
            ("q", "Quit"),
        ],
    ),
];
fn draw_help(frame: &mut Frame) {
    let column = |groups: &[(&str, &[(&str, &str)])]| -> Vec<Line<'static>> {
        let mut lines = vec![];
        for (group, keys) in groups {
            if !lines.is_empty() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(group.to_string()).fg(ACCENT).bold());
            for (key, action) in *keys {
                lines.push(Line::from(vec![
                    format!(" {key:<11}").yellow(),
                    action.to_string().into(),
                ]));
            }
        }
        lines
    };
    let (left, right) = (column(&KEYS[..2]), column(&KEYS[2..]));
    let height = left.len().max(right.len()) as u16 + 3;
    let area = popup(frame, 84, height);
    let block = bordered()
        .border_style(Style::default().fg(ACCENT))
        .title(" Keys ".fg(ACCENT).bold())
        .title_bottom(Line::from(" any key closes ").fg(DIM).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = Rect {
        x: inner.x + 1,
        width: inner.width.saturating_sub(1),
        ..inner
    };
    let [a, b] = Layout::horizontal([Constraint::Ratio(1, 2); 2]).areas(inner);
    frame.render_widget(Paragraph::new(left), a);
    frame.render_widget(Paragraph::new(right), b);
}
