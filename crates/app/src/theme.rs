//! Temas y estilos de Surco. Un solo acento; cada tema elige su forma
//! ([`Look`]): plano, vidrio, neo-brutalista o brillo.
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

/// Fuente en negrita (títulos del tema Brutal).
pub const BOLD: iced::Font = iced::Font {
    weight: iced::font::Weight::Bold,
    ..iced::Font::DEFAULT
};

/// Forma de un tema, más allá de los colores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Colores lisos (los temas de siempre).
    Flat,
    /// Fondo de degradado con paneles translúcidos.
    Glass,
    /// Bordes duros, sin redondeo, sombras planas.
    Brutal,
    /// Oscuro minimalista con un brillo de color detrás.
    Glow,
}

/// Colores de un tema.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// Clave que se guarda en los ajustes.
    pub id: &'static str,
    pub look: Look,
    /// Redondeo de botones, campos y tarjetas.
    pub radius: f32,
    /// Ancho de los bordes.
    pub border_width: f32,
    /// Fondo de diálogos y menús flotantes (opaco, aun en temas translúcidos).
    pub panel: Color,
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

pub const THEMES: [Colors; 8] = [
    Colors {
        id: "dark",
        look: Look::Flat,
        radius: 6.0,
        border_width: 1.0,
        background: color!(0x0D1016),
        panel: color!(0x131722),
        surface: color!(0x10131A),
        raised: color!(0x1A2030),
        hover: color!(0x171C27),
        border: color!(0x232A37),
        rail: color!(0x2A3242),
        text: color!(0xE8EBF1),
        text_secondary: color!(0x8B94A5),
        accent: color!(0x4C7EF3),
        accent_hover: color!(0x6A95F7),
        on_accent: color!(0xFFFFFF),
        danger: color!(0xE06C75),
        warning: color!(0xE5C07B),
    },
    Colors {
        id: "green",
        look: Look::Flat,
        radius: 6.0,
        border_width: 1.0,
        background: color!(0x121212),
        panel: color!(0x121212),
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
        look: Look::Flat,
        radius: 6.0,
        border_width: 1.0,
        background: color!(0x170F14),
        panel: color!(0x170F14),
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
        look: Look::Flat,
        radius: 6.0,
        border_width: 1.0,
        background: color!(0xFAFAFA),
        panel: color!(0xFAFAFA),
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
        look: Look::Flat,
        radius: 6.0,
        border_width: 1.0,
        background: color!(0xFFF7FA),
        panel: color!(0xFFF7FA),
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
    Colors {
        id: "glass",
        look: Look::Glass,
        radius: 12.0,
        border_width: 1.0,
        background: color!(0x111827),
        panel: Color::from_rgba(0.09, 0.10, 0.17, 0.97),
        surface: Color::from_rgba(1.0, 1.0, 1.0, 0.07),
        raised: Color::from_rgba(1.0, 1.0, 1.0, 0.15),
        hover: Color::from_rgba(1.0, 1.0, 1.0, 0.10),
        border: Color::from_rgba(1.0, 1.0, 1.0, 0.16),
        rail: Color::from_rgba(1.0, 1.0, 1.0, 0.22),
        text: color!(0xF4F6FB),
        text_secondary: Color::from_rgba(0.90, 0.93, 1.0, 0.66),
        accent: color!(0x9DB2FF),
        accent_hover: color!(0xB7C6FF),
        on_accent: color!(0x0E1020),
        danger: color!(0xFF8A8A),
        warning: color!(0xF5C26B),
    },
    Colors {
        id: "brutal",
        look: Look::Brutal,
        radius: 0.0,
        border_width: 2.0,
        background: color!(0x101010),
        panel: color!(0x151515),
        surface: color!(0x151515),
        raised: color!(0x1E1E1E),
        hover: color!(0x242424),
        border: color!(0x3A3A3A),
        rail: color!(0x3A3A3A),
        text: color!(0xFFFFFF),
        text_secondary: color!(0xA3A3A3),
        accent: color!(0xB4F233),
        accent_hover: color!(0xC8FF55),
        on_accent: color!(0x000000),
        danger: color!(0xFF5C5C),
        warning: color!(0xFFD400),
    },
    Colors {
        id: "glow",
        look: Look::Glow,
        radius: 10.0,
        border_width: 1.0,
        background: color!(0x09090F),
        panel: color!(0x13131C),
        surface: Color::from_rgba(1.0, 1.0, 1.0, 0.03),
        raised: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
        hover: Color::from_rgba(1.0, 1.0, 1.0, 0.05),
        border: Color::from_rgba(1.0, 1.0, 1.0, 0.08),
        rail: Color::from_rgba(1.0, 1.0, 1.0, 0.14),
        text: color!(0xECECF4),
        text_secondary: color!(0x8B8BA3),
        accent: color!(0x8B7CF6),
        accent_hover: color!(0xA497FF),
        on_accent: color!(0x0B0B12),
        danger: color!(0xFF7A8A),
        warning: color!(0xF5C26B),
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
        Some("glass") => tr!("Vidrio", "Glass"),
        Some("brutal") => tr!("Brutal", "Brutal"),
        Some("glow") => tr!("Brillo", "Glow"),
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
        look: Look::Flat,
        radius: RADIUS,
        border_width: 1.0,
        background,
        panel: background,
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

/// Redondeo del tema activo.
pub fn radius() -> f32 {
    colors().radius
}

/// Sombra de botones y tarjetas: ninguna. Con tiny-skia las sombras (aun
/// sin difuminar) se dibujaban encima del contenido de las tarjetas con un
/// botón adentro y dejaban restos al redibujar, así que Brutal marca los
/// bordes con color en lugar de sombra.
fn hard_shadow() -> iced::Shadow {
    iced::Shadow::default()
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
        border: border::rounded(radius())
            .width(colors().border_width)
            .color(c.border),
        shadow: hard_shadow(),
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
    move |theme| {
        let c = colors();
        let status = if lifted {
            button::Status::Hovered
        } else {
            button::Status::Active
        };
        let row = song_row(current)(theme, status);
        container::Style {
            background: row.background,
            border: if target {
                border::rounded(radius())
                    .width(c.border_width.max(2.0))
                    .color(c.accent)
            } else {
                row.border
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
        background: match c.look {
            Look::Glass => Color::from_rgba(1.0, 1.0, 1.0, 0.10).into(),
            Look::Flat => mix(c.background, c.raised, 0.5).into(),
            _ => c.surface.into(),
        },
        border: border::rounded(if c.look == Look::Glass {
            999.0
        } else {
            radius()
        })
        .width(colors().border_width)
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
        border: border::rounded(radius())
            .width(colors().border_width)
            .color(colors().danger),
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
        border: border::rounded(radius()),
        shadow: hard_shadow(),
        ..button::Style::default()
    }
}

/// Acción que borra algo: rellena con el color de peligro.
pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let alpha = match status {
        button::Status::Hovered | button::Status::Pressed => 0.85,
        _ => 1.0,
    };
    button::Style {
        background: Some(
            Color {
                a: alpha,
                ..c.danger
            }
            .into(),
        ),
        text_color: Color::WHITE,
        border: border::rounded(radius()),
        ..button::Style::default()
    }
}

/// Velo oscuro detrás de un diálogo.
pub fn scrim(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
        ..container::Style::default()
    }
}

pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then(|| c.hover.into()),
        text_color: c.accent,
        border: border::rounded(radius())
            .width(colors().border_width)
            .color(c.accent),
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
            backgrounds: (
                match c.look {
                    Look::Glass | Look::Glow => accent_gradient(1.0),
                    _ => c.accent.into(),
                },
                c.rail.into(),
            ),
            width: 4.0,
            border: border::rounded(2),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 6.0 },
            background: if c.look == Look::Glass {
                Color::WHITE.into()
            } else {
                handle.into()
            },
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
        match c.look {
            // Cada opción en su caja; la elegida rellena con el acento y sombra.
            Look::Brutal => button::Style {
                background: Some(
                    if selected {
                        c.accent
                    } else if hovered {
                        c.hover
                    } else {
                        c.surface
                    }
                    .into(),
                ),
                text_color: if selected { c.on_accent } else { c.text },
                border: border::rounded(0).width(c.border_width).color(if selected {
                    Color::BLACK
                } else {
                    c.border
                }),
                shadow: if selected {
                    hard_shadow()
                } else {
                    iced::Shadow::default()
                },
                ..button::Style::default()
            },
            _ => button::Style {
                background: if selected {
                    Some(tint().into())
                } else {
                    hovered.then(|| c.hover.into())
                },
                text_color: c.text,
                border: if selected {
                    border::rounded(radius()).width(1).color(tint_border())
                } else {
                    border::rounded(radius())
                },
                ..button::Style::default()
            },
        }
    }
}

/// Fondo de lo elegido: el acento muy suave (vidrio: blanco translúcido).
fn tint() -> Color {
    let c = colors();
    match c.look {
        Look::Glass => Color::from_rgba(1.0, 1.0, 1.0, 0.16),
        Look::Glow => Color::from_rgba(1.0, 1.0, 1.0, 0.07),
        _ => c.accent.scale_alpha(0.14),
    }
}

/// Borde de lo elegido.
fn tint_border() -> Color {
    let c = colors();
    match c.look {
        Look::Glass => Color::from_rgba(1.0, 1.0, 1.0, 0.30),
        Look::Glow => Color::from_rgba(1.0, 1.0, 1.0, 0.12),
        _ => c.accent.scale_alpha(0.35),
    }
}

/// Fila de una lista de canciones; la que suena, resaltada.
pub fn song_row(playing: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let c = colors();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (background, border) = match c.look {
            Look::Brutal => (
                if hovered { c.hover } else { c.surface },
                border::rounded(0).width(c.border_width).color(if playing {
                    c.accent
                } else {
                    c.border
                }),
            ),
            Look::Glass => (
                Color::from_rgba(
                    1.0,
                    1.0,
                    1.0,
                    if playing {
                        0.16
                    } else if hovered {
                        0.11
                    } else {
                        0.06
                    },
                ),
                border::rounded(8).width(1).color(if playing {
                    Color::from_rgba(1.0, 1.0, 1.0, 0.30)
                } else {
                    Color::TRANSPARENT
                }),
            ),
            Look::Glow => (
                if playing {
                    c.raised
                } else if hovered {
                    c.hover
                } else {
                    Color::TRANSPARENT
                },
                border::rounded(radius()).width(1).color(if playing {
                    tint_border()
                } else {
                    Color::TRANSPARENT
                }),
            ),
            Look::Flat => (
                if playing {
                    c.raised
                } else if hovered {
                    c.hover
                } else {
                    row_color()
                },
                border::rounded(4),
            ),
        };
        button::Style {
            background: Some(background.into()),
            text_color: c.text,
            border,
            ..button::Style::default()
        }
    }
}

/// Fondo de las filas en los temas lisos: apenas distinto del fondo, y la
/// separación entre filas hace de línea divisoria.
fn row_color() -> Color {
    let c = colors();
    mix(c.background, c.surface, 0.8)
}

/// Separación entre filas de canciones.
pub fn row_gap() -> u32 {
    match colors().look {
        Look::Brutal => 6,
        Look::Glass => 4,
        Look::Glow => 2,
        Look::Flat => 1,
    }
}

/// Separación entre los botones de navegación.
pub fn nav_gap() -> u32 {
    if colors().look == Look::Brutal {
        8
    } else {
        2
    }
}

/// Color del ícono dentro del botón de play.
pub fn play_icon() -> Color {
    let c = colors();
    match c.look {
        Look::Flat => c.accent,
        Look::Glass => color!(0x1A1D2E),
        Look::Brutal => Color::BLACK,
        Look::Glow => Color::WHITE,
    }
}

/// Degradado del acento (Brillo y Vidrio): de un tono más frío al acento.
fn accent_gradient(alpha: f32) -> iced::Background {
    let c = colors();
    let from = match c.look {
        Look::Glass => color!(0x7FB2FF),
        _ => color!(0x5B8CFF),
    };
    let to = match c.look {
        Look::Glass => color!(0xC08CFF),
        _ => c.accent,
    };
    iced::Gradient::Linear(
        iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2)
            .add_stop(0.0, from.scale_alpha(alpha))
            .add_stop(1.0, to.scale_alpha(alpha)),
    )
    .into()
}

/// Botón redondo de play en las tarjetas de playlist.
pub fn tile_play(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let k = if hovered { 0.85 } else { 1.0 };
    let (background, border) = match c.look {
        Look::Brutal => (
            c.accent.scale_alpha(k).into(),
            border::rounded(999).width(2).color(Color::BLACK),
        ),
        Look::Glass | Look::Glow => (
            Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.32 } else { 0.22 }).into(),
            border::rounded(999)
                .width(1)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.35)),
        ),
        Look::Flat => (c.accent.scale_alpha(k).into(), border::rounded(999)),
    };
    button::Style {
        background: Some(background),
        border,
        ..button::Style::default()
    }
}

/// Color del ícono del play de las tarjetas.
pub fn tile_play_icon() -> Color {
    let c = colors();
    match c.look {
        Look::Brutal => Color::BLACK,
        Look::Glass | Look::Glow => Color::WHITE,
        Look::Flat => c.on_accent,
    }
}

/// Etiqueta con la cantidad de canciones (Brutal: recuadro amarillo).
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        background: Some(colors().warning.into()),
        text_color: Some(Color::BLACK),
        ..container::Style::default()
    }
}

/// Logo y engranaje del tema Brutal: en caja con sombra plana.
pub fn brand_button(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { c.hover } else { c.surface }.into()),
        text_color: c.text,
        border: border::rounded(0).width(2).color(c.text_secondary),
        shadow: hard_shadow(),
        ..button::Style::default()
    }
}

/// Botón sin fondo ni borde.
pub fn plain_button(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        text_color: colors().text,
        ..button::Style::default()
    }
}

/// Línea divisoria de `color`.
pub fn rule(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        ..container::Style::default()
    }
}

/// Barra lateral.
pub fn sidebar(_: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(
            match c.look {
                Look::Glass => Color::from_rgba(1.0, 1.0, 1.0, 0.08),
                Look::Glow => Color::from_rgba(0.0, 0.0, 0.0, 0.25),
                _ => c.surface,
            }
            .into(),
        ),
        ..container::Style::default()
    }
}

/// Barra de reproducción.
pub fn player_bar(_: &Theme) -> container::Style {
    let c = colors();
    match c.look {
        Look::Glass => container::Style {
            background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.10).into()),
            border: border::rounded(14)
                .width(1)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.22)),
            ..container::Style::default()
        },
        Look::Glow => container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.30).into()),
            ..container::Style::default()
        },
        _ => container::Style {
            background: Some(c.surface.into()),
            ..container::Style::default()
        },
    }
}

/// Panel principal del tema Vidrio (barra lateral más contenido).
pub fn glass_panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.04).into()),
        border: border::rounded(14)
            .width(1)
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.20)),
        ..container::Style::default()
    }
}

/// Línea que separa barra lateral, contenido y barra de reproducción.
pub fn divider() -> Option<(Color, f32)> {
    let c = colors();
    match c.look {
        Look::Brutal => Some((c.accent, 3.0)),
        Look::Flat | Look::Glow => Some((c.border, 1.0)),
        Look::Glass => None,
    }
}

/// Botones cuadrados de la derecha de la barra (Brutal: en caja).
pub fn side_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    if c.look != Look::Brutal {
        return icon_button(theme, status);
    }
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { c.hover } else { c.background }.into()),
        border: border::rounded(0).width(2).color(c.text),
        ..button::Style::default()
    }
}

/// Botón "+" de las filas (Brutal: cuadrado blanco).
pub fn add_button(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    if c.look != Look::Brutal {
        return icon_button(theme, status);
    }
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { c.accent } else { Color::WHITE }.into()),
        border: border::rounded(0).width(1).color(Color::BLACK),
        ..button::Style::default()
    }
}

/// Color del "+" de las filas.
pub fn add_icon() -> Color {
    let c = colors();
    if c.look == Look::Brutal {
        Color::BLACK
    } else {
        c.text_secondary
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
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let k = if hovered { 0.85 } else { 1.0 };
    let (background, border): (iced::Background, iced::Border) = match c.look {
        // Por defecto: acento suave con borde del acento.
        Look::Flat => (
            c.accent
                .scale_alpha(if hovered { 0.30 } else { 0.20 })
                .into(),
            border::rounded(999).width(1.5).color(c.accent),
        ),
        Look::Glass => (Color::WHITE.scale_alpha(k).into(), border::rounded(999)),
        Look::Glow => (accent_gradient(k), border::rounded(999)),
        // Sin sombra: en un botón redondo la sombra plana dejaba restos al
        // redibujar.
        Look::Brutal => (
            c.accent.scale_alpha(k).into(),
            border::rounded(999).width(2).color(Color::BLACK),
        ),
    };
    button::Style {
        background: Some(background),
        border,
        ..button::Style::default()
    }
}

/// Lugar de la portada mientras no hay imagen.
pub fn cover_placeholder(_: &Theme) -> container::Style {
    container::Style {
        background: Some(colors().raised.into()),
        border: border::rounded(radius()),
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
        a: a.a + (b.a - a.a) * t,
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
                border::rounded(radius())
                    .width(colors().border_width.max(2.0))
                    .color(c.accent)
            } else {
                border::rounded(radius())
            },
            ..container::Style::default()
        }
    }
}

/// Tarjeta clickeable de la pantalla de inicio.
pub fn tile(_: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (background, border): (iced::Background, iced::Border) = match c.look {
        Look::Brutal => (
            if hovered { c.hover } else { c.surface }.into(),
            border::rounded(0).width(c.border_width).color(c.accent),
        ),
        Look::Glass => (
            Color::from_rgba(1.0, 1.0, 1.0, if hovered { 0.18 } else { 0.12 }).into(),
            border::rounded(radius())
                .width(1)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.28)),
        ),
        // Brillo: degradado de azul petróleo a violeta.
        Look::Glow => (
            iced::Gradient::Linear(
                iced::gradient::Linear::new(std::f32::consts::FRAC_PI_2)
                    .add_stop(
                        0.0,
                        color!(0x163447).scale_alpha(if hovered { 1.0 } else { 0.9 }),
                    )
                    .add_stop(
                        1.0,
                        color!(0x2B2560).scale_alpha(if hovered { 1.0 } else { 0.9 }),
                    ),
            )
            .into(),
            border::rounded(radius())
                .width(1)
                .color(Color::from_rgba(1.0, 1.0, 1.0, 0.14)),
        ),
        Look::Flat => (
            if hovered {
                c.accent.scale_alpha(0.16)
            } else {
                c.accent.scale_alpha(0.09)
            }
            .into(),
            border::rounded(radius())
                .width(1)
                .color(c.accent.scale_alpha(0.32)),
        ),
    };
    button::Style {
        background: Some(background),
        text_color: c.text,
        border,
        shadow: hard_shadow(),
        ..button::Style::default()
    }
}

/// Diálogo flotante.
pub fn dialog(_: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(c.panel.into()),
        // Sin sombra: con tiny-skia, al redibujar solo la zona que cambió
        // (pasar el mouse por las opciones) la sombra difusa se acumulaba y
        // oscurecía el diálogo entero.
        border: border::rounded(radius() + 2.0)
            .width(colors().border_width)
            .color(c.border),
        ..container::Style::default()
    }
}

/// Muestra del color de un tema (en el selector).
pub fn swatch(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border::rounded(999)
            .width(colors().border_width)
            .color(colors().border),
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
