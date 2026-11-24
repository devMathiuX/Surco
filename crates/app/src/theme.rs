//! Paleta y estilos de Surco. Un solo acento, sin sombras, bordes de 6 px.

use iced::theme::Palette;
use iced::widget::{container, text};
use iced::{border, color, Color, Theme};

pub const BACKGROUND: Color = color!(0x111111);
pub const SURFACE: Color = color!(0x1A1A1A);
pub const TEXT: Color = color!(0xEDEDED);
pub const TEXT_SECONDARY: Color = color!(0x8A8A8A);
pub const ACCENT: Color = color!(0x7C9CFF);

pub const SIZE_S: u32 = 13;
pub const SIZE_M: u32 = 15;
pub const SIZE_L: u32 = 20;
pub const RADIUS: f32 = 6.0;

pub fn dark() -> Theme {
    Theme::custom(
        "Surco",
        Palette {
            background: BACKGROUND,
            text: TEXT,
            primary: ACCENT,
            success: ACCENT,
            warning: color!(0xE5C07B),
            danger: color!(0xE06C75),
        },
    )
}

pub fn surface(_: &Theme) -> container::Style {
    container::Style {
        background: Some(SURFACE.into()),
        border: border::rounded(0),
        ..container::Style::default()
    }
}

pub fn secondary_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(TEXT_SECONDARY),
    }
}

pub fn accent_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(ACCENT),
    }
}

/// Contenedor con borde de acento y radio estándar.
pub fn outlined(_: &Theme) -> container::Style {
    container::Style {
        border: border::rounded(RADIUS).color(ACCENT).width(1),
        ..container::Style::default()
    }
}
