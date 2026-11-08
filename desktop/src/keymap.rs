//! Keyboard shortcuts and the application menus: every binding is one row of
//! [`key_bindings`], which a shortcuts page can list.
use crate::i18n::t;
use gpui_kit::*;
use std::rc::Rc;

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
        CursorUp,
        CursorDown,
        ExtendUp,
        ExtendDown,
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
    pub fn current(self) -> bool {
        match self {
            Os::All => true,
            Os::Mac => cfg!(target_os = "macos"),
            Os::Other => !cfg!(target_os = "macos"),
        }
    }
}

/// One keyboard shortcut.
pub struct Shortcut {
    pub action: Box<dyn Action>,
    /// The default keystroke; `secondary-` is Command on macOS, Control elsewhere.
    pub keys: &'static str,
    /// What it does, in Chinese and English.
    #[allow(dead_code)] // For the keyboard shortcuts page.
    pub label: (&'static str, &'static str),
    /// The key context it applies in; `None` is everywhere.
    pub context: Option<&'static str>,
    pub os: Os,
}

#[allow(dead_code)] // For the keyboard shortcuts page.
impl Shortcut {
    /// What it does, in the current language.
    pub fn label(&self) -> &'static str {
        t(self.label.0, self.label.1)
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
            "Hide Others",
            None,
            Mac,
        ),
        row(
            CloseWindow,
            "secondary-w",
            "关闭窗口",
            "Close Window",
            None,
            All,
        ),
        row(Minimize, "cmd-m", "最小化", "Minimize", None, Mac),
        // Files menu
        row(
            NewFolder,
            "secondary-shift-n",
            "新建文件夹",
            "New Folder",
            None,
            All,
        ),
        row(
            UploadFiles,
            "secondary-u",
            "上传文件",
            "Upload Files",
            None,
            All,
        ),
        row(
            UploadFolder,
            "secondary-shift-u",
            "上传文件夹",
            "Upload Folder",
            None,
            All,
        ),
        // Pages
        row(ShowFiles, "secondary-1", "文件", "Files", None, All),
        row(ShowTransfers, "secondary-2", "传输", "Transfers", None, All),
        row(ShowRoutes, "secondary-3", "线路", "Routes", None, All),
        row(ShowSettings, "secondary-4", "设置", "Settings", None, All),
        row(Refresh, "secondary-r", "刷新", "Refresh", None, All),
        row(Refresh, "f5", "刷新", "Refresh", None, Other),
        // The file list
        row(SelectAll, "secondary-a", "全选", "Select all", files, All),
        row(Copy, "secondary-c", "复制", "Copy", files, All),
        row(Cut, "secondary-x", "剪切", "Cut", files, All),
        row(Paste, "secondary-v", "粘贴", "Paste", files, All),
        row(
            Cancel,
            "escape",
            "取消选择 / 清除筛选",
            "Clear selection, then filter",
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
            Parent,
            "secondary-[",
            "上级文件夹",
            "Parent folder",
            files,
            All,
        ),
        row(Parent, "cmd-up", "上级文件夹", "Parent folder", files, Mac),
        row(
            Parent,
            "alt-up",
            "上级文件夹",
            "Parent folder",
            files,
            Other,
        ),
        row(
            Parent,
            "backspace",
            "上级文件夹",
            "Parent folder",
            files,
            All,
        ),
        row(FocusFilter, "secondary-f", "筛选", "Filter", files, All),
        row(CursorUp, "up", "上一项", "Previous item", files, All),
        row(CursorDown, "down", "下一项", "Next item", files, All),
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

/// Registers [`key_bindings`] for this platform.
pub fn bind(cx: &mut App) {
    let bindings: Vec<KeyBinding> = key_bindings()
        .into_iter()
        .filter(|s| s.os.current())
        .filter_map(|s| {
            let context = s
                .context
                .and_then(|c| KeyBindingContextPredicate::parse(c).ok())
                .map(Rc::new);
            KeyBinding::load(s.keys, s.action, context, false, None, cx).ok()
        })
        .collect();
    cx.bind_keys(bindings);
}

/// Application commands that need no window: quitting and hiding.
pub fn install(cx: &mut App) {
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
        ]),
        Menu::new(t("显示", "View")).items([
            MenuItem::action(t("文件", "Files"), ShowFiles),
            MenuItem::action(t("传输", "Transfers"), ShowTransfers),
            MenuItem::action(t("线路", "Routes"), ShowRoutes),
            MenuItem::action(t("设置", "Settings"), ShowSettings),
            MenuItem::separator(),
            MenuItem::action(t("列表视图", "as List"), ViewList),
            MenuItem::action(t("网格视图", "as Grid"), ViewGrid),
            MenuItem::separator(),
            MenuItem::action(t("刷新", "Refresh"), Refresh),
        ]),
        Menu::new(t("窗口", "Window")).items([
            MenuItem::action(t("最小化", "Minimize"), Minimize),
            MenuItem::action(t("缩放", "Zoom"), Zoom),
        ]),
    ]);
}

#[cfg(test)]
mod tests {
    use super::key_bindings;
    use gpui_kit::{Keystroke, OperatingSystem};

    /// Every default keystroke parses on both platform families.
    #[test]
    fn keystrokes_parse() {
        for s in key_bindings() {
            for os in [OperatingSystem::Mac, OperatingSystem::Windows] {
                assert!(
                    Keystroke::parse_with_os(s.keys, os).is_ok(),
                    "{} does not parse",
                    s.keys
                );
            }
        }
    }
}
