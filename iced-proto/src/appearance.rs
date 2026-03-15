use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::stream::BoxStream;
use iced::futures::StreamExt;
use iced::{Subscription, Theme};
use mundy::{ColorScheme, Interest, Preferences};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
    Unspecified,
}

impl From<ColorScheme> for Mode {
    fn from(scheme: ColorScheme) -> Self {
        match scheme {
            ColorScheme::Dark => Mode::Dark,
            ColorScheme::Light => Mode::Light,
            ColorScheme::NoPreference => Mode::Unspecified,
        }
    }
}

impl Mode {
    pub fn theme(self) -> Theme {
        match self {
            Mode::Dark | Mode::Unspecified => crate::ui::theme::dark(),
            Mode::Light => crate::ui::theme::light(),
        }
    }
}

struct Appearance;

impl subscription::Recipe for Appearance {
    type Output = Mode;

    fn hash(&self, state: &mut Hasher) {
        use std::hash::Hash;
        struct Marker;
        std::any::TypeId::of::<Marker>().hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: subscription::EventStream,
    ) -> BoxStream<'static, Mode> {
        Preferences::stream(Interest::ColorScheme)
            .map(|preference| Mode::from(preference.color_scheme))
            .boxed()
    }
}

pub fn subscription() -> Subscription<Mode> {
    subscription::from_recipe(Appearance)
}
