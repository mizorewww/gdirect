//! Small presentation helpers shared by the pages.
use crate::format::Kind;
use gpui_kit::{prelude::*, *};
use gpui_kit_material::icons::{Icon, Style, Symbol};
use gpui_kit_material::prelude::*;
use std::{cell::RefCell, collections::HashMap};

/// Applies a Material type role to text (the library keeps its own private).
pub fn type_role<E: Styled>(element: E, role: TypeRole) -> E {
    let scale = role.scale();
    element
        .font_family(scale.font_family)
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

/// A white card holding rows, as the settings panes and queues group them.
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
                .bg(color(ColorRole::SURFACE_CONTAINER_LOWEST))
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
        .trigger(line(TypeRole::BodyMedium, ink, content))
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
