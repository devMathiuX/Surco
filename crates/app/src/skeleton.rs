//! Esqueletos de carga: bloques grises con la forma de lo que va a aparecer
//! (filas de canciones, tarjetas), que "laten" mientras algo carga.
//!
//! El latido sale de `Surco::pulse`, que avanza con un timer solo mientras
//! hay algo cargando (ver `Surco::is_loading`).

use iced::widget::{column, container, row, Column, Space};
use iced::{Alignment, Element, Length};

use crate::{theme, Message};

/// Pasos de un latido completo (a ~8 por segundo, unos 2 s).
const CYCLE: u32 = 16;

/// Intensidad del latido, de 0 a 1 y de vuelta.
pub fn level(pulse: u32) -> f32 {
    let half = CYCLE / 2;
    let t = (pulse % CYCLE) as f32 / half as f32;
    if t > 1.0 {
        2.0 - t
    } else {
        t
    }
}

/// Un bloque de `width` × `height`.
pub fn block<'a>(width: impl Into<Length>, height: f32, pulse: u32) -> Element<'a, Message> {
    container(Space::new())
        .width(width)
        .height(height)
        .style(theme::skeleton(level(pulse)))
        .into()
}

/// Barra que ocupa el `percent` % del ancho disponible.
fn bar<'a>(percent: u16, height: f32, pulse: u32) -> Element<'a, Message> {
    let percent = percent.clamp(1, 99);
    row![
        block(Length::FillPortion(percent), height, pulse),
        Space::new().width(Length::FillPortion(100 - percent)),
    ]
    .into()
}

/// `n` filas de canción: portada, título, artista y duración.
pub fn song_rows<'a>(n: usize, pulse: u32) -> Element<'a, Message> {
    // Anchos distintos para que no parezca una grilla.
    const WIDTHS: [u16; 5] = [55, 38, 70, 47, 62];
    Column::with_children((0..n).map(|i| {
        let w = WIDTHS[i % WIDTHS.len()];
        row![
            block(36, 36.0, pulse),
            column![bar(w, 12.0, pulse), bar(w / 2 + 10, 10.0, pulse)]
                .spacing(8)
                .width(Length::Fill),
            block(34, 10.0, pulse),
        ]
        .spacing(12)
        .padding([6, 8])
        .align_y(Alignment::Center)
        .into()
    }))
    .spacing(2)
    .into()
}

/// Encabezado de playlist: nombre y detalle.
pub fn header<'a>(pulse: u32) -> Element<'a, Message> {
    column![bar(35, 20.0, pulse), bar(50, 12.0, pulse)]
        .spacing(10)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_goes_up_and_down() {
        assert_eq!(level(0), 0.0);
        assert_eq!(level(CYCLE / 2), 1.0);
        assert!(level(CYCLE / 4) > 0.4 && level(CYCLE / 4) < 0.6);
        assert_eq!(level(CYCLE), 0.0);
        assert!((0..100).all(|p| (0.0..=1.0).contains(&level(p))));
    }
}
