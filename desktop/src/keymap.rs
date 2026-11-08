//! Keyboard shortcuts and the application menus: every binding is one row of
//! [`key_bindings`], which Settings › Keyboard shortcuts lists and lets the
//! user change. Changes are stored as overrides of the defaults.
use crate::{i18n::t, settings::KeymapOverrides};
use gpui_kit::*;
use std::{
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

/// The text field's editing commands; the file list answers the clipboard
/// ones and Select All for its files when it has focus.
pub use gpui_kit::base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};

actions!(
    gdirect,
    [
        About,
        OpenSettings,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        NewFolder,
        UploadFiles,
        UploadFolder,
        CloseWindow,
        ShowFiles,
        ShowTransfers,
        ShowRoutes,
        ShowSettings,
        ViewList,
        ViewGrid,
        Refresh,
        Minimize,
        Zoom,
        ShowShortcuts,
        GoBack,
        GoForward,
        GoMyDrive,
        GoShared,
        GoStarred,
        GoRecent,
        GoTrash,
    ]
);

actions!(
    files,
    [
        Cancel,
        Open,
        Trash,
        Rename,
        Parent,
        FocusFilter,
        FilterFolder,
        CursorUp,
        CursorDown,
        CursorFirst,
        CursorLast,
        CursorLeft,
        CursorRight,
        ExtendUp,
        ExtendDown,
        CopyLink,
    ]
);

/// The window's key context, around every page.
pub const APP: &str = "Gdirect";
/// The Files page's key context, around the filter and the list.
pub const FILES: &str = "Files";

/// The platforms a binding applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    All,
    Mac,
    /// Windows and Linux.
    Other,
}

impl Os {
    /// Whether the binding applies on `os`.
    pub fn on(self, os: OperatingSystem) -> bool {
        match self {
            Os::All => true,
            Os::Mac => os == OperatingSystem::Mac,
            Os::Other => os != OperatingSystem::Mac,
        }
    }
}

/// One keyboard shortcut.
pub struct Shortcut {
    pub action: Box<dyn Action>,
    /// The default keystroke; `secondary-` is Command on macOS, Control elsewhere.
    pub keys: &'static str,
    /// What it does, in Chinese and English.
    pub label: (&'static str, &'static str),
    /// The key context it applies in; `None` is everywhere.
    pub context: Option<&'static str>,
    pub os: Os,
}

impl Shortcut {
    /// The binding's stable name in the stored overrides: its action and its
    /// default keystroke, which tell apart an action's several bindings.
    pub fn id(&self) -> String {
        format!("{} {}", self.action.name(), self.keys)
    }
}

/// Every shortcut the app binds, in the order a shortcuts page lists them.
pub fn key_bindings() -> Vec<Shortcut> {
    fn row(
        action: impl Action,
        keys: &'static str,
        zh: &'static str,
        en: &'static str,
        context: Option<&'static str>,
        os: Os,
    ) -> Shortcut {
        Shortcut {
            action: Box::new(action),
            keys,
            label: (zh, en),
            context,
            os,
        }
    }
    let files = Some(FILES);
    use Os::{All, Mac, Other};
    vec![
        // Application
        row(OpenSettings, "secondary-,", "设置", "Settings", None, All),
        row(Quit, "cmd-q", "退出 gdirect", "Quit gdirect", None, Mac),
        row(Quit, "ctrl-q", "退出 gdirect", "Quit gdirect", None, Other),
        row(Hide, "cmd-h", "隐藏 gdirect", "Hide gdirect", None, Mac),
        row(
            HideOthers,
            "cmd-alt-h",
            "隐藏其他",
            "Hide others",
            None,
            Mac,
        ),
        row(
            CloseWindow,
            "secondary-w",
            "关闭窗口",
            "Close window",
            None,
            All,
        ),
        row(Minimize, "cmd-m", "最小化", "Minimize", None, Mac),
        row(
            ShowShortcuts,
            "secondary-/",
            "键盘快捷键",
            "Keyboard shortcuts",
            None,
            All,
        ),
        // Files menu
        row(
            NewFolder,
            "secondary-shift-n",
            "新建文件夹",
            "New folder",
            None,
            All,
        ),
        row(
            UploadFiles,
            "secondary-u",
            "上传文件",
            "Upload files",
            None,
            All,
        ),
        row(
            UploadFolder,
            "secondary-shift-u",
            "上传文件夹",
            "Upload folder",
            None,
            All,
        ),
        // Pages
        row(ShowFiles, "secondary-1", "文件", "Files", None, All),
        row(ShowTransfers, "secondary-2", "传输", "Transfers", None, All),
        row(ShowRoutes, "secondary-3", "线路", "Routes", None, All),
        // Settings sits beside the pages too: the same action as ⌘, so the
        // shortcuts page lists both keys on its row (the menu shows ⌘,,
        // which is bound first).
        row(OpenSettings, "secondary-4", "设置", "Settings", None, All),
        row(Refresh, "secondary-r", "刷新", "Refresh", None, All),
        row(Refresh, "f5", "刷新", "Refresh", None, Other),
        row(FocusFilter, "secondary-f", "查找", "Find", None, All),
        row(
            FilterFolder,
            "secondary-alt-f",
            "筛选此文件夹",
            "Filter this folder",
            None,
            All,
        ),
        // The Go menu
        row(GoBack, "secondary-[", "后退", "Back", None, All),
        row(GoForward, "secondary-]", "前进", "Forward", None, All),
        row(
            GoMyDrive,
            "secondary-shift-d",
            "我的云端硬盘",
            "My Drive",
            None,
            All,
        ),
        row(
            GoShared,
            "secondary-shift-s",
            "与我共享",
            "Shared with me",
            None,
            All,
        ),
        row(GoRecent, "secondary-shift-r", "最近", "Recent", None, All),
        row(GoTrash, "secondary-shift-t", "回收站", "Trash", None, All),
        // The file list
        row(SelectAll, "secondary-a", "全选", "Select all", files, All),
        row(Copy, "secondary-c", "复制", "Copy", files, All),
        row(Cut, "secondary-x", "剪切", "Cut", files, All),
        row(Paste, "secondary-v", "粘贴", "Paste", files, All),
        row(
            Cancel,
            "escape",
            "取消选择或清除搜索",
            "Clear selection or search",
            files,
            All,
        ),
        row(Open, "enter", "打开或下载", "Open or download", files, All),
        row(
            Open,
            "secondary-o",
            "打开或下载",
            "Open or download",
            files,
            All,
        ),
        row(Trash, "delete", "移至回收站", "Move to trash", files, All),
        row(
            Trash,
            "secondary-backspace",
            "移至回收站",
            "Move to trash",
            files,
            All,
        ),
        row(Rename, "f2", "重命名", "Rename", files, All),
        row(
            CopyLink,
            "secondary-alt-c",
            "复制链接",
            "Copy link",
            files,
            All,
        ),
        row(
            Parent,
            "cmd-up",
            "上级文件夹",
            "Enclosing folder",
            files,
            Mac,
        ),
        row(
            Parent,
            "alt-up",
            "上级文件夹",
            "Enclosing folder",
            files,
            Other,
        ),
        row(
            Parent,
            "backspace",
            "上级文件夹",
            "Enclosing folder",
            files,
            All,
        ),
        row(Undo, "secondary-z", "撤销", "Undo", files, All),
        row(CursorUp, "up", "上一项", "Previous item", files, All),
        row(CursorDown, "down", "下一项", "Next item", files, All),
        row(CursorFirst, "home", "第一项", "First item", files, All),
        row(CursorLast, "end", "最后一项", "Last item", files, All),
        row(
            CursorLeft,
            "left",
            "左侧一项（网格）",
            "Item to the left (grid)",
            files,
            All,
        ),
        row(
            CursorRight,
            "right",
            "右侧一项（网格）",
            "Item to the right (grid)",
            files,
            All,
        ),
        row(
            ExtendUp,
            "shift-up",
            "向上扩展选择",
            "Extend selection up",
            files,
            All,
        ),
        row(
            ExtendDown,
            "shift-down",
            "向下扩展选择",
            "Extend selection down",
            files,
            All,
        ),
    ]
}

/// Where the shortcuts page lists a binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Global,
    Window,
    Files,
}

impl Section {
    pub const ALL: [Section; 3] = [Section::Global, Section::Window, Section::Files];

    pub fn label(self) -> &'static str {
        match self {
            Section::Global => t("全局", "Global"),
            Section::Window => t("窗口", "Window"),
            Section::Files => t("文件", "Files"),
        }
    }
}

/// One binding as it stands: its default and the keystroke it has now.
pub struct Binding {
    /// See [`Shortcut::id`].
    pub id: String,
    pub action: Box<dyn Action>,
    pub label: (&'static str, &'static str),
    pub context: Option<&'static str>,
    pub section: Section,
    /// The default keystroke, spelled for this platform (`cmd-,`).
    pub default: String,
    /// The keystroke now, spelled the same way; None when unset.
    pub keys: Option<String>,
}

impl Binding {
    pub fn label(&self) -> &'static str {
        t(self.label.0, self.label.1)
    }

    /// Whether the user changed it.
    pub fn changed(&self) -> bool {
        self.keys.as_deref() != Some(self.default.as_str())
    }
}

/// The host platform, as keystrokes are parsed for it.
pub fn host_os(cx: &App) -> OperatingSystem {
    cx.platform_info().os
}

/// A keystroke in one spelling for `os`: `secondary-` resolved, modifiers in
/// a fixed order, the key in lower case.
pub fn normalize(keys: &str, os: OperatingSystem) -> Option<String> {
    Keystroke::parse_with_os(keys, os)
        .ok()
        .filter(|keystroke| !keystroke.key.is_empty())
        .map(|keystroke| keystroke.unparse(os))
}

/// The key caps of a keystroke: macOS symbols (⌃⌥⇧⌘) on a Mac, words
/// (Ctrl, Shift) elsewhere, then the key.
pub fn caps(keys: &str, os: OperatingSystem) -> Vec<String> {
    let Ok(keystroke) = Keystroke::parse_with_os(keys, os) else {
        return vec![keys.to_owned()];
    };
    let m = keystroke.modifiers;
    let mac = os == OperatingSystem::Mac;
    let mut caps: Vec<String> = Vec::new();
    let mut push = |on: bool, cap: &str| {
        if on {
            caps.push(cap.to_owned());
        }
    };
    if mac {
        // Apple's order: Control, Option, Shift, Command.
        push(m.function, "fn");
        push(m.control, "⌃");
        push(m.alt, "⌥");
        push(m.shift, "⇧");
        push(m.platform, "⌘");
    } else {
        push(m.control, "Ctrl");
        push(m.alt, "Alt");
        push(m.shift, "Shift");
        push(
            m.platform,
            if os == OperatingSystem::Windows {
                "Win"
            } else {
                "Super"
            },
        );
        push(m.function, "Fn");
    }
    caps.push(key_cap(&keystroke.key, mac));
    caps
}

/// A keystroke as one string: `⇧⌘N` on a Mac, `Ctrl+Shift+N` elsewhere.
pub fn display(keys: &str, os: OperatingSystem) -> String {
    let caps = caps(keys, os);
    if os == OperatingSystem::Mac {
        caps.concat()
    } else {
        caps.join("+")
    }
}

fn key_cap(key: &str, mac: bool) -> String {
    let named = match (key, mac) {
        ("backspace", true) => "⌫",
        ("backspace", false) => "Backspace",
        ("delete", true) => "⌦",
        ("delete", false) => "Delete",
        ("enter", true) => "↩",
        ("enter", false) => "Enter",
        ("escape", _) => "Esc",
        ("tab", true) => "⇥",
        ("tab", false) => "Tab",
        ("space", _) => "Space",
        ("up", true) => "↑",
        ("down", true) => "↓",
        ("left", true) => "←",
        ("right", true) => "→",
        ("up", false) => "Up",
        ("down", false) => "Down",
        ("left", false) => "Left",
        ("right", false) => "Right",
        ("pageup", true) => "⇞",
        ("pagedown", true) => "⇟",
        ("pageup", false) => "PgUp",
        ("pagedown", false) => "PgDn",
        ("home", true) => "↖",
        ("end", true) => "↘",
        ("home", false) => "Home",
        ("end", false) => "End",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Every binding on `os`, with the user's overrides applied.
pub fn bindings(overrides: &KeymapOverrides, os: OperatingSystem) -> Vec<Binding> {
    key_bindings()
        .into_iter()
        .filter(|s| s.os.on(os))
        .map(|s| {
            let id = s.id();
            let default = normalize(s.keys, os).unwrap_or_else(|| s.keys.to_owned());
            let keys = match overrides.get(&id) {
                Some(keys) if keys.is_empty() => None,
                // One that no longer parses falls back to the default.
                Some(keys) => Some(normalize(keys, os).unwrap_or_else(|| default.clone())),
                None => Some(default.clone()),
            };
            let section = match (s.context, s.action.name()) {
                (Some(_), _) => Section::Files,
                (None, name)
                    if [CloseWindow.name(), Minimize.name(), Zoom.name()].contains(&name) =>
                {
                    Section::Window
                }
                (None, _) => Section::Global,
            };
            Binding {
                id,
                action: s.action,
                label: s.label,
                context: s.context,
                section,
                default,
                keys,
            }
        })
        .collect()
}

/// The bindings, other than `list[ix]`, that already answer `keys` where
/// `list[ix]` applies: in its own key context, or everywhere. A binding for
/// everywhere clashes with those of every context.
pub fn conflicts(list: &[Binding], ix: usize, keys: &str) -> Vec<usize> {
    let context = list[ix].context;
    list.iter()
        .enumerate()
        .filter(|&(j, b)| {
            j != ix
                && b.keys.as_deref() == Some(keys)
                && (b.context == context || b.context.is_none() || context.is_none())
        })
        .map(|(j, _)| j)
        .collect()
}

/// Gives the binding `id` the keystroke `keys` (None unsets it) in the
/// overrides; one back at its default is no override at all.
pub fn set(overrides: &mut KeymapOverrides, list: &[Binding], id: &str, keys: Option<String>) {
    let Some(binding) = list.iter().find(|b| b.id == id) else {
        return;
    };
    match keys {
        Some(keys) if keys == binding.default => {
            overrides.remove(id);
        }
        keys => {
            overrides.insert(id.to_owned(), keys.unwrap_or_default());
        }
    }
}

/// Whether a keystroke recorded for a binding in `context` may become one.
/// It needs a modifier other than Shift, so typing never triggers it; a
/// function key will do anywhere, and the named keys a list answers (Enter,
/// Delete, the arrows…) will do in a context such as the file list.
pub fn accepts(keystroke: &Keystroke, context: Option<&str>) -> bool {
    if keystroke.is_modifier_key() || keystroke.key.is_empty() {
        return false;
    }
    let m = keystroke.modifiers;
    if m.platform || m.control || m.alt || m.function {
        return true;
    }
    let key = keystroke.key.as_str();
    let function_key =
        key.len() >= 2 && key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit());
    let named = matches!(
        key,
        "enter"
            | "delete"
            | "backspace"
            | "up"
            | "down"
            | "left"
            | "right"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
    );
    function_key || (named && context.is_some())
}

/// The overrides in force and the key bindings installed from them.
#[derive(Default)]
struct Installed {
    overrides: KeymapOverrides,
    bindings: Vec<KeyBinding>,
}

impl Global for Installed {}

/// The overrides in force.
pub fn overrides(cx: &App) -> KeymapOverrides {
    cx.try_global::<Installed>()
        .map(|i| i.overrides.clone())
        .unwrap_or_default()
}

/// Puts `overrides` in force at once: the key bindings and the menus' keys.
pub fn apply(overrides: KeymapOverrides, cx: &mut App) {
    cx.default_global::<Installed>().overrides = overrides;
    bind(cx);
    set_menus(cx);
}

/// Registers [`key_bindings`] for this platform with the overrides in force,
/// replacing those registered before.
///
/// GPUI can only clear the whole keymap, which also holds the text fields',
/// dialogs' and menus' bindings that `gpui_kit::init` and
/// `gpui_kit_material::init` registered. So the keymap is read back first,
/// this module's previous bindings taken out of it one for one, and the rest
/// put back, in their order, before ours: everyone else's bindings survive,
/// whenever they were registered, and ours keep the precedence that being
/// added last gives them.
pub fn bind(cx: &mut App) {
    let previous = std::mem::take(&mut cx.default_global::<Installed>().bindings);
    let mut kept: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
    for old in &previous {
        if let Some(ix) = kept.iter().rposition(|b| same_binding(b, old)) {
            kept.remove(ix);
        }
    }
    let os = host_os(cx);
    let overrides = overrides(cx);
    let ours: Vec<KeyBinding> = bindings(&overrides, os)
        .into_iter()
        .filter_map(|b| {
            let keys = b.keys?;
            let context = b
                .context
                .and_then(|c| KeyBindingContextPredicate::parse(c).ok())
                .map(Rc::new);
            KeyBinding::load(&keys, b.action, context, false, None, cx).ok()
        })
        .collect();
    if !previous.is_empty() {
        cx.clear_key_bindings();
        cx.bind_keys(kept);
    }
    cx.bind_keys(ours.iter().cloned());
    cx.default_global::<Installed>().bindings = ours;
}

fn same_binding(a: &KeyBinding, b: &KeyBinding) -> bool {
    a.keystrokes() == b.keystrokes()
        && a.predicate() == b.predicate()
        && a.action().partial_eq(b.action())
}

/// Set by [`ShowShortcuts`]; the settings page takes it to open its
/// Keyboard shortcuts pane.
static OPEN_SHORTCUTS: AtomicBool = AtomicBool::new(false);

/// Whether Keyboard Shortcuts was chosen since the last call.
pub fn take_open_shortcuts() -> bool {
    OPEN_SHORTCUTS.swap(false, Ordering::Relaxed)
}

/// Application commands that need no window: quitting and hiding. Binds the
/// keys with the stored `overrides` and builds the menus.
pub fn install(overrides: KeymapOverrides, cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &CloseWindow, cx| {
        // The key's window is being updated while it dispatches the action.
        if let Some(window) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            });
        }
    });
    // Settings › Keyboard shortcuts: the window shows Settings, which then
    // opens the pane.
    cx.on_action(|_: &ShowShortcuts, cx| {
        OPEN_SHORTCUTS.store(true, Ordering::Relaxed);
        // A menu chosen while the app is inactive has no active window.
        if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    window.dispatch_action(Box::new(ShowSettings), cx)
                });
            });
        }
    });
    cx.set_global(Installed {
        overrides,
        bindings: Vec::new(),
    });
    bind(cx);
    set_menus(cx);
}

/// The menu bar, in the current language; set again after a language switch.
pub fn set_menus(cx: &mut App) {
    cx.set_menus([
        Menu::new("gdirect").items([
            MenuItem::action(t("关于 gdirect", "About gdirect"), About),
            MenuItem::action(t("设置…", "Settings…"), OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu(t("服务", "Services"), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(t("隐藏 gdirect", "Hide gdirect"), Hide),
            MenuItem::action(t("隐藏其他", "Hide Others"), HideOthers),
            MenuItem::action(t("全部显示", "Show All"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t("退出 gdirect", "Quit gdirect"), Quit),
        ]),
        Menu::new(t("文件", "File")).items([
            MenuItem::action(t("新建文件夹", "New Folder"), NewFolder),
            MenuItem::action(t("上传文件…", "Upload Files…"), UploadFiles),
            MenuItem::action(t("上传文件夹…", "Upload Folder…"), UploadFolder),
            MenuItem::separator(),
            MenuItem::action(t("打开", "Open"), Open),
            MenuItem::action(t("重命名…", "Rename…"), Rename),
            MenuItem::action(t("复制链接", "Copy Link"), CopyLink),
            MenuItem::action(t("移至回收站", "Move to Trash"), Trash),
            MenuItem::separator(),
            MenuItem::action(t("关闭窗口", "Close Window"), CloseWindow),
        ]),
        Menu::new(t("编辑", "Edit")).items([
            MenuItem::os_action(t("撤销", "Undo"), Undo, OsAction::Undo),
            MenuItem::os_action(t("重做", "Redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t("剪切", "Cut"), Cut, OsAction::Cut),
            MenuItem::os_action(t("复制", "Copy"), Copy, OsAction::Copy),
            MenuItem::os_action(t("粘贴", "Paste"), Paste, OsAction::Paste),
            MenuItem::os_action(t("全选", "Select All"), SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action(t("查找", "Find"), FocusFilter),
            MenuItem::action(t("筛选此文件夹", "Filter This Folder"), FilterFolder),
        ]),
        Menu::new(t("显示", "View")).items([
            MenuItem::action(t("文件", "Files"), ShowFiles),
            MenuItem::action(t("传输", "Transfers"), ShowTransfers),
            MenuItem::action(t("线路", "Routes"), ShowRoutes),
            MenuItem::separator(),
            MenuItem::action(t("列表视图", "as List"), ViewList),
            MenuItem::action(t("网格视图", "as Grid"), ViewGrid),
            MenuItem::separator(),
            MenuItem::action(t("刷新", "Refresh"), Refresh),
        ]),
        Menu::new(t("前往", "Go")).items([
            MenuItem::action(t("后退", "Back"), GoBack),
            MenuItem::action(t("前进", "Forward"), GoForward),
            MenuItem::action(t("上级文件夹", "Enclosing Folder"), Parent),
            MenuItem::separator(),
            MenuItem::action(t("我的云端硬盘", "My Drive"), GoMyDrive),
            MenuItem::action(t("与我共享", "Shared with Me"), GoShared),
            MenuItem::action(t("已加星标", "Starred"), GoStarred),
            MenuItem::action(t("最近", "Recent"), GoRecent),
            MenuItem::action(t("回收站", "Trash"), GoTrash),
        ]),
        Menu::new(t("窗口", "Window")).items([
            MenuItem::action(t("最小化", "Minimize"), Minimize),
            MenuItem::action(t("缩放", "Zoom"), Zoom),
        ]),
        Menu::new(t("帮助", "Help")).items([MenuItem::action(
            t("键盘快捷键", "Keyboard Shortcuts"),
            ShowShortcuts,
        )]),
    ]);
}

#[cfg(test)]
mod tests {
    use super::{
        FILES, KeymapOverrides, accepts, bindings, caps, conflicts, display, key_bindings,
        normalize, set,
    };
    use gpui_kit::{Keystroke, OperatingSystem};

    const MAC: OperatingSystem = OperatingSystem::Mac;
    const WIN: OperatingSystem = OperatingSystem::Windows;

    /// Every default keystroke parses on both platform families.
    #[test]
    fn keystrokes_parse() {
        for s in key_bindings() {
            for os in [MAC, WIN] {
                assert!(
                    Keystroke::parse_with_os(s.keys, os).is_ok(),
                    "{} does not parse",
                    s.keys
                );
            }
        }
    }

    /// No two bindings share an id, and no two defaults clash.
    #[test]
    fn defaults_are_distinct() {
        for os in [MAC, WIN, OperatingSystem::Linux] {
            let list = bindings(&KeymapOverrides::new(), os);
            let mut ids: Vec<&str> = list.iter().map(|b| b.id.as_str()).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), list.len());
            for (ix, b) in list.iter().enumerate() {
                assert!(
                    conflicts(&list, ix, b.keys.as_deref().unwrap()).is_empty(),
                    "{} clashes on {os:?}",
                    b.id
                );
            }
        }
    }

    #[test]
    fn formats_key_caps() {
        assert_eq!(caps("secondary-shift-n", MAC), ["⇧", "⌘", "N"]);
        assert_eq!(display("secondary-shift-n", MAC), "⇧⌘N");
        assert_eq!(display("secondary-shift-n", WIN), "Ctrl+Shift+N");
        assert_eq!(display("cmd-alt-h", MAC), "⌥⌘H");
        assert_eq!(display("ctrl-alt-shift-cmd-k", MAC), "⌃⌥⇧⌘K");
        assert_eq!(display("alt-up", WIN), "Alt+Up");
        assert_eq!(display("cmd-up", MAC), "⌘↑");
        assert_eq!(display("secondary-backspace", MAC), "⌘⌫");
        assert_eq!(display("secondary-,", WIN), "Ctrl+,");
        assert_eq!(display("f2", WIN), "F2");
        assert_eq!(display("escape", MAC), "Esc");
        assert_eq!(display("secondary-/", MAC), "⌘/");
    }

    #[test]
    fn normalizes_spellings() {
        assert_eq!(normalize("secondary-,", MAC).as_deref(), Some("cmd-,"));
        assert_eq!(normalize("secondary-,", WIN).as_deref(), Some("ctrl-,"));
        assert_eq!(normalize("shift-cmd-N", MAC), normalize("cmd-shift-n", MAC));
        assert_eq!(normalize("", MAC), None);
    }

    #[test]
    fn overrides_apply_and_reset() {
        let mut overrides = KeymapOverrides::new();
        let list = bindings(&overrides, MAC);
        let refresh = "gdirect::Refresh secondary-r";
        let rename = "files::Rename f2";
        set(&mut overrides, &list, refresh, Some("cmd-shift-r".into()));
        set(&mut overrides, &list, rename, None);
        assert_eq!(overrides[refresh], "cmd-shift-r");
        assert_eq!(overrides[rename], "");
        let list = bindings(&overrides, MAC);
        let find = |id: &str| list.iter().find(|b| b.id == id).unwrap();
        assert_eq!(find(refresh).keys.as_deref(), Some("cmd-shift-r"));
        assert!(find(refresh).changed());
        assert_eq!(find(rename).keys, None);
        // Back at the default: no override left.
        set(&mut overrides, &list, refresh, Some("cmd-r".into()));
        assert!(!overrides.contains_key(refresh));
        // An unknown id changes nothing.
        set(&mut overrides, &list, "nope", None);
        assert!(!overrides.contains_key("nope"));
    }

    #[test]
    fn detects_conflicts() {
        let list = bindings(&KeymapOverrides::new(), MAC);
        let ix = |id: &str| list.iter().position(|b| b.id == id).unwrap();
        let ids = |found: Vec<usize>| -> Vec<String> {
            found.into_iter().map(|j| list[j].id.clone()).collect()
        };
        // A global binding taking another global one's keys.
        let refresh = ix("gdirect::Refresh secondary-r");
        assert_eq!(
            ids(conflicts(&list, refresh, "cmd-1")),
            ["gdirect::ShowFiles secondary-1"]
        );
        // A file-list binding taking a global one's keys clashes too…
        let rename = ix("files::Rename f2");
        assert_eq!(
            ids(conflicts(&list, rename, "cmd-u")),
            ["gdirect::UploadFiles secondary-u"]
        );
        // …and a global one a file-list binding's.
        assert_eq!(ids(conflicts(&list, refresh, "f2")), ["files::Rename f2"]);
        // Free keys, and its own keys, clash with nothing.
        assert!(conflicts(&list, refresh, "cmd-shift-k").is_empty());
        assert!(conflicts(&list, refresh, "cmd-r").is_empty());
        // An unset binding holds no keys.
        let mut overrides = KeymapOverrides::new();
        set(
            &mut overrides,
            &list,
            "gdirect::ShowFiles secondary-1",
            None,
        );
        let list = bindings(&overrides, MAC);
        assert!(conflicts(&list, refresh, "cmd-1").is_empty());
    }

    #[test]
    fn accepts_only_safe_keystrokes() {
        let key = |s: &str| Keystroke::parse_with_os(s, MAC).unwrap();
        assert!(accepts(&key("cmd-k"), None));
        assert!(accepts(&key("ctrl-alt-k"), None));
        assert!(accepts(&key("f6"), None));
        assert!(accepts(&key("enter"), Some(FILES)));
        assert!(!accepts(&key("enter"), None));
        assert!(!accepts(&key("k"), Some(FILES)));
        assert!(!accepts(&key("shift-k"), None));
        assert!(!accepts(&key("shift"), None));
        assert!(!accepts(&key("f"), None));
    }
}
