//! Bienvenida: la primera vez que se abre Surco, elegir el tema y los
//! ajustes principales antes de empezar.

use iced::widget::{button, column, container, row, scrollable, text, toggler, Space};
use iced::{gradient, Alignment, Background, Color, Element, Length, Radians, Task};

use crate::settings::Change;
use crate::theme::{self, Colors, Look};
use crate::{icons, Message, Screen, Surco};

/// Ajuste que marca que la bienvenida ya se mostró.
const SETUP_DONE: &str = "setup_done";

impl Surco {
    /// Decide si mostrar la bienvenida: solo la primera vez. Quien ya tenía
    /// playlists (actualizó desde otra versión) no la ve.
    pub(crate) fn maybe_welcome(&mut self) {
        let done = self
            .store
            .as_ref()
            .and_then(|s| s.setting(SETUP_DONE).ok().flatten())
            .is_some();
        if done {
            return;
        }
        if self.playlists.is_empty() {
            self.screen = Screen::Welcome;
        } else {
            self.save_setting(SETUP_DONE, "1");
        }
    }

    pub(crate) fn finish_welcome(&mut self) -> Task<Message> {
        self.save_setting(SETUP_DONE, "1");
        self.screen = Screen::Home;
        Task::none()
    }

    pub(crate) fn view_welcome(&self) -> Element<'_, Message> {
        let section = |title: &'static str| text(title).size(theme::SIZE_M);
        let themes = self.theme_cards(false, false);
        let switch = |label: &'static str, on: bool, f: fn(bool) -> Change| {
            toggler(on)
                .label(label)
                .on_toggle(move |v| Message::Setting(f(v)))
                .size(20)
                .text_size(theme::SIZE_S)
                .spacing(10)
                .style(theme::switch)
        };
        let body = column![
            row![
                icons::logo(56.0),
                column![
                    text(tr!("Te damos la bienvenida a Surco", "Welcome to Surco"))
                        .size(26),
                    text(tr!(
                        "Elegí cómo querés que se vea. Todo esto se puede cambiar después en Ajustes.",
                        "Pick how you want it to look. You can change all of this later in Settings."
                    ))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                ]
                .spacing(4),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
            column![section(tr!("Tema", "Theme")), themes].spacing(10),
            row![
                column![section(tr!("Idioma", "Language")), self.languages_row()].spacing(10),
                column![
                    section(tr!("Tamaño de la interfaz", "Interface size")),
                    self.scale_row()
                ]
                .spacing(10),
            ]
            .spacing(48),
            column![
                section(tr!("Precargar canciones", "Preload songs")),
                text(tr!(
                    "Baja las canciones antes de que suenen, para no esperar. Se borra al cerrar Surco.",
                    "Downloads songs before they play, so you don't wait. It's cleared when Surco closes."
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
                self.precache_row(),
            ]
            .spacing(10),
            column![
                switch(
                    tr!("Mostrar portadas en la lista", "Show covers in the list"),
                    self.settings.covers_in_list,
                    Change::CoversInList
                ),
                switch(
                    tr!(
                        "Al terminar la cola, seguir con canciones parecidas",
                        "When the queue ends, keep playing similar songs"
                    ),
                    self.settings.autoplay,
                    Change::Autoplay
                ),
            ]
            .spacing(12),
            row![
                Space::new().width(Length::Fill),
                button(text(tr!("Empezar", "Get started")).size(theme::SIZE_M))
                    .on_press(Message::FinishWelcome)
                    .padding([10, 28])
                    .style(theme::primary_button),
            ],
        ]
        .spacing(28)
        .max_width(860);
        container(crate::smooth::smooth(scrollable(
            container(body).padding([40, 48]).center_x(Length::Fill),
        )))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    /// Tarjetas de los temas (bienvenida y ajustes). `custom` suma la del
    /// tema personalizado, con los colores que eligió el usuario.
    /// `compact`: tarjetas más chicas, con una vista previa simplificada
    /// (ajustes).
    pub(crate) fn theme_cards(&self, custom: bool, compact: bool) -> Element<'_, Message> {
        let n = if custom {
            theme::CUSTOM + 1
        } else {
            theme::THEMES.len()
        };
        row((0..n).map(|i| self.theme_card(i, compact)))
            .spacing(if compact { 10 } else { 12 })
            .wrap()
            .into()
    }

    /// Tarjeta de un tema con una vista previa chiquita: fondo, panel lateral,
    /// una fila resaltada y el acento.
    fn theme_card(&self, index: usize, compact: bool) -> Element<'_, Message> {
        let t = theme::THEMES
            .get(index)
            .copied()
            .unwrap_or_else(|| theme::derive(self.settings.custom.0, self.settings.custom.1));
        let selected = self.settings.theme == index;
        let radius = t.radius.min(8.0);
        let block = |w: f32, h: f32, c: Color| {
            container(Space::new())
                .width(w)
                .height(h)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(c.into()),
                    border: iced::border::rounded(radius.min(3.0)),
                    ..container::Style::default()
                })
        };
        let side = column![
            block(30.0, 5.0, t.accent),
            block(36.0, 4.0, t.text_secondary),
            block(28.0, 4.0, t.text_secondary),
        ]
        .spacing(6)
        .padding(8)
        .width(52)
        .height(Length::Fill);
        let side = container(side).style(move |_: &iced::Theme| container::Style {
            background: Some(t.surface.into()),
            ..container::Style::default()
        });
        let content = column![
            block(70.0, 6.0, t.text),
            container(block(60.0, 4.0, t.text_secondary))
                .padding(6)
                .width(Length::Fill)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(t.raised.into()),
                    border: if t.look == Look::Brutal {
                        iced::border::rounded(0).width(1.5).color(t.accent)
                    } else {
                        iced::border::rounded(radius.min(4.0))
                    },
                    ..container::Style::default()
                }),
            block(50.0, 4.0, t.text_secondary),
            row![
                Space::new().width(Length::Fill),
                block(14.0, 14.0, t.accent).style(move |_: &iced::Theme| container::Style {
                    background: Some(t.accent.into()),
                    border: iced::border::rounded(999),
                    ..container::Style::default()
                })
            ],
        ]
        .spacing(7)
        .padding(10)
        .width(Length::Fill);
        let (width, height) = if compact {
            (118.0, 68.0)
        } else {
            (170.0, 104.0)
        };
        let inner: Element<'_, Message> = if compact {
            // Una ventanita con el acento y dos renglones.
            let window = container(
                row![
                    block(20.0, 20.0, t.accent).style(move |_: &iced::Theme| container::Style {
                        background: Some(t.accent.into()),
                        border: iced::border::rounded(999),
                        ..container::Style::default()
                    }),
                    column![block(46.0, 5.0, t.text), block(32.0, 4.0, t.text_secondary)]
                        .spacing(5),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding(9)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(t.raised.into()),
                border: if t.look == Look::Brutal {
                    iced::border::rounded(0).width(1.5).color(t.accent)
                } else {
                    iced::border::rounded(radius.min(6.0))
                },
                ..container::Style::default()
            });
            container(window).center(Length::Fill).into()
        } else {
            row![side, content].into()
        };
        let preview = container(inner)
            .width(width)
            .height(height)
            .clip(true)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(preview_background(&t)),
                border: iced::border::rounded(radius).width(1).color(t.border),
                ..container::Style::default()
            });
        let accent = t.accent;
        // Tilde arriba a la derecha del tema elegido.
        let check: Element<'_, Message> = if selected {
            container(
                container(text("✓").size(11).color(t.on_accent))
                    .center(18)
                    .style(move |_: &iced::Theme| container::Style {
                        background: Some(accent.into()),
                        border: iced::border::rounded(999),
                        ..container::Style::default()
                    }),
            )
            .padding(6)
            .align_right(width)
            .into()
        } else {
            Space::new().into()
        };
        // Puntito de opción: lleno con el acento del tema si es el elegido.
        let dot = container(Space::new())
            .width(12)
            .height(12)
            .style(move |_: &iced::Theme| {
                let c = theme::colors();
                container::Style {
                    background: Some(if selected { c.accent } else { accent }.into()),
                    border: iced::border::rounded(999)
                        .width(if selected { 3 } else { 0 })
                        .color(c.raised),
                    ..container::Style::default()
                }
            });
        button(
            column![
                iced::widget::stack![preview, check],
                row![dot, text(theme::name_of(index)).size(theme::SIZE_S)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            ]
            .spacing(10),
        )
        .on_press(Message::Setting(Change::Theme(index)))
        .padding(8)
        .style(move |th, status| {
            let c = theme::colors();
            let mut st = theme::list_button(false)(th, status);
            st.text_color = c.text;
            st.border = iced::border::rounded(theme::radius() + 4.0)
                .width(if selected { 2.0 } else { 1.0 })
                .color(if selected { c.accent } else { c.border });
            st
        })
        .into()
    }
}

/// Fondo de la vista previa: los temas con fondo dibujado llevan un
/// degradado que lo recuerda.
fn preview_background(t: &Colors) -> Background {
    let linear = |a: u32, b: u32| {
        let c = |v: u32| Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8);
        gradient::Linear::new(Radians(2.4))
            .add_stop(0.0, c(a))
            .add_stop(1.0, c(b))
    };
    match t.look {
        Look::Glass => Background::Gradient(
            linear(0x1E7C82, 0x8E4C9E)
                .add_stop(0.5, Color::from_rgb8(0x4A, 0x3A, 0x8E))
                .into(),
        ),
        Look::Glow => Background::Gradient(linear(0x09090F, 0x35247E).into()),
        Look::Flat | Look::Brutal | Look::Minimal => t.background.into(),
    }
}
