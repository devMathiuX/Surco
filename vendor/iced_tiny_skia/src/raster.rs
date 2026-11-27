use crate::core::image as raster;
use crate::core::{Rectangle, Size};
use crate::graphics;

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::collections::hash_map;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
        }
    }

    pub fn load(
        &self,
        handle: &raster::Handle,
    ) -> Result<raster::Allocation, raster::Error> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle)?;

        #[allow(unsafe_code)]
        Ok(unsafe {
            raster::allocate(handle, Size::new(image.width(), image.height()))
        })
    }

    pub fn dimensions(&self, handle: &raster::Handle) -> Option<Size<u32>> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle).ok()?;

        Some(Size::new(image.width(), image.height()))
    }

    pub fn draw(
        &mut self,
        handle: &raster::Handle,
        filter_method: raster::FilterMethod,
        bounds: Rectangle,
        opacity: f32,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        transform: tiny_skia::Transform,
        clip_mask: Option<&tiny_skia::Mask>,
        clip_bounds: Rectangle,
    ) {
        let mut cache = self.cache.borrow_mut();

        let Ok(image) = cache.allocate(handle) else {
            return;
        };

        let width_scale = bounds.width / image.width() as f32;
        let height_scale = bounds.height / image.height() as f32;

        let transform = transform.pre_scale(width_scale, height_scale);

        // Surco: imagen sin rotar, opaca a pleno, sin escalar o escalada sin
        // suavizar (los cuadritos del fondo de Vidrio y Brillo): se copia
        // directo, fila por fila, recortada a mano. Por `draw_pixmap` cada
        // píxel pasaba por un shader de patrón y la máscara (~45 % de un
        // cuadro en Vidrio).
        let unscaled = transform.sx == 1.0 && transform.sy == 1.0;
        if opacity >= 1.0
            && transform.kx == 0.0
            && transform.ky == 0.0
            && transform.sx > 0.0
            && transform.sy > 0.0
            && (unscaled || filter_method == raster::FilterMethod::Nearest)
        {
            let origin = (
                (bounds.x / width_scale) as i32,
                (bounds.y / height_scale) as i32,
            );
            blit(pixels, image, origin, transform, clip_bounds);
            return;
        }

        let quality = match filter_method {
            raster::FilterMethod::Linear => tiny_skia::FilterQuality::Bilinear,
            raster::FilterMethod::Nearest => tiny_skia::FilterQuality::Nearest,
        };

        pixels.draw_pixmap(
            (bounds.x / width_scale) as i32,
            (bounds.y / height_scale) as i32,
            image,
            &tiny_skia::PixmapPaint {
                quality,
                opacity,
                ..Default::default()
            },
            transform,
            clip_mask,
        );
    }

    pub fn trim_cache(&mut self) {
        self.cache.borrow_mut().trim();
    }
}

/// Copia `image` en `pixels`, puesta en `origin` (unidades de la imagen) y
/// llevada a la pantalla por `transform` (solo escala y corrimiento), sin
/// suavizar. Solo dentro de `clip` (entra el píxel cuyo centro cae adentro,
/// como la máscara). Mezcla "source over" con colores premultiplicados
/// (copia directa donde la imagen es opaca).
fn blit(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    image: tiny_skia::PixmapRef<'_>,
    origin: (i32, i32),
    transform: tiny_skia::Transform,
    clip: Rectangle,
) {
    let (iw, ih) = (image.width() as usize, image.height() as usize);
    let (dw, dh) = (pixels.width() as i64, pixels.height() as i64);
    // Borde de la imagen en pantalla.
    let left = transform.sx * origin.0 as f32 + transform.tx;
    let top = transform.sy * origin.1 as f32 + transform.ty;
    let right = left + transform.sx * iw as f32;
    let bottom = top + transform.sy * ih as f32;
    let snap = |v: f32| (v - 0.5).ceil() as i64;
    let x0 = snap(left.max(clip.x)).max(0);
    let y0 = snap(top.max(clip.y)).max(0);
    let x1 = snap(right.min(clip.x + clip.width)).min(dw);
    let y1 = snap(bottom.min(clip.y + clip.height)).min(dh);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    // Columna y fila de la imagen para el centro de cada píxel.
    let src_of = |d: i64, start: f32, scale: f32, max: usize| {
        (((d as f32 + 0.5 - start) / scale).floor().max(0.0) as usize)
            .min(max - 1)
    };
    let cols: Vec<usize> = (x0..x1)
        .map(|x| src_of(x, left, transform.sx, iw))
        .collect();
    let contiguous = transform.sx == 1.0;
    let src = image.data();
    let dst_stride = dw as usize * 4;
    let dst = pixels.data_mut();
    for row in y0..y1 {
        let sy = src_of(row, top, transform.sy, ih);
        let src_row = &src[sy * iw * 4..(sy + 1) * iw * 4];
        let d = row as usize * dst_stride + x0 as usize * 4;
        let out = &mut dst[d..d + (x1 - x0) as usize * 4];
        if contiguous {
            let s = &src_row[cols[0] * 4..cols[0] * 4 + out.len()];
            if s.chunks_exact(4).all(|p| p[3] == 255) {
                out.copy_from_slice(s);
                continue;
            }
        }
        for (dp, &c) in out.chunks_exact_mut(4).zip(&cols) {
            let sp = &src_row[c * 4..c * 4 + 4];
            if sp[3] == 255 {
                dp.copy_from_slice(sp);
            } else {
                let inv = 255 - u16::from(sp[3]);
                for i in 0..4 {
                    dp[i] = sp[i].saturating_add(
                        ((u16::from(dp[i]) * inv + 127) / 255) as u8,
                    );
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct Cache {
    entries: FxHashMap<raster::Id, Option<Entry>>,
    hits: FxHashSet<raster::Id>,
}

impl Cache {
    pub fn allocate(
        &mut self,
        handle: &raster::Handle,
    ) -> Result<tiny_skia::PixmapRef<'_>, raster::Error> {
        let id = handle.id();

        if let hash_map::Entry::Vacant(entry) = self.entries.entry(id) {
            let image = match graphics::image::load(handle) {
                Ok(image) => image,
                Err(error) => {
                    let _ = entry.insert(None);

                    return Err(error);
                }
            };

            if image.width() == 0 || image.height() == 0 {
                return Err(raster::Error::Empty);
            }

            let mut buffer =
                vec![0u32; image.width() as usize * image.height() as usize];

            for (i, pixel) in image.pixels().enumerate() {
                let [r, g, b, a] = pixel.0;

                buffer[i] = bytemuck::cast(
                    tiny_skia::ColorU8::from_rgba(b, g, r, a).premultiply(),
                );
            }

            let _ = entry.insert(Some(Entry {
                width: image.width(),
                height: image.height(),
                pixels: buffer,
            }));
        }

        let _ = self.hits.insert(id);

        Ok(self
            .entries
            .get(&id)
            .unwrap()
            .as_ref()
            .map(|entry| {
                tiny_skia::PixmapRef::from_bytes(
                    bytemuck::cast_slice(&entry.pixels),
                    entry.width,
                    entry.height,
                )
                .expect("Build pixmap from image bytes")
            })
            .expect("Image should be allocated"))
    }

    fn trim(&mut self) {
        self.entries.retain(|key, _| self.hits.contains(key));
        self.hits.clear();
    }
}

#[derive(Debug)]
struct Entry {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}
