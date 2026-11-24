//! Paleta y estilos de Surco. Un solo acento, sin sombras, bordes de 6 px.

use iced::theme::Palette;
use iced::widget::{button, container, slider, text, text_input};
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

pub fn error_text(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().danger),
    }
}

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: SURFACE.into(),
        border: border::rounded(RADIUS).width(1).color(if focused {
            ACCENT
        } else {
            color!(0x2A2A2A)
        }),
        icon: TEXT_SECONDARY,
        placeholder: TEXT_SECONDARY,
        value: theme.palette().text,
        selection: ACCENT.scale_alpha(0.35),
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0x93AEFF),
        button::Status::Disabled => SURFACE,
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: BACKGROUND,
        border: border::rounded(RADIUS),
        ..button::Style::default()
    }
}

pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| color!(0x222222).into()),
        text_color: ACCENT,
        border: border::rounded(RADIUS).width(1).color(ACCENT),
        ..button::Style::default()
    }
}

pub fn seek_slider(_: &Theme, status: slider::Status) -> slider::Style {
    let handle = match status {
        slider::Status::Dragged | slider::Status::Hovered => color!(0x93AEFF),
        slider::Status::Active => ACCENT,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (ACCENT.into(), color!(0x333333).into()),
            width: 4.0,
            border: border::rounded(2),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: handle.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// Elemento de la barra lateral; el seleccionado va resaltado.
pub fn list_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (selected || hovered).then(|| color!(0x242424).into()),
            text_color: if selected { ACCENT } else { TEXT },
            border: border::rounded(RADIUS),
            ..button::Style::default()
        }
    }
}

/// Botón redondo de ícono (controles de reproducción).
pub fn icon_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| color!(0x262626).into()),
        border: border::rounded(999),
        ..button::Style::default()
    }
}

/// Botón principal redondo (play/pausa).
pub fn play_button(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => color!(0xFFFFFF),
        _ => TEXT,
    };
    button::Style {
        background: Some(bg.into()),
        border: border::rounded(999),
        ..button::Style::default()
    }
}

/// Lugar de la portada mientras no hay imagen.
pub fn cover_placeholder(_: &Theme) -> container::Style {
    container::Style {
        background: Some(color!(0x262626).into()),
        border: border::rounded(RADIUS),
        ..container::Style::default()
    }
}
