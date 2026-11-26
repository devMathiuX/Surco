//! Scroll suave con la rueda del mouse.
//!
//! El `scrollable` de iced salta 60 px de golpe por cada paso de la rueda.
//! [`Smooth`] envuelve un `scrollable`: se queda con esos pasos y se los va
//! pasando de a poco, cuadro a cuadro, como desplazamientos en píxeles (lo
//! mismo que manda un touchpad). Los touchpads ya mandan píxeles: esos pasan
//! tal cual.
//!
//! El movimiento es un resorte con amortiguación crítica que persigue el
//! destino: arranca suave, acelera, y frena sin pasarse. Cada paso nuevo de
//! la rueda corre el destino sin cortar la velocidad que traía, así que
//! girar la rueda seguido da un deslizamiento continuo, con inercia.

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
/// Rigidez del resorte (1/s). Con 18, un paso recorre el 90 % en ~210 ms y
/// termina a los ~350 ms.
const OMEGA: f32 = 18.0;
/// Debajo de esto (en píxeles y píxeles por segundo) se termina.
const DONE: f32 = 1.0;
const DONE_SPEED: f32 = 20.0;
/// Si un cuadro tarda más que esto, se cuenta como si hubiera tardado esto.
const MAX_DT: f32 = 0.05;

/// Envuelve un `scrollable` para que la rueda lo mueva suave.
pub fn smooth<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Smooth {
        content: content.into(),
        keep: None,
    })
}

/// Como [`smooth`], pero de los mensajes del `scrollable` solo pasan los que
/// cumplen `keep`. Cada mensaje rearma toda la vista, y `on_scroll` manda uno
/// por cuadro: así la lista avisa solo cuando cambia algo que importa.
pub fn smooth_filtered<'a, Message: 'a, Theme: 'a, Renderer: renderer::Renderer + 'a>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    keep: impl Fn(&Message) -> bool + 'a,
) -> Element<'a, Message, Theme, Renderer> {
    Element::new(Smooth {
        content: content.into(),
        keep: Some(Box::new(keep)),
    })
}

/// Qué mensajes del contenido pasan (`None`: todos).
type Keep<'a, Message> = Option<Box<dyn Fn(&Message) -> bool + 'a>>;

struct Smooth<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    keep: Keep<'a, Message>,
}

/// Le pasa el evento al contenido, filtrando sus mensajes con `keep`.
#[allow(clippy::too_many_arguments)]
fn forward<Message, Theme, Renderer: renderer::Renderer>(
    content: &mut Element<'_, Message, Theme, Renderer>,
    keep: &Keep<'_, Message>,
    tree: &mut Tree,
    event: &Event,
    layout: Layout<'_>,
    cursor: mouse::Cursor,
    renderer: &Renderer,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, Message>,
    viewport: &Rectangle,
) {
    let Some(keep) = keep else {
        content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
        return;
    };
    let mut messages = Vec::new();
    let mut inner = Shell::new(&mut messages);
    content.as_widget_mut().update(
        tree, event, layout, cursor, renderer, clipboard, &mut inner, viewport,
    );
    if inner.is_event_captured() {
        shell.capture_event();
    }
    shell.request_redraw_at(inner.redraw_request());
    if inner.is_layout_invalid() {
        shell.invalidate_layout();
    }
    if inner.are_widgets_invalid() {
        shell.invalidate_widgets();
    }
    shell.request_input_method(inner.input_method());
    drop(inner);
    for m in messages {
        if keep(&m) {
            shell.publish(m);
        }
    }
}

#[derive(Default)]
struct State {
    /// Lo que falta recorrer (positivo: hacia abajo).
    pending: f32,
    /// Velocidad actual, en píxeles por segundo (positivo: hacia abajo).
    velocity: f32,
    last_frame: Option<Instant>,
    shift: bool,
}

impl State {
    fn moving(&self) -> bool {
        self.pending != 0.0
    }
}

/// Avanza el resorte `dt` segundos. Devuelve cuánto moverse en este cuadro
/// y deja en `pending` y `velocity` lo que queda.
fn step(pending: &mut f32, velocity: &mut f32, dt: f32) -> f32 {
    // Solución exacta del resorte críticamente amortiguado, con
    // d = posición - destino = -pending.
    let d = -*pending;
    let v = *velocity;
    let e = (-OMEGA * dt).exp();
    let k = v + OMEGA * d;
    let d2 = (d + k * dt) * e;
    let v2 = (v - OMEGA * k * dt) * e;
    if d2.abs() < DONE && v2.abs() < DONE_SPEED {
        // Llegó: lo que falta, de una.
        let s = *pending;
        *pending = 0.0;
        *velocity = 0.0;
        return s;
    }
    *pending = -d2;
    *velocity = v2;
    d2 - d
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
                // Cambio de sentido: lo que faltaba del otro lado se olvida
                // y frena en seco.
                if state.moving() && state.pending.signum() != target.signum() {
                    state.pending = 0.0;
                    state.velocity = 0.0;
                }
                if !state.moving() {
                    state.last_frame = None;
                }
                state.pending += target;
                shell.capture_event();
                shell.request_redraw();
                return;
            }
            // iced manda el mismo cuadro hasta 3 veces si la vista cambió
            // (la lista avisa que se movió): solo la primera mueve, pero
            // todas piden el cuadro siguiente (vale el pedido de la última).
            Event::Window(window::Event::RedrawRequested(now))
                if state.moving() && state.last_frame == Some(*now) =>
            {
                shell.request_redraw();
            }
            Event::Window(window::Event::RedrawRequested(now)) if state.moving() => {
                let dt = state
                    .last_frame
                    .map(|t| now.saturating_duration_since(t).as_secs_f32())
                    // Primer cuadro: como si hubiera pasado uno de 60 Hz.
                    .unwrap_or(1.0 / 60.0)
                    .min(MAX_DT);
                state.last_frame = Some(*now);
                let s = step(&mut state.pending, &mut state.velocity, dt);
                // Al `scrollable` le llega como un desplazamiento en píxeles,
                // con el mouse en el medio de la lista (aunque se haya ido).
                let at = cursor
                    .position()
                    .filter(|p| bounds.contains(*p))
                    .unwrap_or_else(|| bounds.center());
                let wheel = Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -s },
                });
                forward(
                    &mut self.content,
                    &self.keep,
                    &mut children[0],
                    &wheel,
                    layout,
                    mouse::Cursor::Available(at),
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
                if state.moving() {
                    shell.request_redraw();
                }
            }
            _ => {}
        }
        // Mientras se desliza, nada queda "con el mouse encima": si no, un
        // botón que pasa por debajo del mouse cambia de color (o se queda
        // con el color de encima después de irse).
        let cursor = if tree.state.downcast_ref::<State>().moving() {
            cursor.levitate()
        } else {
            cursor
        };
        forward(
            &mut self.content,
            &self.keep,
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

    /// Simula `seconds` a `fps`. Devuelve lo recorrido en cada cuadro.
    fn run(pending: &mut f32, velocity: &mut f32, fps: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|_| step(pending, velocity, 1.0 / fps))
            .collect()
    }

    #[test]
    fn one_notch_glides_and_lands_exactly() {
        let (mut p, mut v) = (LINE, 0.0);
        let moves = run(&mut p, &mut v, 60.0, 60);
        assert_eq!(p, 0.0, "no terminó en un segundo");
        assert_eq!(v, 0.0);
        let total: f32 = moves.iter().sum();
        assert!((total - LINE).abs() < 1e-3, "recorrió {total}");
        // Nunca se pasa ni vuelve para atrás.
        assert!(moves.iter().all(|m| *m >= 0.0));
        // Arranca suave: el primer cuadro mueve poco.
        assert!(moves[0] < LINE * 0.1, "primer cuadro: {}", moves[0]);
        // Dura entre 200 y 500 ms.
        let frames = moves.iter().filter(|m| **m != 0.0).count();
        assert!((12..=30).contains(&frames), "{frames} cuadros");
    }

    #[test]
    fn new_notch_keeps_the_speed() {
        let (mut p, mut v) = (LINE, 0.0);
        run(&mut p, &mut v, 60.0, 5);
        let before = v;
        assert!(before > 0.0);
        p += LINE;
        let moves = run(&mut p, &mut v, 60.0, 1);
        // Sigue acelerando, no frena ni salta.
        assert!(v >= before && moves[0] > 0.0);
        let rest: f32 = run(&mut p, &mut v, 60.0, 120).iter().sum();
        assert_eq!(p, 0.0);
        assert!(rest > 0.0);
    }

    #[test]
    fn works_upwards_and_at_low_fps() {
        let (mut p, mut v) = (-2.0 * LINE, 0.0);
        let total: f32 = run(&mut p, &mut v, 24.0, 48).iter().sum();
        assert_eq!(p, 0.0);
        assert!((total + 2.0 * LINE).abs() < 1e-3);
    }
}
