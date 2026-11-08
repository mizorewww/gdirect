//! Small presentation helpers shared by the pages.
use crate::format::Kind;
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::{Icon, Style, Symbol};
use gpui_kit_material::prelude::*;
use std::{cell::RefCell, collections::HashMap};

/// The interface's typeface: Roboto, or in Chinese the platform's CJK UI
/// face, whose full-width punctuation (“”，：) keeps CJK widths, where
/// Roboto, which has those marks, draws them narrow. User content is set in
/// [`latin`] instead.
pub fn font_family() -> &'static str {
    if crate::i18n::chinese() {
        cjk_family()
    } else {
        "Roboto"
    }
}

/// The platform's CJK UI face, which [`font_family`] uses in Chinese and
/// [`latin`] falls back to.
fn cjk_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "PingFang SC"
    } else if cfg!(target_os = "windows") {
        "Microsoft YaHei UI"
    } else {
        "Noto Sans CJK SC"
    }
}

/// Sets user content (file names, paths, addresses) in Roboto, with the CJK
/// face for what Roboto lacks: its hyphens, digits and dots look as they do
/// in the text fields, which the CJK face's Latin would not (its hyphen
/// reads as a dash). Applied after a type role, which picks the interface's
/// face.
pub fn latin<E: Styled>(mut element: E) -> E {
    let style = element.text_style();
    style.font_family = Some("Roboto".into());
    style.font_fallbacks = Some(FontFallbacks::from_fonts(vec![cjk_family().to_owned()]));
    element
}

/// A [`line`] of user content, in [`latin`].
pub fn name_line(role: TypeRole, ink: ColorRole, content: impl Into<SharedString>) -> Div {
    latin(line(role, ink, content))
}

/// The container of a card that holds rows: white on the tinted panes in
/// light, and in dark a step lighter than the page, never darker.
pub fn card() -> ColorRole {
    if gpui_kit_material::theme::is_dark() {
        ColorRole::SURFACE_CONTAINER_HIGH
    } else {
        ColorRole::SURFACE_CONTAINER_LOWEST
    }
}

/// Applies a Material type role to text (the library keeps its own private).
pub fn type_role<E: Styled>(element: E, role: TypeRole) -> E {
    let scale = role.scale();
    element
        .font_family(if scale.font_family == "Roboto" {
            font_family()
        } else {
            scale.font_family
        })
        .text_size(scale.font_size)
        .line_height(scale.line_height)
        .font_weight(FontWeight(scale.font_weight as f32))
}

pub fn text(role: TypeRole, ink: ColorRole, content: impl Into<SharedString>) -> Div {
    type_role(div(), role)
        .text_color(color(ink))
        .child(content.into())
}

/// One line that ends in an ellipsis when it does not fit.
pub fn line(role: TypeRole, ink: ColorRole, content: impl Into<SharedString>) -> Div {
    text(role, ink, content)
        .min_w_0()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
}

pub fn mono(content: impl Into<SharedString>) -> Div {
    div()
        .font_family("Roboto Mono")
        .text_size(rem(13.))
        .child(content.into())
}

thread_local! {
    static PATHS: RefCell<HashMap<(&'static str, bool), SharedString>> = Default::default();
}

/// A rounded Material Symbol by name; resolved paths are cached.
pub fn symbol(name: &'static str, filled: bool) -> Icon {
    let path = PATHS.with(|paths| {
        paths
            .borrow_mut()
            .entry((name, filled))
            .or_insert_with(|| {
                Symbol::named(name)
                    .unwrap_or(Symbol::HELP)
                    .icon()
                    .style(Style::Rounded)
                    .filled(filled)
                    .asset_path()
                    .into()
            })
            .clone()
    });
    Icon::new(path)
}

pub fn icon(name: &'static str, size: f32, ink: ColorRole) -> Icon {
    symbol(name, false).size(rem(size)).color(color(ink))
}

/// The tonal pair a file kind's tile is painted with.
pub fn kind_roles(kind: Kind) -> (ColorRole, ColorRole) {
    match kind {
        Kind::Folder => (
            ColorRole::SECONDARY_CONTAINER,
            ColorRole::ON_SECONDARY_CONTAINER,
        ),
        Kind::Pdf => (ColorRole::ERROR_CONTAINER, ColorRole::ON_ERROR_CONTAINER),
        Kind::Sheet => (ColorRole::TINT_GREEN, ColorRole::ON_TINT_GREEN),
        Kind::Image => (ColorRole::TINT_PINK, ColorRole::ON_TINT_PINK),
        Kind::Video | Kind::Audio => (ColorRole::TINT_ORANGE, ColorRole::ON_TINT_ORANGE),
        Kind::Doc => (
            ColorRole::PRIMARY_CONTAINER,
            ColorRole::ON_PRIMARY_CONTAINER,
        ),
        Kind::GoogleDoc => (ColorRole::TINT_BLUE, ColorRole::ON_TINT_BLUE),
        Kind::Archive => (ColorRole::TINT_AMBER, ColorRole::ON_TINT_AMBER),
        Kind::Other => (
            ColorRole::SURFACE_CONTAINER_HIGHEST,
            ColorRole::ON_SURFACE_VARIANT,
        ),
    }
}

/// A file kind's icon on its tonal tile.
pub fn kind_tile(kind: Kind, size: f32) -> Div {
    let (bg, fg) = kind_roles(kind);
    div()
        .flex_none()
        .size(rem(size))
        .rounded(rem(size / 4.))
        .bg(color(bg))
        .flex()
        .items_center()
        .justify_center()
        .child(
            symbol(kind.icon(), true)
                .size(rem(size * 0.6))
                .color(color(fg)),
        )
}

/// A pill-shaped informational chip (not interactive).
pub fn pill(label: impl Into<SharedString>, bg: ColorRole, fg: ColorRole) -> Div {
    type_role(div(), TypeRole::LabelMedium)
        .flex_none()
        .px(rem(8.))
        .py(rem(2.))
        .rounded(RADIUS_SMALL)
        .bg(color(bg))
        .text_color(color(fg))
        .child(label.into())
}

/// A card holding rows, as the settings panes and queues group them.
pub fn group(rows: Vec<AnyElement>) -> Div {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap(rem(2.))
        .children(rows.into_iter().enumerate().map(move |(i, row)| {
            let first = i == 0;
            let last = i + 1 == count;
            let big = rem(20.);
            let small = rem(6.);
            div()
                .bg(color(card()))
                .rounded_tl(if first { big } else { small })
                .rounded_tr(if first { big } else { small })
                .rounded_bl(if last { big } else { small })
                .rounded_br(if last { big } else { small })
                .child(row)
        }))
}

/// The indent of a settings row's text from the row's content edge: the
/// leading icon (24) and the gap after it (16).
pub const ROW_INDENT: f32 = 40.;

/// A settings row's title over its supporting element.
fn row_text(title: SharedString, supporting: Option<AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rem(2.))
        .child(text(TypeRole::BodyLarge, ColorRole::ON_SURFACE, title))
        .children(supporting)
}

/// Supporting text under a settings row's title.
pub fn supporting(content: impl Into<SharedString>) -> AnyElement {
    text(TypeRole::BodyMedium, ColorRole::ON_SURFACE_VARIANT, content).into_any_element()
}

/// A path as supporting text: one line that ends in an ellipsis, with the
/// whole path in a tooltip.
pub fn path(id: &'static str, content: impl Into<SharedString>) -> AnyElement {
    path_in(id, ColorRole::ON_SURFACE_VARIANT, content)
}

/// A [`path`] in another ink, for surfaces other than the settings cards.
pub fn path_in(id: &'static str, ink: ColorRole, content: impl Into<SharedString>) -> AnyElement {
    let content = content.into();
    MaterialTooltip::plain(id, content.clone())
        .trigger(name_line(TypeRole::BodyMedium, ink, content))
        .into_any_element()
}

/// A settings row: an icon, a title over supporting text, and a trailing control.
pub fn setting_row(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    supporting: Option<SharedString>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    setting_row_with(icon_name, title, supporting.map(self::supporting), trailing)
}

/// [`setting_row`] with any element, such as a [`path`], as supporting text.
pub fn setting_row_with(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    supporting: Option<AnyElement>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    // The text claims a real width, so a control that does not fit beside it
    // wraps onto a line of its own, under the title rather than at the edge.
    div()
        .flex()
        .items_center()
        .gap(rem(16.))
        .min_h(rem(72.))
        .px(rem(20.))
        .py(rem(12.))
        .child(
            div()
                .flex_none()
                .flex()
                .child(icon(icon_name, 24., ColorRole::ON_SURFACE_VARIANT)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(rem(16.))
                .gap_y(rem(8.))
                .child(
                    row_text(title.into(), supporting)
                        .flex_grow(1.)
                        .flex_basis(rem(240.))
                        .min_w(rem(200.)),
                )
                .when_some(trailing, |d, t| d.child(div().flex_none().child(t))),
        )
        .into_any_element()
}

/// A settings row whose control always sits under the text, lined up with the
/// title: for controls too wide to share a line with it.
pub fn setting_row_stacked(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    supporting: Option<SharedString>,
    control: impl IntoElement,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap(rem(16.))
        .px(rem(20.))
        .py(rem(16.))
        .child(
            div()
                .flex_none()
                .flex()
                .child(icon(icon_name, 24., ColorRole::ON_SURFACE_VARIANT)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap(rem(12.))
                .child(row_text(title.into(), supporting.map(self::supporting)).w_full())
                .child(control),
        )
        .into_any_element()
}

/// The look of a text button that cannot be pressed right now, without the
/// filled container the kit gives disabled buttons.
pub fn text_button_disabled(label: impl Into<SharedString>) -> Div {
    type_role(div(), TypeRole::LabelLarge)
        .flex_none()
        .h(rem(40.))
        .px(rem(12.))
        .flex()
        .items_center()
        .text_color(color(ColorRole::ON_SURFACE))
        .opacity(0.38)
        .child(label.into())
}

/// Small print under a group, inset like a section label.
pub fn footnote(content: impl Into<SharedString>) -> Div {
    text(TypeRole::BodySmall, ColorRole::ON_SURFACE_VARIANT, content).px(rem(4.))
}

pub fn section_label(label: impl Into<SharedString>) -> Div {
    text(TypeRole::TitleSmall, ColorRole::ON_SURFACE_VARIANT, label).px(rem(4.))
}

/// The empty state of a list: an icon over a short explanation.
pub fn empty(
    icon_name: &'static str,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rem(8.))
        .py(rem(64.))
        .px(rem(24.))
        .child(icon(icon_name, 48., ColorRole::OUTLINE))
        .child(text(TypeRole::TitleMedium, ColorRole::ON_SURFACE, title))
        .child(
            text(TypeRole::BodyMedium, ColorRole::ON_SURFACE_VARIANT, detail)
                .text_center()
                .max_w(rem(420.)),
        )
}

/// A right-click menu that opens down from the pointer and, where the
/// window has no room for that, slides up just enough to fit whole: the
/// kit's `MaterialContextMenu` opens below or above the pointer and scrolls
/// when neither side has the room, which in a short window is most of the
/// list.
/// Builds a menu for the frame it shows on.
type MenuBuilder = Box<dyn Fn(&mut Window, &mut App) -> MaterialMenu>;
/// Told when the menu opens or closes.
type OpenChange = std::rc::Rc<dyn Fn(bool, &mut Window, &mut App)>;

#[derive(gpui_kit::IntoElement)]
pub struct ContextMenu {
    id: ElementId,
    menu: MenuBuilder,
    button: MouseButton,
    on_open_change: Option<OpenChange>,
}

impl ContextMenu {
    /// `menu` builds the menu on each frame it is shown, and no other.
    pub fn new(
        id: impl Into<ElementId>,
        menu: impl Fn(&mut Window, &mut App) -> MaterialMenu + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            menu: Box::new(menu),
            button: MouseButton::Right,
            on_open_change: None,
        }
    }

    /// Calls `f` when the menu opens (true) and closes (false), so its
    /// target can stay highlighted meanwhile.
    pub fn on_open_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_open_change = Some(std::rc::Rc::new(f));
        self
    }

    /// Opens on a left press instead, over a button such as a row's ⋮.
    pub fn on_left_press(mut self) -> Self {
        self.button = MouseButton::Left;
        self
    }
}

#[derive(Default)]
struct ContextMenuState {
    /// In the tree since the first press; it stays, closed, after that.
    mounted: bool,
    /// The popup enters the tree closed for one frame, so it has a trigger
    /// position to open at.
    entering: bool,
    open: bool,
    /// The pointer's offset from the parent's top left, and in the window.
    position: Point<Pixels>,
    in_window: Point<Pixels>,
}

/// The menus' width, which the menu does not report; the file menus' 220
/// with some to spare.
const CONTEXT_MENU_WIDTH: f32 = 240.;

impl RenderOnce for ContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id.clone(), "context-menu"), cx, |_, _| {
            ContextMenuState::default()
        });
        let button = self.button;
        let opened = self.on_open_change.clone();
        let press = canvas(
            |bounds, window, _| (bounds, window.insert_hitbox(bounds, HitboxBehavior::Normal)),
            {
                let state = state.clone();
                move |_, (bounds, hitbox): (Bounds<Pixels>, Hitbox), window, _| {
                    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble() && event.button == button && hitbox.is_hovered(window) {
                            // Bubbling reaches the innermost context menu first.
                            cx.stop_propagation();
                            let position = event.position - bounds.origin;
                            state.update(cx, |state, cx| {
                                state.entering = !state.mounted;
                                state.mounted = true;
                                state.open = true;
                                state.position = position;
                                state.in_window = event.position;
                                cx.notify();
                            });
                            if let Some(opened) = &opened {
                                opened(true, window, cx);
                            }
                        }
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut popup = None;
        if state.read(cx).mounted {
            let entering = state.update(cx, |state, _| std::mem::take(&mut state.entering));
            if entering {
                window.request_animation_frame();
            }
            let (open, position, in_window) = {
                let state = state.read(cx);
                (state.open && !entering, state.position, state.in_window)
            };
            let menu = (self.menu)(window, cx);
            // The menu opens below its trigger, a point here moved up and
            // left until the whole menu fits: the kit's popup then finds the
            // room below and neither flips nor scrolls it. Base's 8px margin
            // and the popup's 4px gap stay clear, with a little to spare.
            let rem_size = window.rem_size();
            let viewport = window.viewport_size();
            let height = rem(menu.visible_height()).to_pixels(rem_size);
            let width = rem(CONTEXT_MENU_WIDTH).to_pixels(rem_size);
            let edge = px(12.);
            let lowest = (viewport.height - edge - px(6.) - height).max(edge);
            let rightmost = (viewport.width - edge - width).max(edge);
            let left = position.x - (in_window.x - rightmost).max(px(0.));
            let top = position.y - (in_window.y - lowest).max(px(0.));
            let close = state.clone();
            let closed = self.on_open_change.clone();
            popup = Some(
                div().absolute().left(left).top(top).child(
                    MenuPopup::new((self.id, "popup"), div(), menu)
                        .open(open)
                        .on_open_change(move |next, window, cx| {
                            close.update(cx, |state, cx| {
                                state.open = *next;
                                cx.notify();
                            });
                            if let Some(closed) = &closed {
                                closed(*next, window, cx);
                            }
                        }),
                ),
            );
        }
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(press)
            .children(popup)
    }
}

/// Focuses something from inside a parent that renders before it, such as a
/// dialog that moves focus to its own surface on the frame it opens: the
/// work is deferred from this element's render, so it runs after the
/// parent's. Renders nothing.
#[derive(gpui_kit::IntoElement)]
pub struct FocusLater(Deferred);

type Deferred = Box<dyn FnOnce(&mut Window, &mut App)>;

impl FocusLater {
    pub fn new(focus: impl FnOnce(&mut Window, &mut App) + 'static) -> Self {
        Self(Box::new(focus))
    }
}

impl RenderOnce for FocusLater {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.defer(cx, self.0);
        div()
    }
}
