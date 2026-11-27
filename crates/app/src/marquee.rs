//! Títulos que no entran: se deslizan de un lado al otro.
//!
//! [`marquee`] envuelve un texto de una línea. Si entra, se dibuja tal cual.
//! Si no entra y está activo, espera un poco, se corre hacia la izquierda
//! hasta mostrar el final, espera y vuelve. Solo pide cuadros mientras se
//! mueve y está a la vista, a [`FPS`] por segundo, para gastar poco.

use std::time::Duration;

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::time::Instant;
use iced::{window, Element, Event, Length, Rectangle, Size, Vector};

/// Velocidad del desliz, en píxeles por segundo.
const SPEED: f32 = 32.0;
/// Pausa en cada punta, en segundos.
const PAUSE: f32 = 1.6;
/// Cuadros por segundo mientras se mueve. En macOS cada cuadro cuesta más
/// (ver `vendor/iced_tiny_skia`), así que no conviene pasarse.
const FPS: u64 = 24;
/// Margen al final, para que la última letra no quede pegada al borde.
const END: f32 = 6.0;

/// Desliza `content` si no entra. `active`: si se mueve siempre (lo que
/// suena) o solo con el mouse encima (filas de listas, para no redibujar
/// sin parar).
pub fn marquee<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    active: bool,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Marquee {
        content: content.into(),
        active,
    })
}

struct Marquee<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    active: bool,
}

#[derive(Default)]
struct State {
    /// Cuánto más ancho que el lugar es el contenido.
    overflow: f32,
    /// Desde cuándo se mueve.
    start: Option<Instant>,
    /// Corrimiento actual hacia la izquierda.
    offset: f32,
    /// El mouse está encima.
    hovered: bool,
    /// Segundos de animación que lleva `offset`.
    offset_time: f32,
}

/// Corrimiento a los `t` segundos de empezar, para recorrer `travel` px.
fn offset_at(t: f32, travel: f32) -> f32 {
    let slide = travel / SPEED;
    let cycle = 2.0 * (PAUSE + slide);
    let t = t % cycle;
    if t < PAUSE {
        0.0
    } else if t < PAUSE + slide {
        (t - PAUSE) * SPEED
    } else if t < 2.0 * PAUSE + slide {
        travel
    } else {
        travel - (t - 2.0 * PAUSE - slide) * SPEED
    }
    .clamp(0.0, travel)
}

/// Segundos que faltan, a los `t` de empezar, para que termine la pausa en
/// la que está (0 si se está moviendo). Durante las pausas no hace falta
/// pedir cuadros.
fn still_for(t: f32, travel: f32) -> f32 {
    let slide = travel / SPEED;
    let cycle = 2.0 * (PAUSE + slide);
    let t = t % cycle;
    if t < PAUSE {
        PAUSE - t
    } else if t < PAUSE + slide {
        0.0
    } else if t < 2.0 * PAUSE + slide {
        2.0 * PAUSE + slide - t
    } else {
        0.0
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Marquee<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // El contenido se mide sin límite de ancho; el marquee ocupa lo que
        // haya, como mucho.
        let free = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, limits.max().height));
        let child = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &free);
        let natural = child.size();
        let size = limits.resolve(Length::Shrink, Length::Shrink, natural);
        let state = tree.state.downcast_mut::<State>();
        state.overflow = (natural.width - size.width).max(0.0);
        layout::Node::with_children(size, vec![child])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State>();
        // Con el mouse encima también se mueve.
        let hovered = cursor.is_over(bounds);
        if hovered != state.hovered {
            state.hovered = hovered;
            if state.overflow > 0.5 {
                shell.request_redraw();
            }
        }
        if let Event::Window(window::Event::RedrawRequested(now)) = event {
            let moving = (self.active || state.hovered)
                && state.overflow > 0.5
                && viewport.intersects(&bounds);
            if moving && crate::smooth::scrolling(*now) {
                // Mientras se desliza una lista, queda quieto donde estaba
                // (corriendo el comienzo para seguir desde ahí después).
                if let Some(start) = state.start {
                    let t = state.offset_time;
                    state.start =
                        Some(now.checked_sub(Duration::from_secs_f32(t)).unwrap_or(start));
                }
                shell.request_redraw_at(*now + Duration::from_millis(200));
            } else if moving {
                let start = *state.start.get_or_insert(*now);
                let t = now.saturating_duration_since(start).as_secs_f32();
                state.offset_time = t;
                state.offset = offset_at(t, state.overflow + END);
                // En las pausas, el próximo cuadro es cuando vuelve a moverse.
                let wait = Duration::from_secs_f32(still_for(t, state.overflow + END))
                    .max(Duration::from_millis(1000 / FPS));
                shell.request_redraw_at(*now + wait);
            } else {
                if state.offset != 0.0 {
                    shell.request_redraw();
                }
                state.start = None;
                state.offset = 0.0;
                state.offset_time = 0.0;
            }
        }
        if let Some(child) = layout.children().next() {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        layout
            .children()
            .next()
            .map_or(mouse::Interaction::None, |child| {
                self.content.as_widget().mouse_interaction(
                    &tree.children[0],
                    child,
                    cursor,
                    viewport,
                    renderer,
                )
            })
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(child) = layout.children().next() else {
            return;
        };
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        let state = tree.state.downcast_ref::<State>();
        if state.overflow <= 0.5 {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            );
            return;
        }
        let offset = state.offset;
        renderer.with_layer(clip, |renderer| {
            renderer.with_translation(Vector::new(-offset, 0.0), |renderer| {
                self.content.as_widget().draw(
                    &tree.children[0],
                    renderer,
                    theme,
                    style,
                    child,
                    cursor,
                    &Rectangle {
                        x: clip.x + offset,
                        ..clip
                    },
                );
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slides_and_comes_back() {
        let travel = 64.0;
        assert_eq!(offset_at(0.0, travel), 0.0);
        assert!((offset_at(PAUSE + 1.0, travel) - SPEED).abs() < 0.01);
        let slide = travel / SPEED;
        assert_eq!(offset_at(PAUSE + slide + 0.5, travel), travel);
        assert!((still_for(0.5, travel) - (PAUSE - 0.5)).abs() < 0.01);
        assert_eq!(still_for(PAUSE + 1.0, travel), 0.0);
        assert!((still_for(PAUSE + slide + 0.5, travel) - (PAUSE - 0.5)).abs() < 0.01);
        // Vuelta completa: de nuevo al principio.
        assert_eq!(offset_at(2.0 * (PAUSE + slide) + 0.1, travel), 0.0);
    }
}
