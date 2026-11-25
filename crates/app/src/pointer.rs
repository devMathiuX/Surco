//! Dónde se hizo el último clic, para abrir los menús al lado del mouse.
//!
//! [`Pointer`] envuelve toda la ventana. Antes de pasarle un clic a lo de
//! adentro, avisa la posición (y el tamaño de la ventana) con un mensaje.
//! Como sale primero, cuando llega el mensaje del botón que abre un menú ya se
//! sabe dónde dibujarlo. Solo manda mensajes al apretar, no al mover el mouse.

use iced::advanced::layout;
use iced::advanced::mouse;
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector};

pub struct Pointer<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    on_press: fn(Point, Size) -> Message,
}

impl<'a, Message, Theme, Renderer> Pointer<'a, Message, Theme, Renderer> {
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
        on_press: fn(Point, Size) -> Message,
    ) -> Self {
        Self {
            content: content.into(),
            on_press,
        }
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Pointer<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
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
        if let Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left | mouse::Button::Right,
        )) = event
        {
            if let Some(p) = cursor.position() {
                shell.publish((self.on_press)(p, layout.bounds().size()));
            }
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

impl<'a, Message, Theme, Renderer> From<Pointer<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(p: Pointer<'a, Message, Theme, Renderer>) -> Self {
        Element::new(p)
    }
}

/// Posición de un menú de `size` que se abre en `at` dentro de una ventana
/// de `window`: abajo y a la derecha del mouse; si no entra a la derecha, a
/// la izquierda; si no entra abajo, se sube lo justo.
pub fn place(at: Point, size: Size, window: Size) -> Point {
    const GAP: f32 = 4.0;
    let x = if at.x + GAP + size.width <= window.width {
        at.x + GAP
    } else {
        (at.x - GAP - size.width).max(0.0)
    };
    let y = if at.y + GAP + size.height <= window.height {
        at.y + GAP
    } else {
        (window.height - size.height).max(0.0)
    };
    Point::new(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_open_below_and_flip_when_they_do_not_fit() {
        let win = Size::new(1000.0, 640.0);
        let menu = Size::new(300.0, 200.0);
        assert_eq!(
            place(Point::new(100.0, 100.0), menu, win),
            Point::new(104.0, 104.0)
        );
        // Pegado al borde derecho: va a la izquierda del mouse.
        assert_eq!(
            place(Point::new(900.0, 100.0), menu, win),
            Point::new(596.0, 104.0)
        );
        // Abajo de todo: sube lo justo para entrar.
        assert_eq!(
            place(Point::new(100.0, 600.0), menu, win),
            Point::new(104.0, 440.0)
        );
    }
}
