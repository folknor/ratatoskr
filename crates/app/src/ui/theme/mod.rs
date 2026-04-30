#![allow(dead_code)] // Theme infrastructure; some variants and TOML loader not yet wired in.

use std::cell::Cell;

use iced::Color;

use crate::ui::settings::types::EmailBodyBackground;

mod avatar;
mod button;
mod catalog;
mod color;
mod container;
mod forms;

// Style functions only receive `&Theme`, so we use a thread-local to
// communicate the user's preference to `style_email_body_container`.
thread_local! {
    pub(crate) static EMAIL_BODY_BG_PREF: Cell<EmailBodyBackground> = const { Cell::new(EmailBodyBackground::AlwaysWhite) };
}

/// Set the email body background preference. Call when preferences change.
pub fn set_email_body_background(pref: EmailBodyBackground) {
    EMAIL_BODY_BG_PREF.set(pref);
}

/// Text/icon color on top of avatar circles and primary buttons.
pub const ON_AVATAR: Color = Color::WHITE;

pub use avatar::{avatar_color, initial};
pub use button::{
    ButtonClass, RowPosition, style_filter_container, style_pill_card_button,
    style_recessed_list_panel, style_settings_row_button,
};
pub use catalog::{THEMES, theme_by_index};
pub use color::hex_to_color;
pub use container::ContainerClass;
pub use forms::{
    PickListClass, RadioClass, RuleClass, SliderClass, TextClass, TextInputClass, TogglerClass,
};
