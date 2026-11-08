//! gdirect's desktop client: browse, transfer and mount Google Drive.
// A GUI app on Windows: no console window opens with it.
#![cfg_attr(windows, windows_subsystem = "windows")]
mod accounts;
mod app;
mod endpoints;
mod files;
mod format;
mod i18n;
mod keymap;
mod nav;
mod places;
mod settings;
mod settings_page;
mod transfers;
mod transfers_page;
mod ui;

use gpui_kit::{prelude::*, *};
use gpui_kit_material::{icons::Assets, prelude::*, theme};
use settings::WindowState;
use std::{borrow::Cow, cell::Cell, rc::Rc, time::Duration};

fn main() -> anyhow::Result<()> {
    // A pinned background mount runs rclone under `<this binary> mount-helper`.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--version") {
        println!("gdirect-desktop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
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
            let store = gdirect::store::Store::open(&db_path).ok();
            keymap::install(
                store
                    .as_ref()
                    .map(settings::load_keymap)
                    .unwrap_or_default(),
                cx,
            );
            let min = size(px(600.), px(480.));
            let mut options = WindowOptions {
                window_min_size: Some(min),
                ..title_bar_window_options("gdirect", app::TITLE_BAR, size(px(1280.), px(820.)), cx)
            };
            if let Some((bounds, display)) = store
                .as_ref()
                .and_then(settings::WindowState::load)
                .and_then(|saved| restore_bounds(&saved, min, cx))
            {
                options.window_bounds = Some(bounds);
                options.display_id = Some(display);
            }
            gpui_kit::open_window(options, cx, move |window, cx| {
                if let Some(db) = &store {
                    let settings = settings::Settings::load(db);
                    i18n::apply(settings.language);
                    theme::init_component_theme(
                        app::Gdirect::initial_scheme(&settings, cx),
                        1.0,
                        cx,
                    );
                }
                let view = cx.new(|cx| app::Gdirect::new(db_path, window, cx));
                if let Some(db) = store {
                    save_bounds(&view, db, window, cx);
                }
                view
            })
            .expect("open the window");
        });
    Ok(())
}

/// The saved frame, on the display it was on (or the primary one when that
/// is gone), kept on screen and at least `min`; None opens the default frame.
fn restore_bounds(
    saved: &WindowState,
    min: Size<Pixels>,
    cx: &App,
) -> Option<(WindowBounds, DisplayId)> {
    let display = saved
        .display
        .as_deref()
        .and_then(|uuid| {
            cx.displays()
                .into_iter()
                .find(|d| d.uuid().is_ok_and(|u| u.to_string() == uuid))
        })
        .or_else(|| cx.primary_display())?;
    let area = display.bounds();
    let frame = saved.clamped(
        (
            area.origin.x.into(),
            area.origin.y.into(),
            area.size.width.into(),
            area.size.height.into(),
        ),
        (min.width.into(), min.height.into()),
    )?;
    let bounds = Bounds::new(
        point(px(frame.x), px(frame.y)),
        size(px(frame.w), px(frame.h)),
    );
    let bounds = if frame.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    Some((bounds, display.id()))
}

/// Saves the window's frame half a second after it last moved or resized,
/// through a connection of its own to the state database.
fn save_bounds(
    view: &Entity<app::Gdirect>,
    db: gdirect::store::Store,
    window: &mut Window,
    cx: &mut App,
) {
    let db = Rc::new(db);
    let serial = Rc::new(Cell::new(0u64));
    view.update(cx, |_, cx| {
        cx.observe_window_bounds(window, move |_, window, cx| {
            // Full screen is not restored; the frame it returns to is.
            let (bounds, maximized) = match window.window_bounds() {
                WindowBounds::Windowed(b) | WindowBounds::Fullscreen(b) => (b, false),
                WindowBounds::Maximized(b) => (b, true),
            };
            let state = WindowState {
                x: bounds.origin.x.into(),
                y: bounds.origin.y.into(),
                w: bounds.size.width.into(),
                h: bounds.size.height.into(),
                maximized,
                display: window
                    .display(cx)
                    .and_then(|d| d.uuid().ok())
                    .map(|u| u.to_string()),
            };
            let n = serial.get() + 1;
            serial.set(n);
            let (db, serial) = (db.clone(), serial.clone());
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                if serial.get() == n {
                    state.save(&db);
                }
            })
            .detach();
        })
        .detach();
    });
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
