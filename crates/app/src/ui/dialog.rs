//! Modal dialog content primitives.
//!
//! These build the *contents* of a `Modal` semantic surface (per
//! `docs/glossary/overlay-surfaces.md`). The `modal_overlay()` primitive
//! handles backdrop and stacking; this module owns the card, title, body,
//! and action row, so every confirmation / form dialog in the app reads as
//! a single visual family.
//!
//! Style follows GNOME HIG / libadwaita `AdwAlertDialog`:
//!
//! - Card uses `ContainerClass::DialogCard` (window-like opaque background,
//!   subtle drop shadow, larger rounding).
//! - Title in `TEXT_HEADING` semibold, body in `TEXT_MD` secondary text.
//! - Action row right-aligned. Cancel / dismiss appears on the LEFT,
//!   primary or destructive on the RIGHT, never the other way around.
//! - Action appearances: `Default` is flat (Ghost), `Suggested` is the
//!   accent fill (Suggested == Primary today), `Destructive` is the danger
//!   fill (white text on red).
//!
//! Two builders:
//!
//! - [`alert_dialog`] for plain confirmation copy (title + text body).
//! - [`form_dialog`] when the body is a custom element (input fields,
//!   inline errors, etc).
//!
//! See `docs/ui/overlay-standardization-plan.md` for the surrounding
//! context. Existing call sites should migrate to these builders rather
//! than rolling their own card layout.
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::font;
use crate::ui::layout::{
    DIALOG_CONFIRM_WIDTH, DIALOG_FORM_WIDTH, PAD_BUTTON, PAD_CARD, SPACE_LG, SPACE_SM, SPACE_XS,
    TEXT_HEADING, TEXT_MD,
};
use crate::ui::theme;

/// libadwaita-style appearance for a dialog action button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionAppearance {
    /// Flat / cancel-like button. No fill at rest.
    Default,
    /// Accent (primary) fill. Use for the recommended action.
    Suggested,
    /// Danger fill. Use for irreversible / destructive actions.
    Destructive,
}

/// One button in a dialog's action row.
///
/// Build via [`DialogAction::default_action`], [`DialogAction::suggested`],
/// or [`DialogAction::destructive`]. Pass `None` for `on_press` to render
/// the button disabled.
#[derive(Debug, Clone)]
pub struct DialogAction<M> {
    pub label: String,
    pub appearance: ActionAppearance,
    pub on_press: Option<M>,
}

impl<M: Clone> DialogAction<M> {
    pub fn default_action(label: impl Into<String>, on_press: M) -> Self {
        Self {
            label: label.into(),
            appearance: ActionAppearance::Default,
            on_press: Some(on_press),
        }
    }

    pub fn suggested(label: impl Into<String>, on_press: M) -> Self {
        Self {
            label: label.into(),
            appearance: ActionAppearance::Suggested,
            on_press: Some(on_press),
        }
    }

    pub fn destructive(label: impl Into<String>, on_press: M) -> Self {
        Self {
            label: label.into(),
            appearance: ActionAppearance::Destructive,
            on_press: Some(on_press),
        }
    }

    /// Replace `on_press` with `None` to render the button disabled.
    /// Used by callers that want a "Save"/"Insert" button to gray out
    /// while a required field is empty.
    #[must_use]
    pub fn disabled_when(mut self, disabled: bool) -> Self {
        if disabled {
            self.on_press = None;
        }
        self
    }
}

/// Build a confirmation dialog (title + plain text body + action row).
///
/// Width defaults to [`DIALOG_CONFIRM_WIDTH`]; pass `Some(_)` to override.
///
/// Actions render in left-to-right order. Place cancel-like actions first
/// and the primary / destructive action last so the rightmost button is
/// what GNOME HIG calls the "primary response".
pub fn alert_dialog<'a, M: Clone + 'a>(
    title: impl Into<String>,
    body: impl Into<String>,
    actions: Vec<DialogAction<M>>,
    width: Option<f32>,
) -> Element<'a, M> {
    let body_text: Element<'a, M> = text(body.into())
        .size(TEXT_MD)
        .style(iced::widget::text::secondary)
        .into();
    build_dialog_card(title.into(), body_text, actions, width.unwrap_or(DIALOG_CONFIRM_WIDTH))
}

/// Build a form dialog (title + custom body element + action row).
///
/// Use when the body needs input fields, inline error messages, or any
/// non-text content. The body is sandwiched between the title and the
/// action row exactly like [`alert_dialog`].
pub fn form_dialog<'a, M: Clone + 'a>(
    title: impl Into<String>,
    body: impl Into<Element<'a, M>>,
    actions: Vec<DialogAction<M>>,
    width: Option<f32>,
) -> Element<'a, M> {
    build_dialog_card(title.into(), body.into(), actions, width.unwrap_or(DIALOG_FORM_WIDTH))
}

fn build_dialog_card<'a, M: Clone + 'a>(
    title: String,
    body: Element<'a, M>,
    actions: Vec<DialogAction<M>>,
    width_px: f32,
) -> Element<'a, M> {
    let title_widget = text(title)
        .size(TEXT_HEADING)
        .font(font::text_semibold())
        .style(iced::widget::text::base);

    let action_row = build_action_row(actions);

    let content = column![
        title_widget,
        body,
        Space::new().height(SPACE_LG),
        action_row,
    ]
    .spacing(SPACE_SM);

    container(content)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::DialogCard.style())
        .width(Length::Fixed(width_px))
        .into()
}

fn build_action_row<'a, M: Clone + 'a>(actions: Vec<DialogAction<M>>) -> Element<'a, M> {
    // Right-aligned cluster: Space::Fill on the left pushes buttons to the
    // dialog's trailing edge. Buttons themselves are tightly spaced so the
    // group reads as one cluster, libadwaita-style.
    let mut row_widget = row![Space::new().width(Length::Fill)]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);
    for action in actions {
        row_widget = row_widget.push(render_action(action));
    }
    row_widget.into()
}

fn render_action<'a, M: Clone + 'a>(action: DialogAction<M>) -> Element<'a, M> {
    let label = text(action.label).size(TEXT_MD);
    let label = if matches!(action.appearance, ActionAppearance::Default) {
        label.style(iced::widget::text::base)
    } else {
        // Suggested / Destructive: white text on a colored fill.
        label.font(font::text_semibold()).color(theme::ON_AVATAR)
    };

    let style = match action.appearance {
        ActionAppearance::Default => theme::ButtonClass::Ghost.style(),
        ActionAppearance::Suggested => theme::ButtonClass::Suggested.style(),
        ActionAppearance::Destructive => theme::ButtonClass::Destructive.style(),
    };

    let mut btn = button(label).style(style).padding(PAD_BUTTON);
    if let Some(msg) = action.on_press {
        btn = btn.on_press(msg);
    }
    btn.into()
}
