//! Cola de reproducción: ver lo que sigue, agregar, quitar, saltar y
//! reordenar arrastrando.
//!
//! El arrastre: apretar sobre una fila la toma (`DragStart`), pasar por
//! encima de otra la marca como destino (`DragOver`) y soltar el botón en
//! cualquier lado (evento global) la mueve (`DragEnd`). Apretar y soltar
//! sobre la misma fila es un clic: salta a esa canción.

use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Column, Space};
use iced::{mouse, Alignment, Element, Length, Task};
use surco_core::{format_duration, Track};

use crate::icons::{self, Kind};
use crate::playback::{Active, TrackRes};
use crate::{theme, Message, Notice, Surco};

impl Surco {
    /// Agrega una canción a la cola (al final, o después de la actual).
    pub(crate) fn enqueue(&mut self, track: Track, next: bool) -> Task<Message> {
        let title = track.title.clone();
        match &mut self.active {
            Some(a) => {
                a.tracks.push(track);
                let i = a.tracks.len() - 1;
                if next {
                    a.queue.insert_next(i);
                } else {
                    a.queue.push(i);
                }
            }
            // Nada sonando: la cola arranca con esta.
            None => {
                self.active = Some(Active::new(None, vec![track], 0, false, self.repeat));
                return self.start_current();
            }
        }
        self.extend_failed = None;
        self.notice = Some(Notice {
            error: false,
            text: if next {
                format!("«{title}» suena después de la actual.")
            } else {
                format!("«{title}» agregada a la cola.")
            },
        });
        self.update_preload();
        self.prefetch()
    }

    pub(crate) fn queue_remove(&mut self, pos: usize) {
        if let Some(a) = &mut self.active {
            a.queue.remove_at(pos);
        }
        self.update_preload();
    }

    pub(crate) fn queue_jump(&mut self, pos: usize) -> Task<Message> {
        let Some(a) = &mut self.active else {
            return Task::none();
        };
        if pos == a.queue.pos() {
            return Task::none();
        }
        if a.queue.jump_to_pos(pos).is_none() {
            return Task::none();
        }
        self.failures_in_a_row = 0;
        self.start_current()
    }

    /// Se soltó el botón: mover la fila arrastrada o, si no se movió, saltar a ella.
    pub(crate) fn drag_end(&mut self) -> Task<Message> {
        let Some((from, to)) = self.drag.take() else {
            return Task::none();
        };
        if from == to {
            return self.queue_jump(from);
        }
        if let Some(a) = &mut self.active {
            a.queue.move_item(from, to);
        }
        self.update_preload();
        self.prefetch()
    }

    pub(crate) fn view_queue(&self) -> Element<'_, Message> {
        let Some(a) = &self.active else {
            return column![
                text("Cola").size(theme::SIZE_L),
                text("No hay nada sonando. Elegí una canción de una playlist o del buscador; con el + de cada canción la agregás a la cola.")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(8)
            .into();
        };
        let order = a.queue.order();
        let pos = a.queue.pos();
        let played = pos;
        let upcoming = order.len().saturating_sub(pos + 1);
        let total_ms: u64 = order[pos.min(order.len())..]
            .iter()
            .filter_map(|&i| a.tracks.get(i))
            .map(|t| t.duration_ms)
            .sum();

        let mut list = Column::new().spacing(2);
        for (p, &i) in order.iter().enumerate().skip(pos) {
            let Some(t) = a.tracks.get(i) else { continue };
            if p == pos + 1 {
                list = list.push(
                    container(
                        text("A continuación")
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                    )
                    .padding([10, 8]),
                );
            }
            list = list.push(self.queue_row(p, p == pos, t));
        }
        if a.endless || self.extending {
            list = list.push(
                container(
                    text(if self.extending {
                        "Buscando canciones parecidas…"
                    } else {
                        "Cuando se acerque el final se agregan canciones parecidas."
                    })
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                )
                .padding([10, 8]),
            );
        }

        let mut info = format!("{upcoming} por sonar · {}", format_duration(total_ms));
        if played > 0 {
            info.push_str(&format!(" · {played} ya sonaron"));
        }
        column![
            column![
                text("Cola").size(theme::SIZE_L),
                text(info).size(theme::SIZE_S).style(theme::secondary_text),
                text("Arrastrá una canción para cambiarle el lugar. Clic para escucharla.")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(4),
            scrollable(list.padding(iced::Padding::ZERO.right(16))).height(Length::Fill),
        ]
        .spacing(16)
        .into()
    }

    fn queue_row<'a>(&'a self, pos: usize, current: bool, t: &'a Track) -> Element<'a, Message> {
        let target = self.drag.is_some_and(|(from, to)| to == pos && from != pos);
        let lifted = self.drag.is_some_and(|(from, _)| from == pos);
        let status: Element<'_, Message> = match self.resolution(t) {
            Some(TrackRes::Resolving) => text("buscando…")
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into(),
            Some(TrackRes::Failed(_)) => text("sin match")
                .size(theme::SIZE_S)
                .style(theme::error_text)
                .into(),
            _ => Space::new().into(),
        };
        let lead: Element<'_, Message> = if current {
            icons::glyph(Kind::Volume, theme::colors().accent, 16.0)
        } else {
            icons::glyph(Kind::Grip, theme::colors().text_secondary, 16.0)
        };
        let remove: Element<'_, Message> = if current {
            Space::new().width(28).into()
        } else {
            button(icons::glyph(
                Kind::Close,
                theme::colors().text_secondary,
                14.0,
            ))
            .on_press(Message::QueueRemove(pos))
            .padding(7)
            .style(theme::icon_button)
            .into()
        };
        let cover: Element<'_, Message> = match self
            .video_of_track(t)
            .and_then(|v| self.covers.get(v))
            .filter(|_| self.settings.covers_in_list)
        {
            Some(h) => iced::widget::image(h.clone())
                .width(36)
                .height(36)
                .border_radius(4.0)
                .into(),
            None => Space::new().width(0).into(),
        };
        let content = row![
            container(lead).width(28),
            cover,
            column![
                text(&t.title)
                    .size(theme::SIZE_M)
                    .wrapping(text::Wrapping::None)
                    .style(if current {
                        theme::accent_text
                    } else {
                        theme::plain_text
                    }),
                text(t.artists.join(", "))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(2)
            .width(Length::Fill)
            .clip(true),
            status,
            text(format_duration(t.duration_ms))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            remove,
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let row = container(content)
            .padding([6, 8])
            .width(Length::Fill)
            .style(theme::queue_row(current, target, lifted));
        mouse_area(row)
            .on_press(Message::DragStart(pos))
            .on_enter(Message::DragOver(pos))
            .interaction(if self.drag.is_some() {
                mouse::Interaction::Grabbing
            } else {
                mouse::Interaction::Pointer
            })
            .into()
    }
}
