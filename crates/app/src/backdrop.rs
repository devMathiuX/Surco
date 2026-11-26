//! Fondos dibujados por código para los temas Vidrio y Brillo: manchas de
//! color con caída suave, calculadas una vez en baja resolución y
//! agrandadas al tamaño de la ventana. Sin imágenes en el binario.
//!
//! El fondo se entrega cortado en cuadritos ([`Tiles`]). El renderer por CPU
//! (tiny-skia) redibuja solo lo que cambió, pero una imagen del tamaño de la
//! ventana se volvía a escalar entera con cada cambio (pasar el mouse por
//! una fila costaba ~58 % de un núcleo). En cuadritos sin escalar se
//! redibujan solo los que quedan debajo del cambio (~9 %).

use std::sync::Mutex;

use iced::widget::image::Handle;

use crate::theme::{self, Look};

/// Resolución en la que se calculan las manchas (después se agranda).
const W: u32 = 480;
const H: u32 = 270;

/// Lado de cada cuadrito, en píxeles lógicos.
pub const TILE: u32 = 64;

/// El fondo se arma para tamaños múltiplos de esto, así al arrastrar el
/// borde de la ventana no se regenera a cada píxel.
const STEP: u32 = 128;

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

/// Fondo cortado en cuadritos de [`TILE`] px, fila por fila.
#[derive(Debug, Clone)]
pub struct Tiles {
    pub cols: u32,
    pub rows: u32,
    pub handles: Vec<Handle>,
}

/// Manchas en baja resolución del tema y fondo armado para el último tamaño.
struct Cache {
    theme: &'static str,
    base: Vec<[f32; 3]>,
    size: (u32, u32),
    tiles: Option<Tiles>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// ¿El tema activo lleva fondo?
pub fn active() -> bool {
    matches!(theme::colors().look, Look::Glass | Look::Glow)
}

/// Fondo del tema activo para una ventana de `width`×`height` (lógicos), si
/// el tema lleva uno. Se recalcula solo al cambiar de tema o al pasar a otro
/// escalón de [`STEP`] px.
pub fn tiles(width: f32, height: f32) -> Option<Tiles> {
    let c = theme::colors();
    let spec = match c.look {
        Look::Glass => &GLASS,
        Look::Glow => &GLOW,
        Look::Flat | Look::Brutal | Look::Minimal => return None,
    };
    let round = |v: f32| (v.max(1.0).ceil() as u32).div_ceil(STEP) * STEP;
    let size = (round(width), round(height));
    let mut cache = CACHE.lock().ok()?;
    if cache.as_ref().is_none_or(|k| k.theme != c.id) {
        *cache = Some(Cache {
            theme: c.id,
            base: render(spec.0, spec.1),
            size: (0, 0),
            tiles: None,
        });
    }
    let cache = cache.as_mut()?;
    if cache.size != size || cache.tiles.is_none() {
        // Soltar los cuadritos viejos antes de armar los nuevos.
        cache.tiles = None;
        cache.tiles = Some(cut(&cache.base, size.0, size.1));
        cache.size = size;
    }
    cache.tiles.clone()
}

/// Calcula las manchas sobre el color base, en baja resolución.
fn render(base: [f32; 3], blobs: &[Blob]) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity((W * H) as usize);
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
            out.push(c);
        }
    }
    out
}

/// Agranda `base` a `width`×`height` (interpolando) y lo corta en
/// cuadritos. Lleva un poco de ruido para que el degradado no haga
/// escalones (y dé un grano muy leve).
fn cut(base: &[[f32; 3]], width: u32, height: u32) -> Tiles {
    let cols = width.div_ceil(TILE);
    let rows = height.div_ceil(TILE);
    let mut seed: u32 = 0x9E37_79B9;
    let sx = (W - 1) as f32 / (width.max(2) - 1) as f32;
    let sy = (H - 1) as f32 / (height.max(2) - 1) as f32;
    let at = |x: u32, y: u32| base[(y.min(H - 1) * W + x.min(W - 1)) as usize];
    let mut handles = Vec::with_capacity((cols * rows) as usize);
    for ty in 0..rows {
        for tx in 0..cols {
            let mut px = Vec::with_capacity((TILE * TILE * 4) as usize);
            for y in ty * TILE..(ty + 1) * TILE {
                let fy = y as f32 * sy;
                let (y0, wy) = (fy as u32, fy.fract());
                for x in tx * TILE..(tx + 1) * TILE {
                    let fx = x as f32 * sx;
                    let (x0, wx) = (fx as u32, fx.fract());
                    let (a, b, c, d) = (
                        at(x0, y0),
                        at(x0 + 1, y0),
                        at(x0, y0 + 1),
                        at(x0 + 1, y0 + 1),
                    );
                    // xorshift: ruido de ±1,5 niveles.
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let noise = (seed % 1000) as f32 / 1000.0 * 3.0 - 1.5;
                    for ch in 0..3 {
                        let top = a[ch] + (b[ch] - a[ch]) * wx;
                        let bottom = c[ch] + (d[ch] - c[ch]) * wx;
                        let v = top + (bottom - top) * wy;
                        px.push((v * 255.0 + noise).clamp(0.0, 255.0) as u8);
                    }
                    px.push(255);
                }
            }
            handles.push(Handle::from_rgba(TILE, TILE, px));
        }
    }
    Tiles {
        cols,
        rows,
        handles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_whole_image() {
        let px = render(GLOW.0, GLOW.1);
        assert_eq!(px.len(), (W * H) as usize);
        // El brillo está a la derecha: más claro que la esquina de arriba a la izquierda.
        let at = |x: u32, y: u32| px[(y * W + x) as usize].iter().sum::<f32>();
        assert!(at(W * 2 / 3, H / 2) > at(2, 2) + 0.1);
    }

    #[test]
    fn cuts_into_tiles_that_cover_the_window() {
        let base = render(GLASS.0, GLASS.1);
        let t = cut(&base, 256, 200);
        assert_eq!((t.cols, t.rows), (4, 4));
        assert_eq!(t.handles.len(), 16);
    }
}
