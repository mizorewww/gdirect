//! The interface language. Every user-facing string is written in both
//! languages at its call site: `t("下载", "Download")` for plain text and
//! `tf!("{n} 个文件", "{n} files")` for formatted text.
use std::sync::atomic::{AtomicBool, Ordering};

/// The interface language a user picks; `System` follows the OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    System,
    Chinese,
    English,
}

static CHINESE: AtomicBool = AtomicBool::new(false);

/// Whether the interface is in Chinese.
pub fn chinese() -> bool {
    CHINESE.load(Ordering::Relaxed)
}

/// Switches the interface language; views pick it up when they next render.
pub fn apply(language: Language) {
    let chinese = match language {
        Language::Chinese => true,
        Language::English => false,
        Language::System => system_chinese(),
    };
    CHINESE.store(chinese, Ordering::Relaxed);
}

/// Whether the OS prefers Chinese; anything else gets English.
fn system_chinese() -> bool {
    sys_locale::get_locale().is_some_and(|locale| locale.to_ascii_lowercase().starts_with("zh"))
}

/// The text for the current language.
pub fn t(zh: &'static str, en: &'static str) -> &'static str {
    if chinese() { zh } else { en }
}

/// `format!` with a Chinese and an English template; both see the same
/// arguments and captured variables.
#[macro_export]
macro_rules! tf {
    ($zh:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::chinese() {
            format!($zh $(, $arg)*)
        } else {
            format!($en $(, $arg)*)
        }
    };
}
