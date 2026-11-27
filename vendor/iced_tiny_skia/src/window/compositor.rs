use crate::core::{Color, Rectangle, Size};
use crate::graphics::compositor::{self, Information};
use crate::graphics::damage;
use crate::graphics::error::{self, Error};
use crate::graphics::{self, Shell, Viewport};
use crate::{Layer, Renderer, Settings};

use std::collections::VecDeque;
use std::num::NonZeroU32;

pub struct Compositor {
    context: softbuffer::Context<Box<dyn compositor::Display>>,
    settings: Settings,
}

pub struct Surface {
    window: softbuffer::Surface<
        Box<dyn compositor::Display>,
        Box<dyn compositor::Window>,
    >,
    clip_mask: tiny_skia::Mask,
    layer_stack: VecDeque<Vec<Layer>>,
    background_color: Color,
    max_age: u8,
    /// Surco: copia propia del último cuadro, para cuando el sistema no
    /// guarda el anterior (ver `present`).
    retained: Option<Retained>,
}

/// Surco: último cuadro dibujado, con lo que tenía, para redibujar solo lo
/// que cambió aunque el búfer del sistema venga vacío.
struct Retained {
    pixels: Vec<u32>,
    layers: Vec<Layer>,
    background_color: Color,
}

impl crate::graphics::Compositor for Compositor {
    type Renderer = Renderer;
    type Surface = Surface;

    async fn with_backend(
        settings: graphics::Settings,
        display: impl compositor::Display,
        _compatible_window: impl compositor::Window,
        _shell: Shell,
        backend: Option<&str>,
    ) -> Result<Self, Error> {
        match backend {
            None | Some("tiny-skia") | Some("tiny_skia") => {
                Ok(new(settings.into(), display))
            }
            Some(backend) => Err(Error::GraphicsAdapterNotFound {
                backend: "tiny-skia",
                reason: error::Reason::DidNotMatch {
                    preferred_backend: backend.to_owned(),
                },
            }),
        }
    }

    fn create_renderer(&self) -> Self::Renderer {
        Renderer::new(
            self.settings.default_font,
            self.settings.default_text_size,
        )
    }

    fn create_surface<W: compositor::Window + Clone>(
        &mut self,
        window: W,
        width: u32,
        height: u32,
    ) -> Self::Surface {
        let window = softbuffer::Surface::new(
            &self.context,
            Box::new(window.clone()) as _,
        )
        .expect("Create softbuffer surface for window");

        let mut surface = Surface {
            window,
            clip_mask: tiny_skia::Mask::new(1, 1).expect("Create clip mask"),
            layer_stack: VecDeque::new(),
            background_color: Color::BLACK,
            max_age: 0,
            retained: None,
        };

        if width > 0 && height > 0 {
            self.configure_surface(&mut surface, width, height);
        }

        surface
    }

    fn configure_surface(
        &mut self,
        surface: &mut Self::Surface,
        width: u32,
        height: u32,
    ) {
        surface
            .window
            .resize(
                NonZeroU32::new(width).expect("Non-zero width"),
                NonZeroU32::new(height).expect("Non-zero height"),
            )
            .expect("Resize surface");

        surface.clip_mask =
            tiny_skia::Mask::new(width, height).expect("Create clip mask");
        surface.layer_stack.clear();
        surface.retained = None;
    }

    fn information(&self) -> Information {
        Information {
            adapter: String::from("CPU"),
            backend: String::from("tiny-skia"),
        }
    }

    fn present(
        &mut self,
        renderer: &mut Self::Renderer,
        surface: &mut Self::Surface,
        viewport: &Viewport,
        background_color: Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), compositor::SurfaceError> {
        present(
            renderer,
            surface,
            viewport,
            background_color,
            on_pre_present,
        )
    }

    fn screenshot(
        &mut self,
        renderer: &mut Self::Renderer,
        viewport: &Viewport,
        background_color: Color,
    ) -> Vec<u8> {
        screenshot(renderer, viewport, background_color)
    }
}

pub fn new(
    settings: Settings,
    display: impl compositor::Display,
) -> Compositor {
    #[allow(unsafe_code)]
    let context = softbuffer::Context::new(Box::new(display) as _)
        .expect("Create softbuffer context");

    Compositor { context, settings }
}

/// Más rectángulos dañados que esto se juntan en uno (ver `present`).
const MAX_DAMAGE_RECTS: usize = 8;

pub fn present(
    renderer: &mut Renderer,
    surface: &mut Surface,
    viewport: &Viewport,
    background_color: Color,
    on_pre_present: impl FnOnce(),
) -> Result<(), compositor::SurfaceError> {
    let physical_size = viewport.physical_size();

    let mut buffer = surface
        .window
        .buffer_mut()
        .map_err(|_| compositor::SurfaceError::Lost)?;

    // Surco: en macOS (softbuffer con CoreGraphics) `age()` siempre es 0 y
    // cada cuadro llega en un búfer nuevo y vacío, así que había que
    // dibujar la ventana entera cada vez, aunque cambiara una sola línea
    // (con pantalla Retina, cuatro veces los píxeles). Ahí guardamos una
    // copia del cuadro anterior, dibujamos solo lo dañado sobre ella y la
    // copiamos al búfer. En los demás sistemas el primer cuadro también
    // viene con `age()` 0, pero después no: ahí no hace falta la copia.
    // `SURCO_AGE0=1` simula el caso de macOS en otros sistemas.
    static SIMULATE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let simulate =
        *SIMULATE.get_or_init(|| std::env::var_os("SURCO_AGE0").is_some());
    let age = if simulate { 0 } else { buffer.age() };
    if age == 0 && (cfg!(target_os = "macos") || simulate) {
        present_retained(
            renderer,
            &mut surface.retained,
            &mut surface.clip_mask,
            &mut buffer,
            viewport,
            background_color,
        );
        on_pre_present();
        return buffer
            .present()
            .map_err(|_| compositor::SurfaceError::Lost);
    }
    surface.retained = None;

    let last_layers = {

        surface.max_age = surface.max_age.max(age);
        surface.layer_stack.truncate(surface.max_age as usize);

        if age > 0 {
            surface.layer_stack.get(age as usize - 1)
        } else {
            None
        }
    };

    let damage = last_layers
        .and_then(|last_layers| {
            (surface.background_color == background_color).then(|| {
                damage::diff(
                    last_layers,
                    renderer.layers(),
                    |layer| vec![layer.bounds],
                    Layer::damage,
                )
            })
        })
        .unwrap_or_else(|| vec![Rectangle::with_size(viewport.logical_size())]);

    if damage.is_empty() {
        if let Some(last_layers) = last_layers {
            surface.layer_stack.push_front(last_layers.clone());
        }
    } else {
        surface.layer_stack.push_front(renderer.layers().to_vec());
        surface.background_color = background_color;

        // Surco: si son muchos rectángulos, uno solo (ver `group_damage`).
        let damage = group_damage(damage, viewport);

        let mut pixels = tiny_skia::PixmapMut::from_bytes(
            bytemuck::cast_slice_mut(&mut buffer),
            physical_size.width,
            physical_size.height,
        )
        .expect("Create pixel map");

        renderer.draw(
            &mut pixels,
            &mut surface.clip_mask,
            viewport,
            &damage,
            background_color,
        );
    }

    on_pre_present();
    buffer.present().map_err(|_| compositor::SurfaceError::Lost)
}

/// Surco: rectángulos para dibujar, juntados en uno si son muchos. Al
/// desplazar una lista cambia casi todo lo que hay adentro, y salían 70-80
/// rectángulos dañados que cubrían casi toda la ventana. Cada uno se dibuja
/// aparte (todo lo que lo toca, recortado con la máscara), así que la misma
/// letra o el mismo fondo se pintaba varias veces. Si son muchos, mejor uno
/// solo que los cubra.
fn group_damage(damage: Vec<Rectangle>, viewport: &Viewport) -> Vec<Rectangle> {
    let damage =
        damage::group(damage, Rectangle::with_size(viewport.logical_size()));
    if damage.len() > MAX_DAMAGE_RECTS {
        damage
            .iter()
            .copied()
            .reduce(|a, b| a.union(&b))
            .into_iter()
            .collect()
    } else {
        damage
    }
}

/// Surco: dibuja sobre la copia propia del cuadro anterior solo lo que
/// cambió y la pasa al búfer del sistema (ver `present`).
fn present_retained(
    renderer: &mut Renderer,
    retained: &mut Option<Retained>,
    clip_mask: &mut tiny_skia::Mask,
    buffer: &mut [u32],
    viewport: &Viewport,
    background_color: Color,
) {
    let physical_size = viewport.physical_size();
    let len = physical_size.width as usize * physical_size.height as usize;
    if buffer.len() != len {
        return;
    }

    let damage = match retained.as_ref() {
        Some(last)
            if last.pixels.len() == len
                && last.background_color == background_color =>
        {
            damage::diff(
                &last.layers,
                renderer.layers(),
                |layer| vec![layer.bounds],
                Layer::damage,
            )
        }
        _ => {
            *retained = None;
            vec![Rectangle::with_size(viewport.logical_size())]
        }
    };

    let retained = retained.get_or_insert_with(|| Retained {
        pixels: vec![0; len],
        layers: Vec::new(),
        background_color,
    });

    if !damage.is_empty() {
        let damage = group_damage(damage, viewport);
        let mut pixels = tiny_skia::PixmapMut::from_bytes(
            bytemuck::cast_slice_mut(&mut retained.pixels),
            physical_size.width,
            physical_size.height,
        )
        .expect("Create pixel map");

        renderer.draw(
            &mut pixels,
            clip_mask,
            viewport,
            &damage,
            background_color,
        );
        retained.layers = renderer.layers().to_vec();
        retained.background_color = background_color;
    }

    buffer.copy_from_slice(&retained.pixels);
}

pub fn screenshot(
    renderer: &mut Renderer,
    viewport: &Viewport,
    background_color: Color,
) -> Vec<u8> {
    let size = viewport.physical_size();

    let mut offscreen_buffer: Vec<u32> =
        vec![0; size.width as usize * size.height as usize];

    let mut clip_mask = tiny_skia::Mask::new(size.width, size.height)
        .expect("Create clip mask");

    renderer.draw(
        &mut tiny_skia::PixmapMut::from_bytes(
            bytemuck::cast_slice_mut(&mut offscreen_buffer),
            size.width,
            size.height,
        )
        .expect("Create offscreen pixel map"),
        &mut clip_mask,
        viewport,
        &[Rectangle::with_size(Size::new(
            size.width as f32,
            size.height as f32,
        ))],
        background_color,
    );

    offscreen_buffer.iter().fold(
        Vec::with_capacity(offscreen_buffer.len() * 4),
        |mut acc, pixel| {
            const A_MASK: u32 = 0xFF_00_00_00;
            const R_MASK: u32 = 0x00_FF_00_00;
            const G_MASK: u32 = 0x00_00_FF_00;
            const B_MASK: u32 = 0x00_00_00_FF;

            let a = ((A_MASK & pixel) >> 24) as u8;
            let r = ((R_MASK & pixel) >> 16) as u8;
            let g = ((G_MASK & pixel) >> 8) as u8;
            let b = (B_MASK & pixel) as u8;

            acc.extend([r, g, b, a]);
            acc
        },
    )
}
