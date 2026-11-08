//! gdirect's desktop client: browse, transfer and mount Google Drive.
mod accounts;
mod app;
mod endpoints;
mod files;
mod format;
mod i18n;
mod settings;
mod settings_page;
mod transfers;
mod transfers_page;
mod ui;

use gpui_kit::{prelude::*, *};
use gpui_kit_material::{icons::Assets, prelude::*, theme};
use std::borrow::Cow;

fn main() -> anyhow::Result<()> {
    // A pinned background mount runs rclone under `<this binary> mount-helper`.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "mount-helper") {
        return mount_helper(&args[1..]);
    }
    // `--db PATH` uses another state file, as the command line's --db does.
    let db_path = match args.iter().position(|a| a == "--db") {
        Some(i) => args
            .get(i + 1)
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("--db needs a path"))?,
        None => gdirect::store::default_path()?,
    };
    gpui_kit::application()
        .with_assets(Assets::new())
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../assets/fonts/Roboto-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Roboto-Medium.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Roboto-SemiBold.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/RobotoMono-Regular.ttf")),
                ])
                .expect("bundled fonts");
            gpui_kit::init(cx);
            gpui_kit_material::init(cx);
            let options = WindowOptions {
                window_min_size: Some(size(px(900.), px(600.))),
                ..title_bar_window_options("gdirect", app::TITLE_BAR, size(px(1280.), px(820.)), cx)
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                if let Ok(db) = gdirect::store::Store::open(&db_path) {
                    let settings = settings::Settings::load(&db);
                    i18n::apply(settings.language);
                    theme::init_component_theme(
                        app::Gdirect::initial_scheme(&settings, cx),
                        1.0,
                        cx,
                    );
                }
                cx.new(|cx| app::Gdirect::new(db_path, window, cx))
            })
            .expect("open the window");
        });
    Ok(())
}

/// `mount-helper --endpoint IP… -- rclone …`, as `gdirect mount-helper`.
fn mount_helper(args: &[std::ffi::OsString]) -> anyhow::Result<()> {
    let split = args
        .iter()
        .position(|a| a == "--")
        .ok_or_else(|| anyhow::anyhow!("mount-helper needs -- before the command"))?;
    let mut endpoints = Vec::new();
    let mut flags = args[..split].iter();
    while let Some(flag) = flags.next() {
        anyhow::ensure!(flag == "--endpoint", "unexpected mount-helper argument");
        let ip = flags
            .next()
            .and_then(|ip| ip.to_str())
            .ok_or_else(|| anyhow::anyhow!("--endpoint needs an IP"))?;
        endpoints.push(ip.parse()?);
    }
    gdirect::mount::helper(endpoints, &args[split + 1..])
}
