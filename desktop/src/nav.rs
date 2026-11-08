//! The navigation rail: the pages, and Drive's places under Files. A wide
//! window keeps it expanded beside the content when the user chose so; a
//! narrower one shows it collapsed, and its menu button floats the expanded
//! rail over the content.
use crate::i18n::t;
use crate::{
    app::{Dialog, Gdirect, Page, TITLE_BAR},
    places::{self, Place},
    ui,
};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::Symbol;
use gpui_kit_material::prelude::*;
use gpui_kit_material::tokens::elevation;

/// The setting that keeps the rail expanded in wide windows.
pub const RAIL_KEY: &str = "desktop.rail_expanded";
/// M3's large window class, from which an expanded rail sits beside the content.
const WIDE: f32 = 1200.;
const RAIL_WIDTH: f32 = 256.;

fn nav_icon(name: &'static str) -> NavIcon {
    NavIcon::symbol(Symbol::named(name).unwrap_or(Symbol::FOLDER))
}

impl Gdirect {
    /// Whether the expanded rail sits beside the content of a window this wide.
    fn rail_docked(&self, window: f32) -> bool {
        window >= WIDE && self.rail_expanded && !self.accounts.is_empty()
    }

    /// What the rail takes beyond its collapsed width, which the pages'
    /// breakpoints leave out (see [`Gdirect::wide`]).
    pub fn rail_extra(&self, window: f32) -> f32 {
        if self.rail_docked(window) {
            RAIL_WIDTH - RAIL_COLLAPSED_WIDTH
        } else {
            0.
        }
    }

    /// Reads the account's Shared drives, quota and My Drive ID once per sign-in.
    pub fn load_places(&mut self, _cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else {
            return;
        };
        if self.about.is_some() {
            return;
        }
        let session = self.session;
        self.background(
            move || {
                (
                    api.about().ok(),
                    api.drives().unwrap_or_default(),
                    api.metadata("root").ok().map(|f| f.id),
                )
            },
            move |this, (about, drives, root), cx| {
                if this.session != session {
                    return;
                }
                this.about = about;
                this.drives = drives;
                this.root_id = root;
                cx.notify();
            },
        );
    }

    /// The collapsed rail's destinations: the pages.
    fn page_items(&self) -> Vec<NavItem> {
        let signed_out = self.accounts.is_empty();
        [
            ("files", t("文件", "Files"), "folder"),
            ("transfers", t("传输", "Transfers"), "swap_vert"),
            ("endpoints", t("线路", "Routes"), "network_ping"),
            ("settings", t("设置", "Settings"), "settings"),
        ]
        .into_iter()
        // Signed out, the welcome screen stands in for Files, and Settings
        // stays; the other pages need an account.
        .filter(|(value, ..)| !signed_out || matches!(*value, "files" | "settings"))
        .map(|(value, label, icon)| self.badged(NavItem::new(value, label, nav_icon(icon))))
        .collect()
    }

    /// The expanded rail's destinations: the places under Files, then the
    /// other pages; with their section headers.
    fn place_items(&self) -> (Vec<NavItem>, Vec<(usize, SharedString)>) {
        let place = |place: Place, icon| NavItem::new(place.value(), place.name(), nav_icon(icon));
        let mut items = vec![
            place(Place::MyDrive, "hard_drive"),
            place(Place::Shared, "folder_shared"),
            place(Place::Starred, "star"),
            place(Place::Recent, "schedule"),
            place(Place::Trash, "delete"),
        ];
        let mut sections = vec![(0, t("文件", "Files").into())];
        if !self.drives.is_empty() {
            sections.push((items.len(), t("共享云端硬盘", "Shared drives").into()));
            items.extend(self.drives.iter().map(|d| {
                place(
                    Place::Drive {
                        id: d.id.clone(),
                        name: d.name.clone(),
                    },
                    "folder_supervised",
                )
            }));
        }
        sections.push((items.len(), t("更多", "More").into()));
        items.extend(
            self.page_items()
                .into_iter()
                .filter(|i| i.value() != "files"),
        );
        (items, sections)
    }

    fn badged(&self, item: NavItem) -> NavItem {
        let pending = self.queues.pending();
        if item.value() == "transfers" && pending > 0 {
            item.badge(NavBadge::Large(pending.to_string().into()))
        } else {
            item
        }
    }

    fn nav_selected(&self, expanded: bool) -> String {
        match self.page {
            Page::Files if expanded => self.location.place.value(),
            page => page.value().into(),
        }
    }

    fn navigate(&mut self, value: &str, cx: &mut Context<Self>) {
        self.rail_modal = false;
        let page = match value {
            "files" => Page::Files,
            "transfers" => Page::Transfers,
            "endpoints" => Page::Endpoints,
            "settings" => Page::Settings,
            _ => {
                if let Some(place) = Place::from_value(value, &self.drives) {
                    self.go_place(place, cx);
                }
                return;
            }
        };
        self.go(page, cx);
    }

    /// The menu button: it collapses or expands a docked rail, and opens or
    /// closes the floating one.
    fn rail_menu(
        &self,
        id: &'static str,
        expanded: bool,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> IconButton {
        let (icon, label) = if expanded {
            (Symbol::MENU_OPEN, t("收起导航栏", "Collapse navigation"))
        } else {
            (Symbol::MENU, t("展开导航栏", "Expand navigation"))
        };
        IconButton::new(id, icon, label).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                if wide {
                    this.rail_expanded = !this.rail_expanded;
                    let _ = this
                        .db
                        .set(RAIL_KEY, if this.rail_expanded { "1" } else { "0" });
                } else {
                    this.rail_modal = !this.rail_modal;
                }
                cx.notify();
            },
        ))
    }

    fn new_fab(&self, expanded: bool, floating: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let id = |name: &'static str| -> ElementId {
            if floating {
                (name, 1usize).into()
            } else {
                name.into()
            }
        };
        let disabled = self.account.is_none() || self.api.is_none();
        let toggle = cx.listener(|this, event: &ClickEvent, _, cx| {
            if matches!(event, ClickEvent::Keyboard(_)) {
                this.new_menu = !this.new_menu;
                cx.notify();
            }
        });
        let fab = if expanded {
            ExtendedFab::new(id("new"), Symbol::ADD, t("新建", "New"))
                .color(FabColor::PrimaryContainer)
                .disabled(disabled)
                .on_click(toggle)
                .into_any_element()
        } else {
            Fab::new(id("new"), Symbol::ADD, t("新建", "New"))
                .color(FabColor::PrimaryContainer)
                .disabled(disabled)
                .on_click(toggle)
                .into_any_element()
        };
        // Places that take no new files make them in My Drive, as Drive does.
        let menu = MaterialMenu::new(id("new-menu"))
            .width(220.)
            .item(
                MaterialMenuItem::new(t("新建文件夹", "New folder"))
                    .leading(ui::symbol("create_new_folder", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.new_menu = false;
                        this.rail_modal = false;
                        this.go(Page::Files, cx);
                        this.writable_here(cx);
                        this.open_dialog(Dialog::NewFolder, window, cx);
                    })),
            )
            .item(
                MaterialMenuItem::new(t("上传文件…", "Upload files…"))
                    .leading(ui::symbol("upload_file", false))
                    .divider(false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.new_menu = false;
                        this.rail_modal = false;
                        this.writable_here(cx);
                        this.choose_upload(false, cx);
                    })),
            )
            .item(
                MaterialMenuItem::new(t("上传文件夹…", "Upload folder…"))
                    .leading(ui::symbol("drive_folder_upload", false))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.new_menu = false;
                        this.rail_modal = false;
                        this.writable_here(cx);
                        this.choose_upload(true, cx);
                    })),
            );
        MenuPopup::new(id("new-popup"), fab, menu)
            .open(self.new_menu)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                // The menu ignores its trigger's disabled state.
                this.new_menu = *open && this.api.is_some();
                cx.notify();
            }))
    }

    /// One rail with its quota footer, in a column that scrolls when the
    /// destinations do not fit (the kit's rail clips them).
    fn rail_column(&self, floating: bool, viewport: Size<Pixels>, cx: &mut Context<Self>) -> Div {
        let window = f32::from(viewport.width);
        let wide = window >= WIDE;
        let expanded = floating || self.rail_docked(window);
        let signed_in = !self.accounts.is_empty();
        let (items, sections) = if expanded && signed_in {
            self.place_items()
        } else {
            (self.page_items(), Vec::new())
        };
        // Signed out there is nothing to create.
        let fab = signed_in;
        // The kit's geometry: top inset, menu, FAB and the gap under them;
        // then the items and section headers, and the bottom inset.
        let header = 44. + 48. + if fab { 4. + 56. } else { 0. } + 40.;
        let list = if expanded {
            items.len() as f32 * 56. + sections.len() as f32 * 36.
        } else {
            items.len() as f32 * 68.
        };
        let footer = if expanded { self.render_quota() } else { None };
        let room = f32::from(viewport.height)
            - f32::from(TITLE_BAR)
            - if footer.is_some() { 64. } else { 0. };
        let need = header + list + 20.;
        // A short window takes back most of the kit's top inset, made for a
        // window without a title bar, and the bottom one under the last
        // label; what still does not fit scrolls, with a scrollbar.
        let (lift, drop) = if need > room { (28., 16.) } else { (0., 0.) };
        let overflows = need - lift - drop > room + 0.5;
        let mut rail = NavigationRail::new(
            if floating { "modal-rail" } else { "rail" },
            t("主导航", "Main navigation"),
        )
        .container(ColorRole::SURFACE_CONTAINER)
        .items(items)
        .selected(self.nav_selected(expanded))
        .expanded(expanded)
        .expanded_width(RAIL_WIDTH)
        .on_change(cx.listener(|this, value: &SharedString, _, cx| this.navigate(value, cx)));
        for (before, header) in sections {
            rail = rail.section(before, header);
        }
        if signed_in {
            rail = rail.menu(self.rail_menu(
                if floating {
                    "modal-rail-menu"
                } else {
                    "rail-menu"
                },
                expanded,
                wide,
                cx,
            ));
        }
        if fab {
            rail = rail.fab(self.new_fab(expanded, floating, cx));
        }
        div()
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id(if floating {
                        "modal-rail-scroll"
                    } else {
                        "rail-scroll"
                    })
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .h(rem(room.max(need)))
                            .mt(rem(-lift))
                            .mb(rem(-drop))
                            .flex()
                            .child(rail),
                    ),
            )
            // The list scrolls under the footer: a line marks where.
            .children(footer.map(|footer| {
                footer.when(overflows, |d| {
                    d.border_t_1()
                        .border_color(color(ColorRole::OUTLINE_VARIANT))
                })
            }))
    }

    /// The rail beside the content.
    pub fn render_nav(
        &mut self,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // A window widened past the breakpoint docks the rail instead.
        if f32::from(viewport.width) >= WIDE && self.rail_modal {
            self.rail_modal = false;
        }
        self.rail_column(false, viewport, cx)
    }

    /// The expanded rail floating over a narrow window, on a scrim.
    pub fn render_modal_nav(
        &mut self,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.rail_modal {
            return None;
        }
        let close = cx.listener(|this, _: &(), _, cx| {
            this.rail_modal = false;
            cx.notify();
        });
        let close = std::rc::Rc::new(close);
        let (scrim_close, key_close) = (close.clone(), close);
        Some(
            div()
                .id("nav-overlay")
                .absolute()
                .inset_0()
                // The list behind takes neither the pointer nor the wheel;
                // a press on the scrim only closes the rail.
                .occlude()
                .on_scroll(|_, _, cx| cx.stop_propagation())
                .child(navigation_scrim("nav-scrim", move |window, cx| {
                    scrim_close(&(), window, cx)
                }))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .bg(color(ColorRole::SURFACE_CONTAINER))
                        .rounded_r(RADIUS_LARGE)
                        .shadow(elevation::level(2))
                        .overflow_hidden()
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                key_close(&(), window, cx);
                            }
                        })
                        .child(self.rail_column(true, viewport, cx)),
                )
                .into_any_element(),
        )
    }

    /// "3.2 GB of 15 GB used" over a thin bar, under the expanded rail.
    fn render_quota(&self) -> Option<Div> {
        let quota = &self.about.as_ref()?.storage;
        let label = places::quota(quota.usage, quota.limit);
        let fraction = quota
            .limit
            .filter(|l| *l > 0)
            .map(|l| (quota.usage as f64 / l as f64).clamp(0., 1.) as f32);
        Some(
            div()
                .w(rem(RAIL_WIDTH))
                .h(rem(64.))
                .flex_none()
                .px(rem(28.))
                .pb(rem(16.))
                .flex()
                .flex_col()
                .justify_end()
                .gap(rem(8.))
                .when_some(fraction, |d, fraction| {
                    d.child(
                        LinearProgress::new("quota", label.clone())
                            .value(Some(fraction))
                            .stop_indicator(false),
                    )
                })
                .child(ui::line(
                    TypeRole::BodySmall,
                    ColorRole::ON_SURFACE_VARIANT,
                    label,
                )),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit_material::icons::Symbol;

    /// The symbols the places and their menus name exist (an unknown name
    /// falls back to a placeholder at runtime).
    #[test]
    fn symbols_resolve() {
        for name in [
            "hard_drive",
            "folder_shared",
            "folder_supervised",
            "star",
            "schedule",
            "delete",
            "swap_vert",
            "network_ping",
            "settings",
            "folder",
            "shortcut",
            "person",
            "cloud",
            "open_in_new",
            "link",
            "person_add",
            "delete_forever",
            "restore_from_trash",
            "star_border",
            "kid_star",
            "search",
            "filter_list",
            "arrow_drop_down",
            "folder_open",
            "search_off",
        ] {
            assert!(Symbol::named(name).is_some(), "{name}");
        }
    }
}
