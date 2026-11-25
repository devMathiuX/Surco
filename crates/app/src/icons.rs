//! Íconos de los controles, dibujados con canvas: no dependen de qué fuentes
//! tenga instaladas el sistema (los glifos ⏮ ⏯ 🔀 no existen en todas).

use iced::widget::canvas::{self, path, stroke, Frame, Geometry, Path, Stroke};
use iced::widget::{button, Canvas};
use iced::{mouse, Color, Element, Length, Point, Rectangle, Renderer, Theme};

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Play,
    Pause,
    Previous,
    Next,
    Shuffle,
    Repeat,
    RepeatOne,
    Volume,
    Search,
    /// Radio: canciones parecidas.
    Radio,
    Settings,
    /// Cola de reproducción (lista con una nota).
    Queue,
    /// Agregar (+).
    Add,
    /// Cerrar / quitar (×).
    Close,
    /// Agarre para arrastrar (tres rayas).
    Grip,
    /// Inicio (casita).
    Home,
    /// Importar (flecha hacia una bandeja).
    Import,
    /// Volumen apagado (parlante con una cruz).
    Muted,
    /// Editar (lápiz).
    Edit,
}

#[derive(Debug, Clone, Copy)]
pub struct Icon {
    kind: Kind,
    color: Color,
}

impl<Message> canvas::Program<Message> for Icon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // Todo se dibuja en una grilla de 24×24 y se escala.
        let s = bounds.width.min(bounds.height) / 24.0;
        let p = |x: f32, y: f32| Point::new(x * s, y * s);
        let line = |w: f32| {
            Stroke::default()
                .with_color(self.color)
                .with_width(w * s)
                .with_line_cap(stroke::LineCap::Round)
                .with_line_join(stroke::LineJoin::Round)
        };
        let poly = |pts: &[(f32, f32)]| {
            Path::new(|b| {
                if let Some(&(x, y)) = pts.first() {
                    b.move_to(p(x, y));
                }
                for &(x, y) in &pts[1..] {
                    b.line_to(p(x, y));
                }
                b.close();
            })
        };

        match self.kind {
            Kind::Play => frame.fill(&poly(&[(7.0, 4.5), (19.5, 12.0), (7.0, 19.5)]), self.color),
            Kind::Pause => {
                frame.fill(&rect(p(6.0, 5.0), 4.0 * s, 14.0 * s), self.color);
                frame.fill(&rect(p(14.0, 5.0), 4.0 * s, 14.0 * s), self.color);
            }
            Kind::Previous => {
                frame.fill(&rect(p(5.0, 6.0), 2.5 * s, 12.0 * s), self.color);
                frame.fill(&poly(&[(19.0, 6.0), (8.5, 12.0), (19.0, 18.0)]), self.color);
            }
            Kind::Next => {
                frame.fill(&rect(p(16.5, 6.0), 2.5 * s, 12.0 * s), self.color);
                frame.fill(&poly(&[(5.0, 6.0), (15.5, 12.0), (5.0, 18.0)]), self.color);
            }
            Kind::Shuffle => {
                let st = line(1.8);
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(p(4.0, 7.0));
                        b.line_to(p(8.0, 7.0));
                        b.line_to(p(15.0, 17.0));
                        b.line_to(p(19.5, 17.0));
                        b.move_to(p(4.0, 17.0));
                        b.line_to(p(8.0, 17.0));
                        b.line_to(p(15.0, 7.0));
                        b.line_to(p(19.5, 7.0));
                        // Puntas de flecha.
                        b.move_to(p(17.0, 4.5));
                        b.line_to(p(19.5, 7.0));
                        b.line_to(p(17.0, 9.5));
                        b.move_to(p(17.0, 14.5));
                        b.line_to(p(19.5, 17.0));
                        b.line_to(p(17.0, 19.5));
                    }),
                    st,
                );
            }
            Kind::Repeat | Kind::RepeatOne => {
                let st = line(1.8);
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(p(5.0, 11.0));
                        b.line_to(p(5.0, 8.0));
                        b.line_to(p(18.0, 8.0));
                        b.move_to(p(15.5, 5.5));
                        b.line_to(p(18.0, 8.0));
                        b.line_to(p(15.5, 10.5));
                        b.move_to(p(19.0, 13.0));
                        b.line_to(p(19.0, 16.0));
                        b.line_to(p(6.0, 16.0));
                        b.move_to(p(8.5, 13.5));
                        b.line_to(p(6.0, 16.0));
                        b.line_to(p(8.5, 18.5));
                    }),
                    st,
                );
                if self.kind == Kind::RepeatOne {
                    // Un "1" chico en el centro.
                    frame.stroke(
                        &Path::new(|b| {
                            b.move_to(p(11.0, 11.0));
                            b.line_to(p(12.5, 10.0));
                            b.line_to(p(12.5, 14.0));
                        }),
                        line(1.5),
                    );
                }
            }
            Kind::Muted => {
                frame.fill(
                    &poly(&[
                        (4.0, 9.5),
                        (8.0, 9.5),
                        (12.5, 5.5),
                        (12.5, 18.5),
                        (8.0, 14.5),
                        (4.0, 14.5),
                    ]),
                    self.color,
                );
                frame.stroke(&Path::line(p(15.0, 9.5), p(20.0, 14.5)), line(1.8));
                frame.stroke(&Path::line(p(20.0, 9.5), p(15.0, 14.5)), line(1.8));
            }
            Kind::Edit => {
                // Lápiz en diagonal: cuerpo, punta y una raya abajo.
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(p(15.5, 4.5));
                        b.line_to(p(19.5, 8.5));
                        b.line_to(p(9.0, 19.0));
                        b.line_to(p(4.5, 19.5));
                        b.line_to(p(5.0, 15.0));
                        b.close();
                        b.move_to(p(13.0, 7.0));
                        b.line_to(p(17.0, 11.0));
                    }),
                    line(1.7),
                );
            }
            Kind::Volume => {
                frame.fill(
                    &poly(&[
                        (4.0, 9.5),
                        (8.0, 9.5),
                        (12.5, 5.5),
                        (12.5, 18.5),
                        (8.0, 14.5),
                        (4.0, 14.5),
                    ]),
                    self.color,
                );
                frame.stroke(
                    &Path::new(|b| {
                        b.arc(path::Arc {
                            center: p(13.0, 12.0),
                            radius: 4.5 * s,
                            start_angle: iced::Radians(-0.9),
                            end_angle: iced::Radians(0.9),
                        });
                    }),
                    line(1.8),
                );
            }
            Kind::Search => {
                frame.stroke(&Path::circle(p(10.5, 10.5), 5.5 * s), line(2.0));
                frame.stroke(&Path::line(p(14.8, 14.8), p(19.5, 19.5)), line(2.0));
            }
            Kind::Radio => {
                // Un punto con ondas a cada lado.
                frame.fill(&Path::circle(p(12.0, 12.0), 2.2 * s), self.color);
                for (r, w) in [(5.0, 1.8), (8.5, 1.8)] {
                    for (a, b) in [
                        (-0.8_f32, 0.8_f32),
                        (std::f32::consts::PI - 0.8, std::f32::consts::PI + 0.8),
                    ] {
                        frame.stroke(
                            &Path::new(|bld| {
                                bld.arc(path::Arc {
                                    center: p(12.0, 12.0),
                                    radius: r * s,
                                    start_angle: iced::Radians(a),
                                    end_angle: iced::Radians(b),
                                });
                            }),
                            line(w),
                        );
                    }
                }
            }
            Kind::Queue => {
                let st = line(1.8);
                for y in [6.0, 11.0] {
                    frame.stroke(&Path::line(p(4.0, y), p(15.0, y)), st);
                }
                frame.stroke(&Path::line(p(4.0, 16.0), p(10.0, 16.0)), st);
                // Nota musical.
                frame.stroke(&Path::line(p(18.5, 7.0), p(18.5, 16.5)), line(1.8));
                frame.fill(&Path::circle(p(16.5, 17.0), 2.3 * s), self.color);
            }
            Kind::Add => {
                frame.stroke(&Path::line(p(12.0, 5.0), p(12.0, 19.0)), line(2.0));
                frame.stroke(&Path::line(p(5.0, 12.0), p(19.0, 12.0)), line(2.0));
            }
            Kind::Close => {
                frame.stroke(&Path::line(p(6.0, 6.0), p(18.0, 18.0)), line(2.0));
                frame.stroke(&Path::line(p(18.0, 6.0), p(6.0, 18.0)), line(2.0));
            }
            Kind::Grip => {
                for y in [8.0, 12.0, 16.0] {
                    frame.stroke(&Path::line(p(6.0, y), p(18.0, y)), line(1.6));
                }
            }
            Kind::Home => {
                let st = line(1.9);
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(p(3.5, 11.5));
                        b.line_to(p(12.0, 4.5));
                        b.line_to(p(20.5, 11.5));
                        b.move_to(p(6.0, 9.5));
                        b.line_to(p(6.0, 19.5));
                        b.line_to(p(18.0, 19.5));
                        b.line_to(p(18.0, 9.5));
                    }),
                    st,
                );
                frame.fill(&rect(p(10.0, 13.5), 4.0 * s, 6.0 * s), self.color);
            }
            Kind::Import => {
                let st = line(1.9);
                frame.stroke(
                    &Path::new(|b| {
                        b.move_to(p(12.0, 4.0));
                        b.line_to(p(12.0, 14.5));
                        b.move_to(p(7.5, 10.5));
                        b.line_to(p(12.0, 15.0));
                        b.line_to(p(16.5, 10.5));
                        b.move_to(p(4.5, 15.0));
                        b.line_to(p(4.5, 19.5));
                        b.line_to(p(19.5, 19.5));
                        b.line_to(p(19.5, 15.0));
                    }),
                    st,
                );
            }
            Kind::Settings => {
                // Engranaje: aro y 8 dientes.
                frame.stroke(&Path::circle(p(12.0, 12.0), 5.0 * s), line(2.0));
                frame.stroke(&Path::circle(p(12.0, 12.0), 1.8 * s), line(1.6));
                for i in 0..8 {
                    let ang = i as f32 * std::f32::consts::FRAC_PI_4;
                    let (sin, cos) = ang.sin_cos();
                    frame.stroke(
                        &Path::line(
                            p(12.0 + 6.0 * cos, 12.0 + 6.0 * sin),
                            p(12.0 + 8.5 * cos, 12.0 + 8.5 * sin),
                        ),
                        line(2.6),
                    );
                }
            }
        }
        vec![frame.into_geometry()]
    }
}

fn rect(top_left: Point, w: f32, h: f32) -> Path {
    Path::rounded_rectangle(top_left, iced::Size::new(w, h), (1.0_f32).into())
}

/// Ícono suelto de `size` px.
pub fn icon<'a, Message: 'a>(kind: Kind, color: Color, size: f32) -> Element<'a, Message> {
    Canvas::new(Icon { kind, color })
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

/// Botón con ícono.
pub fn icon_button<'a, Message: Clone + 'a>(
    kind: Kind,
    size: f32,
    on_press: Message,
) -> Element<'a, Message> {
    button(icon(kind, theme::colors().text, size))
        .on_press(on_press)
        .padding(6)
        .style(theme::icon_button)
        .into()
}

/// Botón de un modo que se prende y apaga (shuffle, repetir): acento si está prendido.
pub fn toggle_button<'a, Message: Clone + 'a>(
    kind: Kind,
    size: f32,
    on: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let color = if on {
        theme::colors().accent
    } else {
        theme::colors().text_secondary
    };
    button(icon(kind, color, size))
        .on_press(on_press)
        .padding(6)
        .style(theme::icon_button)
        .into()
}

/// Ícono como texto, para filas dentro de listas con scroll.
///
/// El renderer por CPU (tiny-skia) de iced 0.14 a veces no repinta la zona de
/// un `canvas` que salió de la vista al hacer scroll y deja "fantasmas"; con
/// texto no pasa. Son caracteres que traen las fuentes de todos los sistemas.
pub fn glyph<'a, Message: 'a>(kind: Kind, color: Color, size: f32) -> Element<'a, Message> {
    let c = match kind {
        Kind::Add => "+",
        Kind::Close => "×",
        Kind::Grip => "≡",
        Kind::Volume => "♪",
        _ => "•",
    };
    iced::widget::text(c)
        .size(size)
        .color(color)
        .width(Length::Fixed(size))
        .align_x(iced::alignment::Horizontal::Center)
        .into()
}
