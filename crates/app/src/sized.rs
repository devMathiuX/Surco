//! `responsive` de iced que arma su contenido solo cuando cambia el tamaño.
//!
//! El de iced vuelve a llamar a la vista en cada `layout`, y deslizar una
//! lista rehace el layout en cada cuadro: los botones de adentro se crean de
//! nuevo y se dibujan sin estado, con el color de "deshabilitado" (el botón
//! "Escuchar ahora" se oscurecía al deslizar el inicio). Copiado de
//! iced_widget 0.14 (MIT) con ese cambio.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{mouse, Element, Event, Length, Rectangle, Size, Vector};

/// A widget that is aware of its dimensions.
///
/// A [`Responsive`] widget will always try to fill all the available space of
/// its parent.
pub struct Responsive<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    view: Box<dyn Fn(Size) -> Element<'a, Message, Theme, Renderer> + 'a>,
    width: Length,
    height: Length,
    content: Element<'a, Message, Theme, Renderer>,
    /// Tamaño con el que se armó `content` (`None`: todavía no se armó).
    built_for: Option<Size>,
}

impl<'a, Message, Theme, Renderer> Responsive<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    /// Creates a new [`Responsive`] widget with a closure that produces its
    /// contents.
    ///
    /// The `view` closure will receive the maximum available space for
    /// the [`Responsive`] during layout. You can use this [`Size`] to
    /// conditionally build the contents.
    pub fn new(view: impl Fn(Size) -> Element<'a, Message, Theme, Renderer> + 'a) -> Self {
        Self {
            view: Box::new(view),
            width: Length::Fill,
            height: Length::Fill,
            content: Element::new(iced::widget::Space::new()),
            built_for: None,
        }
    }

    /// Sets the width of the [`Responsive`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Responsive`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Responsive<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn diff(&self, _tree: &mut Tree) {
        // Diff is deferred to layout
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        let size = limits.max();

        if self.built_for != Some(size) {
            self.content = (self.view)(size);
            self.built_for = Some(size);
            tree.diff_children(std::slice::from_ref(&self.content));
        }

        let node =
            self.content
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &limits.loose());

        let size = limits.resolve(self.width, self.height, node.size());

        layout::Node::with_children(size, vec![node])
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            first(layout),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
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
            first(layout),
            cursor,
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
            first(layout),
            cursor,
            viewport,
            renderer,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            first(layout),
            renderer,
            operation,
        );
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            first(layout),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Responsive<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(responsive: Responsive<'a, Message, Theme, Renderer>) -> Self {
        Self::new(responsive)
    }
}

/// El único hijo del layout.
fn first(layout: Layout<'_>) -> Layout<'_> {
    layout.children().next().unwrap_or(layout)
}

/// Igual que [`iced::widget::responsive`].
pub fn responsive<'a, Message, Theme, Renderer>(
    view: impl Fn(Size) -> Element<'a, Message, Theme, Renderer> + 'a,
) -> Responsive<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    Responsive::new(view)
}
