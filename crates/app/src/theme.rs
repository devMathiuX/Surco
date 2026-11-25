//! Temas y estilos de Surco. Un solo acento, sin sombras, bordes de 6 px.
//!
//! El tema activo vive en un índice global ([`set`] / [`colors`]): los estilos
//! son funciones `fn(&Theme, Status)` que iced llama al dibujar, y así todas
//! leen la misma paleta sin pasarla a mano por cada vista.

use std::sync::{PoisonError, RwLock};

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

/// Índice del tema personalizado: va después de los de [`THEMES`].
pub const CUSTOM: usize = THEMES.len();

/// Colores de partida del tema personalizado.
pub const CUSTOM_DEFAULT: (Color, Color) = (color!(0x14161C), color!(0xF2A65A));

/// Tema activo: una entrada de [`THEMES`] o un tema personalizado armado con
/// [`derive`], que se filtra a propósito (~150 bytes por cada color que el
/// usuario elige) para que [`colors`] pueda devolver una referencia estática.
static ACTIVE: RwLock<&'static Colors> = RwLock::new(&THEMES[0]);

/// Colores del tema activo.
pub fn colors() -> &'static Colors {
    *ACTIVE.read().unwrap_or_else(PoisonError::into_inner)
}

/// Cambia el tema activo. `custom` son los dos colores (fondo y acento) del
/// tema personalizado, que se usan si `index` es [`CUSTOM`]. Devuelve el
/// `Theme` de iced que le corresponde.
pub fn set(index: usize, custom: (Color, Color)) -> Theme {
    let active: &'static Colors = if index >= CUSTOM {
        Box::leak(Box::new(derive(custom.0, custom.1)))
    } else {
        &THEMES[index]
    };
    *ACTIVE.write().unwrap_or_else(PoisonError::into_inner) = active;
    iced_theme()
}

/// Índice del tema con esa clave.
pub fn index_of(id: &str) -> Option<usize> {
    if id == CUSTOM_ID {
        return Some(CUSTOM);
    }
    THEMES.iter().position(|t| t.id == id)
}

/// Clave del tema `index` para guardarla en los ajustes.
pub fn id_of(index: usize) -> &'static str {
    THEMES.get(index).map_or(CUSTOM_ID, |t| t.id)
}

const CUSTOM_ID: &str = "custom";

/// Nombre del tema en el idioma elegido.
pub fn name_of(index: usize) -> &'static str {
    match THEMES.get(index).map(|t| t.id) {
        Some("dark") => tr!("Oscuro", "Dark"),
        Some("green") => tr!("Verde", "Green"),
        Some("pink") => tr!("Rosa", "Pink"),
        Some("light") => tr!("Claro", "Light"),
        Some("light-pink") => tr!("Claro rosa", "Light pink"),
        Some(other) => other,
        None => tr!("Personalizado", "Custom"),
    }
}

/// Arma un tema completo a partir de dos colores: el fondo y el acento. Las
/// superficies salen de aclarar (fondo oscuro) u oscurecer (fondo claro) el
/// fondo, y los textos se eligen para que se lean sobre él.
pub fn derive(background: Color, accent: Color) -> Colors {
    let background = Color {
        a: 1.0,
        ..background
    };
    let accent = Color { a: 1.0, ..accent };
    let dark = luminance(background) < 0.4;
    let toward = if dark { Color::WHITE } else { Color::BLACK };
    let text = if dark {
        color!(0xEDEDED)
    } else {
        color!(0x1A1A1A)
    };
    let base = if dark { THEMES[0] } else { THEMES[3] };
    Colors {
        id: CUSTOM_ID,
        background,
        surface: mix(background, toward, 0.04),
        raised: mix(background, toward, 0.10),
        hover: mix(background, toward, 0.07),
        border: mix(background, toward, 0.12),
        rail: mix(background, toward, 0.20),
        text,
        text_secondary: mix(background, text, 0.55),
        accent,
        accent_hover: mix(accent, toward, 0.18),
        on_accent: if luminance(accent) > 0.45 {
            color!(0x111111)
        } else {
            Color::WHITE
        },
        danger: base.danger,
        warning: base.warning,
    }
}

/// Lee un color `#RRGGBB` o `#RGB` (el `#` es opcional).
pub fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let s = match s.len() {
        6 => s.to_owned(),
        3 => s.chars().flat_map(|c| [c, c]).collect(),
        _ => return None,
    };
    let v = u32::from_str_radix(&s, 16).ok()?;
    Some(Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Escribe un color como `#RRGGBB`.
pub fn to_hex(c: Color) -> String {
    let [r, g, b, _] = c.into_rgba8();
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Luminancia relativa (0 negro, 1 blanco).
fn luminance(c: Color) -> f32 {
    let lin = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// `Theme` de iced para los widgets que no tienen estilo propio.
pub fn iced_theme() -> Theme {
    let c = colors();
    Theme::custom(
        format!("Surco {}", c.id),
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

pub fn accent_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(colors().accent),
    }
}

pub fn plain_text(_: &Theme) -> text::Style {
    text::Style {
        color: Some(colors().text),
    }
}

/// Fila de la cola: la que suena resaltada, el destino del arrastre con una
/// línea de acento arriba, la arrastrada más tenue.
pub fn queue_row(current: bool, target: bool, lifted: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let c = colors();
        let background = if lifted {
            Some(c.hover.into())
        } else if current {
            Some(c.raised.into())
        } else {
            None
        };
        container::Style {
            background,
            border: if target {
                border::rounded(RADIUS).width(2).color(c.accent)
            } else {
                border::rounded(RADIUS)
            },
            ..container::Style::default()
        }
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

/// Campo con un valor que no se puede usar (borde rojo).
pub fn input_error(theme: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        border: border::rounded(RADIUS).width(1).color(colors().danger),
        ..input(theme, status)
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

/// Mezcla de dos colores (`t` = 0 es `a`, 1 es `b`).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: 1.0,
    }
}

/// Bloque de un esqueleto de carga; `level` (0 a 1) lo hace "latir".
pub fn skeleton(level: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let c = colors();
        container::Style {
            background: Some(mix(c.raised, c.rail, level * 0.8).into()),
            border: border::rounded(4),
            ..container::Style::default()
        }
    }
}

/// Fila que se puede arrastrar (barra lateral): elegida, debajo del mouse,
/// destino del arrastre (línea de acento) o la que se arrastra.
pub fn drag_row(
    selected: bool,
    hovered: bool,
    target: bool,
    lifted: bool,
) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let c = colors();
        let background = if selected {
            Some(c.raised.into())
        } else if hovered || lifted {
            Some(c.hover.into())
        } else {
            None
        };
        container::Style {
            background,
            text_color: Some(if selected { c.accent } else { c.text }),
            border: if target {
                border::rounded(RADIUS).width(2).color(c.accent)
            } else {
                border::rounded(RADIUS)
            },
            ..container::Style::default()
        }
    }
}

/// Tarjeta clickeable de la pantalla de inicio.
pub fn tile(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { c.hover } else { c.surface }.into()),
        text_color: c.text,
        border: border::rounded(RADIUS),
        ..button::Style::default()
    }
}

/// Diálogo flotante.
pub fn dialog(_: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(c.background.into()),
        // Sin sombra: con tiny-skia, al redibujar solo la zona que cambió
        // (pasar el mouse por las opciones) la sombra difusa se acumulaba y
        // oscurecía el diálogo entero.
        border: border::rounded(RADIUS + 2.0).width(1).color(c.border),
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
        assert_eq!(index_of(id_of(CUSTOM)), Some(CUSTOM));
    }

    #[test]
    fn hex_round_trips() {
        let c = parse_hex("#1db954").unwrap();
        assert_eq!(to_hex(c), "#1DB954");
        assert_eq!(parse_hex("FFFFFF"), Some(Color::WHITE));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#7cf"), parse_hex("#77CCFF"));
        assert_eq!(parse_hex("#GGGGGG"), None);
    }

    #[test]
    fn custom_theme_text_is_readable() {
        let dark = derive(color!(0x101010), color!(0xFFD000));
        assert!(luminance(dark.text) > 0.7);
        assert!(luminance(dark.surface) > luminance(dark.background));
        assert_eq!(dark.on_accent, color!(0x111111));
        let light = derive(color!(0xF5F0E6), color!(0x2040A0));
        assert!(luminance(light.text) < 0.1);
        assert!(luminance(light.surface) < luminance(light.background));
        assert_eq!(light.on_accent, Color::WHITE);
    }
}
