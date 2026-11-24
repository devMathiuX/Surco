//! Temas y estilos de Surco. Un solo acento, sin sombras, bordes de 6 px.
//!
//! El tema activo vive en un índice global ([`set`] / [`colors`]): los estilos
//! son funciones `fn(&Theme, Status)` que iced llama al dibujar, y así todas
//! leen la misma paleta sin pasarla a mano por cada vista.

use std::sync::atomic::{AtomicUsize, Ordering};

use iced::theme::Palette;
use iced::widget::{button, container, slider, text, text_input, toggler};
use iced::{border, color, Color, Theme};

pub const SIZE_S: u32 = 13;
pub const SIZE_M: u32 = 15;
pub const SIZE_L: u32 = 20;
pub const RADIUS: f32 = 6.0;

/// Colores de un tema.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// Clave que se guarda en los ajustes.
    pub id: &'static str,
    /// Nombre que ve el usuario.
    pub name: &'static str,
    pub background: Color,
    /// Barra lateral y barra de reproducción.
    pub surface: Color,
    /// Elemento seleccionado o resaltado.
    pub raised: Color,
    /// Fondo al pasar el mouse.
    pub hover: Color,
    pub border: Color,
    /// Parte vacía de los sliders.
    pub rail: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub accent: Color,
    pub accent_hover: Color,
    /// Texto sobre el acento.
    pub on_accent: Color,
    pub danger: Color,
    pub warning: Color,
}

pub const THEMES: [Colors; 5] = [
    Colors {
        id: "dark",
        name: "Oscuro",
        background: color!(0x111111),
        surface: color!(0x1A1A1A),
        raised: color!(0x242424),
        hover: color!(0x262626),
        border: color!(0x2A2A2A),
        rail: color!(0x333333),
        text: color!(0xEDEDED),
        text_secondary: color!(0x8A8A8A),
        accent: color!(0x7C9CFF),
        accent_hover: color!(0x93AEFF),
        on_accent: color!(0x111111),
        danger: color!(0xE06C75),
        warning: color!(0xE5C07B),
    },
    Colors {
        id: "green",
        name: "Verde (estilo Spotify)",
        background: color!(0x121212),
        surface: color!(0x000000),
        raised: color!(0x282828),
        hover: color!(0x1F1F1F),
        border: color!(0x333333),
        rail: color!(0x4D4D4D),
        text: color!(0xFFFFFF),
        text_secondary: color!(0xB3B3B3),
        accent: color!(0x1DB954),
        accent_hover: color!(0x1ED760),
        on_accent: color!(0x000000),
        danger: color!(0xF15E6C),
        warning: color!(0xFFA42B),
    },
    Colors {
        id: "pink",
        name: "Rosa",
        background: color!(0x170F14),
        surface: color!(0x21151C),
        raised: color!(0x31202A),
        hover: color!(0x2A1B24),
        border: color!(0x3D2834),
        rail: color!(0x4A3240),
        text: color!(0xF8E9F0),
        text_secondary: color!(0xAD8D9D),
        accent: color!(0xFF6FAE),
        accent_hover: color!(0xFF8FC1),
        on_accent: color!(0x1A0E14),
        danger: color!(0xFF7A7A),
        warning: color!(0xF5C26B),
    },
    Colors {
        id: "light",
        name: "Claro",
        background: color!(0xFAFAFA),
        surface: color!(0xF0F0F0),
        raised: color!(0xE2E2E2),
        hover: color!(0xE8E8E8),
        border: color!(0xD4D4D4),
        rail: color!(0xD0D0D0),
        text: color!(0x1A1A1A),
        text_secondary: color!(0x6B6B6B),
        accent: color!(0x3D63DD),
        accent_hover: color!(0x2F52C4),
        on_accent: color!(0xFFFFFF),
        danger: color!(0xC9303E),
        warning: color!(0xA86A00),
    },
    Colors {
        id: "light-pink",
        name: "Claro rosa",
        background: color!(0xFFF7FA),
        surface: color!(0xFCECF3),
        raised: color!(0xF6D7E5),
        hover: color!(0xF9E1EC),
        border: color!(0xEBC6D6),
        rail: color!(0xE8C3D3),
        text: color!(0x2A1520),
        text_secondary: color!(0x8A6675),
        accent: color!(0xD6337A),
        accent_hover: color!(0xBF2A6A),
        on_accent: color!(0xFFFFFF),
        danger: color!(0xC9303E),
        warning: color!(0xA86A00),
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Colores del tema activo.
pub fn colors() -> &'static Colors {
    &THEMES[CURRENT.load(Ordering::Relaxed).min(THEMES.len() - 1)]
}

/// Cambia el tema activo. Devuelve el `Theme` de iced que le corresponde.
pub fn set(index: usize) -> Theme {
    CURRENT.store(index.min(THEMES.len() - 1), Ordering::Relaxed);
    iced_theme()
}

/// Índice del tema con esa clave.
pub fn index_of(id: &str) -> Option<usize> {
    THEMES.iter().position(|t| t.id == id)
}

/// `Theme` de iced para los widgets que no tienen estilo propio.
pub fn iced_theme() -> Theme {
    let c = colors();
    Theme::custom(
        format!("Surco {}", c.name),
        Palette {
            background: c.background,
            text: c.text,
            primary: c.accent,
            success: c.accent,
            warning: c.warning,
            danger: c.danger,
        },
    )
}

pub fn surface(_: &Theme) -> container::Style {
    container::Style {
        background: Some(colors().surface.into()),
        border: border::rounded(0),
        ..container::Style::default()
    }
}

/// Tarjeta (ajustes).
pub fn card(_: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(c.surface.into()),
        border: border::rounded(RADIUS).width(1).color(c.border),
        ..container::Style::default()
    }
}

pub fn secondary_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(colors().text_secondary),
    }
}

pub fn error_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(colors().danger),
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let c = colors();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: c.surface.into(),
        border: border::rounded(RADIUS)
            .width(1)
            .color(if focused { c.accent } else { c.border }),
        icon: c.text_secondary,
        placeholder: c.text_secondary,
        value: c.text,
        selection: c.accent.scale_alpha(0.35),
    }
}

pub fn primary_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => c.accent_hover,
        button::Status::Disabled => c.surface,
        button::Status::Active => c.accent,
    };
    button::Style {
        background: Some(bg.into()),
        text_color: c.on_accent,
        border: border::rounded(RADIUS),
        ..button::Style::default()
    }
}

pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| c.hover.into()),
        text_color: c.accent,
        border: border::rounded(RADIUS).width(1).color(c.accent),
        ..button::Style::default()
    }
}

/// Opción elegible (temas, precarga): la elegida, rellena con el acento.
pub fn choice_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if selected {
            primary_button(theme, button::Status::Active)
        } else {
            ghost_button(theme, status)
        }
    }
}

pub fn seek_slider(_: &Theme, status: slider::Status) -> slider::Style {
    let c = colors();
    let handle = match status {
        slider::Status::Dragged | slider::Status::Hovered => c.accent_hover,
        slider::Status::Active => c.accent,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (c.accent.into(), c.rail.into()),
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

pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let c = colors();
    let on = matches!(
        status,
        toggler::Status::Active { is_toggled: true }
            | toggler::Status::Hovered { is_toggled: true }
    );
    toggler::Style {
        background: if on { c.accent } else { c.rail }.into(),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: if on { c.on_accent } else { c.text }.into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.1,
    }
}

/// Elemento de la barra lateral; el seleccionado va resaltado.
pub fn list_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = colors();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if selected {
                Some(c.raised.into())
            } else {
                hovered.then(|| c.hover.into())
            },
            text_color: if selected { c.accent } else { c.text },
            border: border::rounded(RADIUS),
            ..button::Style::default()
        }
    }
}

/// Botón redondo de ícono (controles de reproducción).
pub fn icon_button(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| colors().hover.into()),
        border: border::rounded(999),
        ..button::Style::default()
    }
}

/// Botón principal redondo (play/pausa).
pub fn play_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => c.text.scale_alpha(0.85),
        _ => c.text,
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
        background: Some(colors().raised.into()),
        border: border::rounded(RADIUS),
        ..container::Style::default()
    }
}

/// Muestra del color de un tema (en el selector).
pub fn swatch(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border::rounded(999).width(1).color(colors().border),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_are_unique_and_found() {
        for (i, t) in THEMES.iter().enumerate() {
            assert_eq!(index_of(t.id), Some(i));
        }
        assert_eq!(index_of("nope"), None);
    }
}
