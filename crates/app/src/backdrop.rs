//! Fondos dibujados por código para los temas Vidrio y Brillo: manchas de
//! color con caída suave, generadas una sola vez en baja resolución y
//! estiradas a la ventana (los degradados suaves no pierden nada al
//! estirarse). Sin imágenes en el binario.

use std::sync::Mutex;

use iced::widget::image::Handle;

use crate::theme::{self, Look};

/// Resolución del fondo: ~520 KB en RAM, se estira con filtro lineal.
const W: u32 = 480;
const H: u32 = 270;

/// Mancha de color: centro (0 a 1), radio (en alto de la imagen) y fuerza.
struct Blob {
    x: f32,
    y: f32,
    r: f32,
    color: [f32; 3],
    strength: f32,
}

const fn rgb(v: u32) -> [f32; 3] {
    [
        ((v >> 16) & 0xFF) as f32 / 255.0,
        ((v >> 8) & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
    ]
}

/// Degradado de colores del tema Vidrio (verde azulado, violeta, rosa y
/// dorado sobre azul noche).
const GLASS: ([f32; 3], &[Blob]) = (
    rgb(0x0B1324),
    &[
        Blob {
            x: 0.12,
            y: 0.20,
            r: 0.95,
            color: rgb(0x1E8C8C),
            strength: 0.85,
        },
        Blob {
            x: 0.92,
            y: 0.08,
            r: 0.80,
            color: rgb(0x23307A),
            strength: 0.70,
        },
        Blob {
            x: 0.58,
            y: 0.48,
            r: 0.70,
            color: rgb(0x7B3FB3),
            strength: 0.80,
        },
        Blob {
            x: 0.78,
            y: 0.82,
            r: 0.60,
            color: rgb(0xB24C7E),
            strength: 0.65,
        },
        Blob {
            x: 1.00,
            y: 1.00,
            r: 0.55,
            color: rgb(0xC0913C),
            strength: 0.60,
        },
        Blob {
            x: 0.05,
            y: 0.95,
            r: 0.55,
            color: rgb(0x14335A),
            strength: 0.55,
        },
    ],
);

/// Negro con un brillo violeta a la derecha y un toque azul abajo.
const GLOW: ([f32; 3], &[Blob]) = (
    rgb(0x09090F),
    &[
        Blob {
            x: 0.66,
            y: 0.42,
            r: 0.75,
            color: rgb(0x4B32B8),
            strength: 0.55,
        },
        Blob {
            x: 0.22,
            y: 0.95,
            r: 0.60,
            color: rgb(0x1F2A78),
            strength: 0.30,
        },
    ],
);

static CACHE: Mutex<Option<(&'static str, Handle)>> = Mutex::new(None);

/// Fondo del tema activo, si lleva uno.
pub fn current() -> Option<Handle> {
    let c = theme::colors();
    let spec = match c.look {
        Look::Glass => &GLASS,
        Look::Glow => &GLOW,
        Look::Flat | Look::Brutal => return None,
    };
    let mut cache = CACHE.lock().ok()?;
    if let Some((id, h)) = cache.as_ref() {
        if *id == c.id {
            return Some(h.clone());
        }
    }
    let h = Handle::from_rgba(W, H, render(spec.0, spec.1));
    *cache = Some((c.id, h.clone()));
    Some(h)
}

/// Pinta las manchas sobre el color base, con un poco de ruido para que el
/// degradado no haga escalones (y dé una textura de grano muy leve).
fn render(base: [f32; 3], blobs: &[Blob]) -> Vec<u8> {
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    let mut seed: u32 = 0x9E37_79B9;
    let aspect = W as f32 / H as f32;
    for py in 0..H {
        for px in 0..W {
            let u = px as f32 / (W - 1) as f32;
            let v = py as f32 / (H - 1) as f32;
            let mut c = base;
            for b in blobs {
                let dx = (u - b.x) * aspect;
                let dy = v - b.y;
                let d2 = (dx * dx + dy * dy) / (b.r * b.r);
                // Caída gaussiana: centro intenso, borde que se funde.
                let k = b.strength * (-2.5 * d2).exp();
                for (ch, bc) in c.iter_mut().zip(b.color) {
                    *ch += (bc - *ch) * k;
                }
            }
            // xorshift: ruido de ±1,5 niveles.
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed % 1000) as f32 / 1000.0 * 3.0 - 1.5;
            for ch in c {
                out.push((ch * 255.0 + noise).clamp(0.0, 255.0) as u8);
            }
            out.push(255);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_whole_image() {
        let px = render(GLOW.0, GLOW.1);
        assert_eq!(px.len(), (W * H * 4) as usize);
        // El brillo está a la derecha: más claro que la esquina de arriba a la izquierda.
        let at = |x: u32, y: u32| {
            let i = ((y * W + x) * 4) as usize;
            u32::from(px[i]) + u32::from(px[i + 1]) + u32::from(px[i + 2])
        };
        assert!(at(W * 2 / 3, H / 2) > at(2, 2) + 30);
    }
}
