use crate::Primitive;
use crate::core::renderer::Quad;
use crate::core::{
    Background, Color, Gradient, Rectangle, Size, Transformation, Vector,
};
use crate::graphics::{Image, Text};
use crate::text;

#[derive(Debug)]
pub struct Engine {
    text_pipeline: text::Pipeline,

    #[cfg(feature = "image")]
    pub(crate) raster_pipeline: crate::raster::Pipeline,
    #[cfg(feature = "svg")]
    pub(crate) vector_pipeline: crate::vector::Pipeline,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            text_pipeline: text::Pipeline::new(),
            #[cfg(feature = "image")]
            raster_pipeline: crate::raster::Pipeline::new(),
            #[cfg(feature = "svg")]
            vector_pipeline: crate::vector::Pipeline::new(),
        }
    }

    pub fn draw_quad(
        &mut self,
        quad: &Quad,
        background: &Background,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        let physical_bounds = quad.bounds * transformation;

        if !clip_bounds.intersects(&physical_bounds) {
            return;
        }

        let clip_mask = (!physical_bounds.is_within(&clip_bounds))
            .then_some(clip_mask as &_);

        let transform = into_transform(transformation);

        // Make sure the border radius is not larger than the bounds
        let border_width = quad
            .border
            .width
            .min(quad.bounds.width / 2.0)
            .min(quad.bounds.height / 2.0);

        let mut fill_border_radius = <[f32; 4]>::from(quad.border.radius);

        for radius in &mut fill_border_radius {
            *radius = (*radius)
                .min(quad.bounds.width / 2.0)
                .min(quad.bounds.height / 2.0);
        }

        let path = rounded_rectangle(quad.bounds, fill_border_radius);

        let shadow = quad.shadow;

        if shadow.color.a > 0.0 {
            let shadow_bounds = Rectangle {
                x: quad.bounds.x + shadow.offset.x - shadow.blur_radius,
                y: quad.bounds.y + shadow.offset.y - shadow.blur_radius,
                width: quad.bounds.width + shadow.blur_radius * 2.0,
                height: quad.bounds.height + shadow.blur_radius * 2.0,
            } * transformation;

            let radii = fill_border_radius
                .into_iter()
                .map(|radius| radius * transformation.scale_factor())
                .collect::<Vec<_>>();
            let (x, y, width, height) = (
                shadow_bounds.x as u32,
                shadow_bounds.y as u32,
                shadow_bounds.width as u32,
                shadow_bounds.height as u32,
            );
            let half_width = physical_bounds.width / 2.0;
            let half_height = physical_bounds.height / 2.0;

            let colors = (y..y + height)
                .flat_map(|y| (x..x + width).map(move |x| (x as f32, y as f32)))
                .filter_map(|(x, y)| {
                    tiny_skia::Size::from_wh(half_width, half_height).map(
                        |size| {
                            let shadow_distance = rounded_box_sdf(
                                Vector::new(
                                    x - physical_bounds.position().x
                                        - (shadow.offset.x
                                            * transformation.scale_factor())
                                        - half_width,
                                    y - physical_bounds.position().y
                                        - (shadow.offset.y
                                            * transformation.scale_factor())
                                        - half_height,
                                ),
                                size,
                                &radii,
                            )
                            .max(0.0);
                            let shadow_alpha = 1.0
                                - smoothstep(
                                    -shadow.blur_radius
                                        * transformation.scale_factor(),
                                    shadow.blur_radius
                                        * transformation.scale_factor(),
                                    shadow_distance,
                                );

                            let mut color = into_color(shadow.color);
                            color.apply_opacity(shadow_alpha);

                            color.to_color_u8().premultiply()
                        },
                    )
                })
                .collect();

            if let Some(pixmap) = tiny_skia::IntSize::from_wh(width, height)
                .and_then(|size| {
                    tiny_skia::Pixmap::from_vec(
                        bytemuck::cast_vec(colors),
                        size,
                    )
                })
            {
                pixels.draw_pixmap(
                    x as i32,
                    y as i32,
                    pixmap.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    tiny_skia::Transform::default(),
                    None,
                );
            }
        }

        if shadow.color.a <= 0.0 {
            // Surco: ver `fill_clipped`.
            fill_clipped(
                pixels,
                quad.bounds,
                fill_border_radius,
                &quad_shader(quad, background),
                transformation.scale_factor(),
                clip_bounds,
                clip_mask,
            );
        } else {
            pixels.fill_path(
                &path,
                &tiny_skia::Paint {
                    shader: quad_shader(quad, background),
                    anti_alias: true,
                    ..tiny_skia::Paint::default()
                },
                tiny_skia::FillRule::EvenOdd,
                transform,
                None,
            );
        }

        if border_width > 0.0 {
            // Border path is offset by half the border width
            let border_bounds = Rectangle {
                x: quad.bounds.x + border_width / 2.0,
                y: quad.bounds.y + border_width / 2.0,
                width: quad.bounds.width - border_width,
                height: quad.bounds.height - border_width,
            };

            // Make sure the border radius is correct
            let mut border_radius = <[f32; 4]>::from(quad.border.radius);
            let mut is_simple_border = true;

            for radius in &mut border_radius {
                *radius = if *radius == 0.0 {
                    // Path should handle this fine
                    0.0
                } else if *radius > border_width / 2.0 {
                    *radius - border_width / 2.0
                } else {
                    is_simple_border = false;
                    0.0
                }
                .min(border_bounds.width / 2.0)
                .min(border_bounds.height / 2.0);
            }

            // Stroking a path works well in this case
            if is_simple_border {
                let border_path =
                    rounded_rectangle(border_bounds, border_radius);

                pixels.stroke_path(
                    &border_path,
                    &tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(into_color(
                            quad.border.color,
                        )),
                        anti_alias: true,
                        ..tiny_skia::Paint::default()
                    },
                    &tiny_skia::Stroke {
                        width: border_width,
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    clip_mask,
                );
            } else {
                // Draw corners that have too small border radii as having no border radius,
                // but mask them with the rounded rectangle with the correct border radius.
                let mut temp_pixmap = tiny_skia::Pixmap::new(
                    quad.bounds.width as u32,
                    quad.bounds.height as u32,
                )
                .unwrap();

                let mut quad_mask = tiny_skia::Mask::new(
                    quad.bounds.width as u32,
                    quad.bounds.height as u32,
                )
                .unwrap();

                let zero_bounds = Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: quad.bounds.width,
                    height: quad.bounds.height,
                };
                let path = rounded_rectangle(zero_bounds, fill_border_radius);

                quad_mask.fill_path(
                    &path,
                    tiny_skia::FillRule::EvenOdd,
                    true,
                    transform,
                );
                let path_bounds = Rectangle {
                    x: border_width / 2.0,
                    y: border_width / 2.0,
                    width: quad.bounds.width - border_width,
                    height: quad.bounds.height - border_width,
                };

                let border_radius_path =
                    rounded_rectangle(path_bounds, border_radius);

                temp_pixmap.stroke_path(
                    &border_radius_path,
                    &tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(into_color(
                            quad.border.color,
                        )),
                        anti_alias: true,
                        ..tiny_skia::Paint::default()
                    },
                    &tiny_skia::Stroke {
                        width: border_width,
                        ..tiny_skia::Stroke::default()
                    },
                    transform,
                    Some(&quad_mask),
                );

                pixels.draw_pixmap(
                    quad.bounds.x as i32,
                    quad.bounds.y as i32,
                    temp_pixmap.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    transform,
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_text(
        &mut self,
        text: &Text,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match text {
            Text::Paragraph {
                paragraph,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) = clip_bounds
                    .intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, paragraph.min_bounds)
                        * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_paragraph(
                    paragraph,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Editor {
                editor,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) = clip_bounds
                    .intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, editor.bounds) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_editor(
                    editor,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Cached {
                content,
                bounds,
                color,
                size,
                line_height,
                font,
                align_x,
                align_y,
                shaping,
                clip_bounds: local_clip_bounds,
            } => {
                let physical_bounds = *local_clip_bounds * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_cached(
                    content,
                    *bounds,
                    *color,
                    *size,
                    *line_height,
                    *font,
                    *align_x,
                    *align_y,
                    *shaping,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Raw {
                raw,
                transformation: local_transformation,
            } => {
                let Some(buffer) = raw.buffer.upgrade() else {
                    return;
                };

                let transformation = transformation * *local_transformation;
                let (width, height) = buffer.size();

                let physical_bounds = Rectangle::new(
                    raw.position,
                    Size::new(
                        width.unwrap_or(clip_bounds.width),
                        height.unwrap_or(clip_bounds.height),
                    ),
                ) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                self.text_pipeline.draw_raw(
                    &buffer,
                    raw.position,
                    raw.color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
        }
    }

    pub fn draw_primitive(
        &mut self,
        primitive: &Primitive,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match primitive {
            Primitive::Fill { path, paint, rule } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x(),
                        y: bounds.y(),
                        width: bounds.width(),
                        height: bounds.height(),
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                pixels.fill_path(
                    path,
                    paint,
                    *rule,
                    into_transform(transformation),
                    clip_mask,
                );
            }
            Primitive::Stroke {
                path,
                paint,
                stroke,
            } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x() - stroke.width / 2.0,
                        y: bounds.y() - stroke.width / 2.0,
                        width: bounds.width() + stroke.width,
                        height: bounds.height() + stroke.width,
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&clip_bounds))
                    .then_some(clip_mask as &_);

                pixels.stroke_path(
                    path,
                    paint,
                    stroke,
                    into_transform(transformation),
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_image(
        &mut self,
        image: &Image,
        _transformation: Transformation,
        _pixels: &mut tiny_skia::PixmapMut<'_>,
        _clip_mask: &mut tiny_skia::Mask,
        _clip_bounds: Rectangle,
    ) {
        match image {
            #[cfg(feature = "image")]
            Image::Raster { image, bounds, .. } => {
                let physical_bounds = *bounds * _transformation;

                if !_clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&_clip_bounds))
                    .then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(image.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.raster_pipeline.draw(
                    &image.handle,
                    image.filter_method,
                    *bounds,
                    image.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                    _clip_bounds,
                );
            }
            #[cfg(feature = "svg")]
            Image::Vector { svg, bounds, .. } => {
                let physical_bounds = *bounds * _transformation;

                if !_clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = (!physical_bounds.is_within(&_clip_bounds))
                    .then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(svg.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.vector_pipeline.draw(
                    &svg.handle,
                    svg.color,
                    *bounds,
                    svg.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                );
            }
            #[cfg(not(feature = "image"))]
            Image::Raster { .. } => {
                log::warn!(
                    "Unsupported primitive in `iced_tiny_skia`: {image:?}",
                );
            }
            #[cfg(not(feature = "svg"))]
            Image::Vector { .. } => {
                log::warn!(
                    "Unsupported primitive in `iced_tiny_skia`: {image:?}",
                );
            }
        }
    }

    pub fn trim(&mut self) {
        self.text_pipeline.trim_cache();

        #[cfg(feature = "image")]
        self.raster_pipeline.trim_cache();

        #[cfg(feature = "svg")]
        self.vector_pipeline.trim_cache();
    }
}

pub fn into_color(color: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(color.b, color.g, color.r, color.a)
        .expect("Convert color from iced to tiny_skia")
}

fn into_transform(transformation: Transformation) -> tiny_skia::Transform {
    let translation = transformation.translation();

    tiny_skia::Transform {
        sx: transformation.scale_factor(),
        kx: 0.0,
        ky: 0.0,
        sy: transformation.scale_factor(),
        tx: translation.x,
        ty: translation.y,
    }
}

fn rounded_rectangle(
    bounds: Rectangle,
    border_radius: [f32; 4],
) -> tiny_skia::Path {
    let [top_left, top_right, bottom_right, bottom_left] = border_radius;

    if top_left == 0.0
        && top_right == 0.0
        && bottom_right == 0.0
        && bottom_left == 0.0
    {
        return tiny_skia::PathBuilder::from_rect(
            tiny_skia::Rect::from_xywh(
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
            )
            .expect("Build quad rectangle"),
        );
    }

    if top_left == top_right
        && top_left == bottom_right
        && top_left == bottom_left
        && top_left == bounds.width / 2.0
        && top_left == bounds.height / 2.0
    {
        return tiny_skia::PathBuilder::from_circle(
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
            top_left,
        )
        .expect("Build circle path");
    }

    let mut builder = tiny_skia::PathBuilder::new();

    builder.move_to(bounds.x + top_left, bounds.y);
    builder.line_to(bounds.x + bounds.width - top_right, bounds.y);

    if top_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width - top_right,
            bounds.y,
            bounds.x + bounds.width,
            bounds.y + top_right,
            top_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bounds.width,
        bounds.y + bounds.height - bottom_right,
    );

    if bottom_right > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bounds.width,
            bounds.y + bounds.height - bottom_right,
            bounds.x + bounds.width - bottom_right,
            bounds.y + bounds.height,
            bottom_right,
        );
    }

    maybe_line_to(
        &mut builder,
        bounds.x + bottom_left,
        bounds.y + bounds.height,
    );

    if bottom_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x + bottom_left,
            bounds.y + bounds.height,
            bounds.x,
            bounds.y + bounds.height - bottom_left,
            bottom_left,
        );
    }

    maybe_line_to(&mut builder, bounds.x, bounds.y + top_left);

    if top_left > 0.0 {
        arc_to(
            &mut builder,
            bounds.x,
            bounds.y + top_left,
            bounds.x + top_left,
            bounds.y,
            top_left,
        );
    }

    builder.finish().expect("Build rounded rectangle path")
}

fn maybe_line_to(path: &mut tiny_skia::PathBuilder, x: f32, y: f32) {
    if path.last_point() != Some(tiny_skia::Point { x, y }) {
        path.line_to(x, y);
    }
}

fn arc_to(
    path: &mut tiny_skia::PathBuilder,
    x_from: f32,
    y_from: f32,
    x_to: f32,
    y_to: f32,
    radius: f32,
) {
    let svg_arc = kurbo::SvgArc {
        from: kurbo::Point::new(f64::from(x_from), f64::from(y_from)),
        to: kurbo::Point::new(f64::from(x_to), f64::from(y_to)),
        radii: kurbo::Vec2::new(f64::from(radius), f64::from(radius)),
        x_rotation: 0.0,
        large_arc: false,
        sweep: true,
    };

    match kurbo::Arc::from_svg_arc(&svg_arc) {
        Some(arc) => {
            arc.to_cubic_beziers(0.1, |p1, p2, p| {
                path.cubic_to(
                    p1.x as f32,
                    p1.y as f32,
                    p2.x as f32,
                    p2.y as f32,
                    p.x as f32,
                    p.y as f32,
                );
            });
        }
        None => {
            path.line_to(x_to, y_to);
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let x = ((x - a) / (b - a)).clamp(0.0, 1.0);

    x * x * (3.0 - 2.0 * x)
}

fn rounded_box_sdf(
    to_center: Vector,
    size: tiny_skia::Size,
    radii: &[f32],
) -> f32 {
    let radius = match (to_center.x > 0.0, to_center.y > 0.0) {
        (true, true) => radii[2],
        (true, false) => radii[1],
        (false, true) => radii[3],
        (false, false) => radii[0],
    };

    let x = (to_center.x.abs() - size.width() + radius).max(0.0);
    let y = (to_center.y.abs() - size.height() + radius).max(0.0);

    (x.powf(2.0) + y.powf(2.0)).sqrt() - radius
}

/// Surco: relleno de un quad.
///
/// El original rasterizaba el rectángulo redondeado entero con antialias (por
/// supermuestreo) y, si se salía del recorte, pasando cada píxel por la
/// máscara, aunque solo se viera un pedazo (el fondo de un panel mientras se
/// desplaza la lista de adentro): ~40 % de cada cuadro. Acá se arma con
/// partes: tres bandas rectas recortadas a mano (sin máscara, por el camino
/// rápido de rectángulos) y las cuatro esquinas, chicas, con la máscara. `bounds` y `radii` van en
/// unidades lógicas; `clip` en píxeles.
fn fill_clipped(
    pixels: &mut tiny_skia::PixmapMut<'_>,
    bounds: Rectangle,
    radii: [f32; 4],
    shader: &tiny_skia::Shader<'_>,
    scale: f32,
    clip: Rectangle,
    mask: Option<&tiny_skia::Mask>,
) {
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    let paint = tiny_skia::Paint {
        shader: shader.clone(),
        anti_alias: true,
        ..tiny_skia::Paint::default()
    };
    // El recorte, llevado a píxeles enteros como la máscara (entra el píxel
    // cuyo centro cae adentro) y vuelto a unidades lógicas.
    let snap = |v: f32| (v - 0.5).ceil();
    let (cx0, cy0) = (snap(clip.x) / scale, snap(clip.y) / scale);
    let (cx1, cy1) = (
        snap(clip.x + clip.width) / scale,
        snap(clip.y + clip.height) / scale,
    );
    let mut fill = |x0: f32, y0: f32, x1: f32, y1: f32| {
        let (x0, y0, x1, y1) =
            (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
        if x1 > x0 && y1 > y0 {
            if let Some(rect) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
                pixels.fill_rect(rect, &paint, transform, None);
            }
        }
    };
    let (x, y, w, h) = (bounds.x, bounds.y, bounds.width, bounds.height);
    let [tl, tr, br, bl] = radii;
    let r = tl.max(tr).max(br).max(bl);
    if r <= 0.0 {
        fill(x, y, x + w, y + h);
        return;
    }
    // Los cortes entre partes caen en bordes de píxel (hacia adentro de la
    // figura): si no, el antialias de las dos partes se suma o deja un hueco
    // y se ve una línea fina (con la interfaz agrandada, por ejemplo).
    let inward = |v: f32, up: bool| {
        let p = v * scale;
        (if up { p.ceil() } else { p.floor() }) / scale
    };
    let (ix0, ix1) = (inward(x + r, true), inward(x + w - r, false));
    let (iy0, iy1) = (inward(y + r, true), inward(y + h - r, false));
    if ix1 <= ix0 || iy1 <= iy0 {
        // Casi todo esquina (una píldora chica): de una, con la máscara.
        let path = rounded_rectangle(bounds, radii);
        pixels.fill_path(
            &path,
            &paint,
            tiny_skia::FillRule::EvenOdd,
            transform,
            mask,
        );
        return;
    }
    // Banda del medio, entera de arriba a abajo, y las de los costados
    // entre las esquinas.
    fill(ix0, y, ix1, y + h);
    fill(x, iy0, ix0, iy1);
    fill(ix1, iy0, x + w, iy1);
    // Esquinas: el cuadrado de lado `r`, con la curva de su radio.
    // (esquina, radio, hacia dónde queda la figura, lado del cuadrado)
    let corners = [
        (x, y, tl, 1.0, 1.0, ix0 - x, iy0 - y),
        (x + w, y, tr, -1.0, 1.0, x + w - ix1, iy0 - y),
        (x + w, y + h, br, -1.0, -1.0, x + w - ix1, y + h - iy1),
        (x, y + h, bl, 1.0, -1.0, ix0 - x, y + h - iy1),
    ];
    for (px, py, radius, sx, sy, rx, ry) in corners {
        let (ax, bx) = (px.min(px + sx * rx), px.max(px + sx * rx));
        let (ay, by) = (py.min(py + sy * ry), py.max(py + sy * ry));
        if bx <= cx0 || ax >= cx1 || by <= cy0 || ay >= cy1 {
            continue;
        }
        let mut builder = tiny_skia::PathBuilder::new();
        // De la punta interior del cuadrado, por el borde, a la curva.
        builder.move_to(px + sx * rx, py + sy * ry);
        builder.line_to(px, py + sy * ry);
        builder.line_to(px, py + sy * radius);
        if radius > 0.0 {
            // Cuarto de círculo con una cúbica (k ≈ 0,5523).
            let k = 0.552_284_8 * radius;
            builder.cubic_to(
                px,
                py + sy * (radius - k),
                px + sx * (radius - k),
                py,
                px + sx * radius,
                py,
            );
        }
        builder.line_to(px + sx * rx, py);
        builder.close();
        if let Some(path) = builder.finish() {
            pixels.fill_path(
                &path,
                &paint,
                tiny_skia::FillRule::Winding,
                transform,
                mask,
            );
        }
    }
}

/// Pintura del fondo de un quad (color o degradado).
fn quad_shader(
    quad: &Quad,
    background: &Background,
) -> tiny_skia::Shader<'static> {
    match background {
        Background::Color(color) => {
            tiny_skia::Shader::SolidColor(into_color(*color))
        }
        Background::Gradient(Gradient::Linear(linear)) => {
            let (start, end) = linear.angle.to_distance(&quad.bounds);

            let stops: Vec<tiny_skia::GradientStop> = linear
                .stops
                .into_iter()
                .flatten()
                .map(|stop| {
                    tiny_skia::GradientStop::new(
                        stop.offset,
                        tiny_skia::Color::from_rgba(
                            stop.color.b,
                            stop.color.g,
                            stop.color.r,
                            stop.color.a,
                        )
                        .expect("Create color"),
                    )
                })
                .collect();

            tiny_skia::LinearGradient::new(
                tiny_skia::Point {
                    x: start.x,
                    y: start.y,
                },
                tiny_skia::Point { x: end.x, y: end.y },
                if stops.is_empty() {
                    vec![tiny_skia::GradientStop::new(
                        0.0,
                        tiny_skia::Color::BLACK,
                    )]
                } else {
                    stops
                },
                tiny_skia::SpreadMode::Pad,
                tiny_skia::Transform::identity(),
            )
            .expect("Create linear gradient")
        }
    }
}

pub fn adjust_clip_mask(clip_mask: &mut tiny_skia::Mask, bounds: Rectangle) {
    // Surco: el original borraba la máscara entera (del tamaño de la ventana)
    // y rellenaba el rectángulo con `fill_path` cada vez que cambiaba el
    // recorte: una vez por capa y por cada texto cortado. Con listas de
    // filas recortadas, eso era ~50 % de cada cuadro. Acá se borra solo el
    // rectángulo anterior y se escribe el nuevo a mano (mismo resultado que
    // `fill_path` sin antialias: entra el píxel cuyo centro cae adentro).
    use std::cell::Cell;

    thread_local! {
        /// Máscara (puntero y tamaño) y rectángulo que quedó lleno.
        static LAST: Cell<Option<(usize, u32, u32, [u32; 4])>> =
            const { Cell::new(None) };
    }

    let (width, height) = (clip_mask.width(), clip_mask.height());
    let span = |start: f32, end: f32, max: u32| {
        let a = (start - 0.5).ceil().clamp(0.0, max as f32) as u32;
        let b = (end - 0.5).ceil().clamp(0.0, max as f32) as u32;
        (a, b.max(a))
    };
    let (x0, x1) = span(bounds.x, bounds.x + bounds.width, width);
    let (y0, y1) = span(bounds.y, bounds.y + bounds.height, height);
    let rect = [x0, y0, x1, y1];
    let id = clip_mask.data().as_ptr() as usize;
    let stride = width as usize;

    let last = LAST.with(Cell::get);
    let data = clip_mask.data_mut();

    match last {
        Some((last_id, w, h, last_rect))
            if last_id == id && w == width && h == height =>
        {
            if last_rect == rect
                && (x0 == x1
                    || y0 == y1
                    || data[y0 as usize * stride + x0 as usize] == 255)
            {
                return;
            }
            let [lx0, ly0, lx1, ly1] = last_rect;
            for y in ly0..ly1 {
                let row = y as usize * stride;
                data[row + lx0 as usize..row + lx1 as usize].fill(0);
            }
        }
        // Otra máscara (o la primera vez): se borra entera.
        _ => data.fill(0),
    }

    for y in y0..y1 {
        let row = y as usize * stride;
        data[row + x0 as usize..row + x1 as usize].fill(255);
    }

    LAST.with(|l| l.set(Some((id, width, height, rect))));
}
