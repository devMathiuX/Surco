//! Scroll suave con la rueda del mouse.
//!
//! El `scrollable` de iced salta 60 px de golpe por cada paso de la rueda.
//! [`Smooth`] envuelve un `scrollable`: se queda con esos pasos y se los va
//! pasando de a poco, cuadro a cuadro, como desplazamientos en píxeles (lo
//! mismo que manda un touchpad), con una curva que frena al final. Los
//! touchpads ya mandan píxeles: esos pasan tal cual.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{tree, Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::time::Instant;
use iced::{keyboard, window, Element, Event, Length, Rectangle, Size, Vector};

/// Píxeles por paso de la rueda (lo mismo que iced).
const LINE: f32 = 60.0;
/// Qué tan rápido llega al destino: con 14, a los ~150 ms ya recorrió el 90 %.
const SPEED: f32 = 14.0;
/// Debajo de esto se termina la animación.
const DONE: f32 = 0.5;

/// Envuelve un `scrollable` para que la rueda lo mueva suave.
pub fn smooth<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Smooth {
        content: content.into(),
    })
}

struct Smooth<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

#[derive(Default)]
struct State {
    /// Lo que falta recorrer (positivo: hacia abajo).
    pending: f32,
    last_frame: Option<Instant>,
    shift: bool,
}

/// Cuánto avanzar en este cuadro de lo que falta (`pending`), si pasaron
/// `dt` segundos desde el anterior.
fn step(pending: f32, dt: f32) -> f32 {
    if pending.abs() <= DONE {
        return pending;
    }
    let s = pending * (1.0 - (-SPEED * dt).exp());
    // Al menos medio píxel, para no quedar arrastrándose.
    if s.abs() < DONE {
        DONE.copysign(pending)
    } else {
        s
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Smooth<'_, Message, Theme, Renderer>
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
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
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
        let (state_tree, children) = (&mut tree.state, &mut tree.children);
        let state = state_tree.downcast_mut::<State>();
        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.shift = m.shift(),
            // Un paso de la rueda vertical sobre la lista: se anima.
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x, y },
            }) if *x == 0.0 && !state.shift && cursor.is_over(bounds) => {
                let target = -y * LINE;
                // Cambio de sentido: lo que faltaba del otro lado se olvida.
                if state.pending != 0.0 && state.pending.signum() != target.signum() {
                    state.pending = 0.0;
                }
                if state.pending == 0.0 {
                    state.last_frame = None;
                }
                state.pending += target;
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            Event::Window(window::Event::RedrawRequested(now)) if state.pending != 0.0 => {
                let dt = state
                    .last_frame
                    .map(|t| now.saturating_duration_since(t).as_secs_f32())
                    // Primer cuadro: como si hubiera pasado uno de 60 Hz.
                    .unwrap_or(1.0 / 60.0)
                    .min(0.1);
                state.last_frame = Some(*now);
                let s = step(state.pending, dt);
                state.pending -= s;
                if state.pending.abs() < f32::EPSILON {
                    state.pending = 0.0;
                }
                // Al `scrollable` le llega como un desplazamiento en píxeles,
                // con el mouse en el medio de la lista (aunque se haya ido).
                let at = cursor
                    .position()
                    .filter(|p| bounds.contains(*p))
                    .unwrap_or_else(|| bounds.center());
                let wheel = Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -s },
                });
                self.content.as_widget_mut().update(
                    &mut children[0],
                    &wheel,
                    layout,
                    mouse::Cursor::Available(at),
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
                if state.pending != 0.0 {
                    shell.request_redraw();
                }
            }
            _ => {}
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_out_and_finishes() {
        let mut pending = 3.0 * LINE;
        let mut frames = 0;
        let mut first = None;
        while pending != 0.0 {
            let s = step(pending, 1.0 / 60.0);
            first.get_or_insert(s);
            pending -= s;
            if pending.abs() < f32::EPSILON {
                pending = 0.0;
            }
            frames += 1;
            assert!(frames < 200, "no termina");
        }
        // Arranca rápido (más de 30 px en el primer cuadro) y frena.
        assert!(first.unwrap() > 30.0);
        assert!(frames > 10, "demasiado brusco: {frames} cuadros");
        // Hacia arriba igual.
        assert!(step(-LINE, 1.0 / 60.0) < 0.0);
    }
}
