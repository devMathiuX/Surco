//! Cola de reproducción: ver lo que sigue, agregar, quitar, saltar y
//! reordenar arrastrando.
//!
//! El arrastre: apretar sobre una fila la toma (`DragStart`), pasar por
//! encima de otra la marca como destino (`DragOver`) y soltar el botón en
//! cualquier lado (evento global) la mueve (`DragEnd`). Apretar y soltar
//! sobre la misma fila es un clic: salta a esa canción.

use iced::widget::{
    button, column, container, mouse_area, responsive, row, scrollable, text, text_input, Column,
    Space,
};
use iced::{mouse, Alignment, Element, Length, Task};
use surco_core::{format_duration, Track};

use crate::icons::{self, Kind};
use crate::playback::{Active, TrackRes};
use crate::{theme, Drag, DragList, Message, Notice, Surco};

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
                trf!(
                    "«{title}» suena después de la actual.",
                    "“{title}” plays after the current one."
                )
            } else {
                trf!(
                    "«{title}» agregada a la cola.",
                    "“{title}” added to the queue."
                )
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

    /// Saca todo lo que viene después de la que suena.
    pub(crate) fn queue_clear(&mut self) {
        if let Some(a) = &mut self.active {
            let pos = a.queue.pos();
            while a.queue.order().len() > pos + 1 {
                if a.queue.remove_at(pos + 1).is_none() {
                    break;
                }
            }
        }
        self.update_preload();
    }

    /// Guarda la cola (de la que suena en adelante) como playlist propia.
    pub(crate) fn queue_save(&mut self) -> Task<Message> {
        let Some(a) = &self.active else {
            return Task::none();
        };
        let tracks: Vec<Track> = a.queue.order()[a.queue.pos().min(a.queue.order().len())..]
            .iter()
            .filter_map(|&i| a.tracks.get(i).cloned())
            .collect();
        if tracks.is_empty() {
            return Task::none();
        }
        self.new_playlist_with(tracks)
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
        let Some(Drag { list, from, to }) = self.drag.take() else {
            return Task::none();
        };
        if list == DragList::Sidebar {
            // Sin moverse es un clic: abrir la playlist.
            if from == to {
                return self.handle(Message::SelectPlaylist(Some(from)));
            }
            self.move_playlist(from, to);
            return Task::none();
        }
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
                text(tr!("Cola de reproducción", "Play queue")).size(theme::SIZE_L + 6),
                text(tr!("No hay nada sonando. Elegí una canción de una playlist o del buscador; con el + de cada canción la agregás a la cola.", "Nothing is playing. Pick a song from a playlist or from search; the + on each song adds it to the queue."))
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

        let mut info = trf!(
            "{upcoming} por sonar · {}",
            "{upcoming} up next · {}",
            format_duration(total_ms)
        );
        if played > 0 {
            info.push_str(&trf!(
                " · {played} ya sonaron",
                " · {played} already played"
            ));
        }
        let c = theme::colors();
        let action = |kind: Kind, label: &'static str, msg: Option<Message>, on: bool| {
            button(
                row![
                    icons::icon(kind, c.text_secondary, 14.0),
                    text(label).size(theme::SIZE_S)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .on_press_maybe(msg)
            .padding([7, 12])
            .style(theme::pill(on))
        };
        let actions = row![
            action(
                Kind::Close,
                tr!("Vaciar cola", "Clear queue"),
                (upcoming > 0).then_some(Message::QueueClear),
                false
            ),
            action(
                Kind::Add,
                tr!("Guardar como playlist", "Save as playlist"),
                Some(Message::QueueSave),
                false
            ),
            action(
                Kind::Shuffle,
                tr!("Mezclar", "Shuffle"),
                Some(Message::ToggleShuffle),
                a.queue.shuffle()
            ),
            container(
                text_input(
                    tr!("Buscar en la cola…", "Search the queue…"),
                    &self.queue_filter
                )
                .on_input(Message::QueueFilter)
                .padding(7)
                .size(theme::SIZE_S)
                .style(theme::input)
            )
            .width(200),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .wrap();
        let header = row![
            column![
                text(tr!("Cola de reproducción", "Play queue")).size(theme::SIZE_L + 6),
                text(info).size(theme::SIZE_S).style(theme::secondary_text),
            ]
            .spacing(4)
            .width(Length::Shrink),
            Space::new().width(Length::Fill),
            actions,
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let tip = container(
            row![
                icons::glyph(Kind::Grip, c.text_secondary, 13.0),
                text(tr!(
                    "Arrastrá una canción para cambiarle el lugar. Clic para escucharla.",
                    "Drag a song to move it. Click to play it."
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([5, 10])
        .style(theme::inset);

        // Las que ya sonaron, la última primero.
        let history: Vec<usize> = (0..pos.min(order.len())).rev().take(30).collect();
        let body = responsive(move |size| {
            let main = self.queue_main();
            if size.width < 900.0 {
                return main;
            }
            let rows = history.iter().filter_map(|&p| {
                let t = order.get(p).and_then(|&i| a.tracks.get(i))?;
                Some(
                    button(
                        row![
                            crate::home::cover(self, self.video_of_track(t), 36.0),
                            column![
                                crate::marquee::marquee(
                                    text(&t.title)
                                        .size(theme::SIZE_S + 1)
                                        .wrapping(text::Wrapping::None),
                                    false,
                                ),
                                crate::marquee::marquee(
                                    text(t.artists.join(", "))
                                        .size(theme::SIZE_S)
                                        .style(theme::secondary_text)
                                        .wrapping(text::Wrapping::None),
                                    false,
                                ),
                            ]
                            .spacing(2)
                            .width(Length::Fill)
                            .clip(true),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::QueueJump(p))
                    .padding([5, 8])
                    .width(Length::Fill)
                    .style(theme::song_row(false))
                    .into(),
                )
            });
            let panel = container(
                column![
                    text(tr!("Ya sonaron", "Already played")).size(theme::SIZE_M + 1),
                    if history.is_empty() {
                        Element::from(
                            text(tr!("Todavía nada.", "Nothing yet."))
                                .size(theme::SIZE_S)
                                .style(theme::secondary_text),
                        )
                    } else {
                        crate::smooth::smooth(scrollable(Column::with_children(rows).spacing(2)))
                    },
                ]
                .spacing(12),
            )
            .padding(16)
            .width(300)
            .style(theme::card);
            row![main, column![panel]].spacing(20).into()
        });
        column![header, tip, body].spacing(16).into()
    }

    /// Lo que suena y lo que sigue (filtrado por "Buscar en la cola").
    fn queue_main(&self) -> Element<'_, Message> {
        let Some(a) = &self.active else {
            return Space::new().into();
        };
        let order = a.queue.order();
        let pos = a.queue.pos();
        let upcoming = order.len().saturating_sub(pos + 1);
        let filter = crate::settings_view::fold(self.queue_filter.trim());
        let keep = |t: &Track| {
            filter.is_empty()
                || crate::settings_view::fold(&t.title).contains(&filter)
                || crate::settings_view::fold(&t.artists.join(" ")).contains(&filter)
        };
        let mut list = Column::new().spacing(6);
        let mut shown = 0;
        for (p, &i) in order.iter().enumerate().skip(pos + 1) {
            let Some(t) = a.tracks.get(i) else { continue };
            if !keep(t) {
                continue;
            }
            list = list.push(self.queue_row(p, false, t));
            shown += 1;
        }
        if shown == 0 && upcoming > 0 {
            list = list.push(
                text(tr!("Nada coincide con lo buscado.", "Nothing matches."))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            );
        }
        if self.extending {
            list = list.push(crate::skeleton::song_rows(3, self.pulse));
        } else if a.endless {
            list = list.push(
                container(
                    text(tr!(
                        "Cuando se acerque el final se agregan canciones parecidas.",
                        "Similar songs get added as the end gets close."
                    ))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                )
                .padding([10, 8]),
            );
        }

        let now = order
            .get(pos)
            .and_then(|&i| a.tracks.get(i))
            .map(|t| self.queue_now(t));
        column![]
            .push(now)
            .push(
                text(tr!("A continuación", "Up next"))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            )
            .push(crate::smooth::smooth(
                scrollable(list.padding(iced::Padding::ZERO.right(16))).height(Length::Fill),
            ))
            .spacing(12)
            .into()
    }

    /// Tarjeta de lo que suena: portada grande, título y "Siguiente".
    fn queue_now<'a>(&'a self, t: &'a Track) -> Element<'a, Message> {
        let c = theme::colors();
        container(
            row![
                crate::home::big_cover(self, self.video_of_track(t), 96.0),
                column![
                    text(tr!("Sonando ahora", "Now playing"))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                    crate::marquee::marquee(
                        text(&t.title)
                            .size(theme::SIZE_L)
                            .wrapping(text::Wrapping::None),
                        !theme::minimal(),
                    ),
                    crate::marquee::marquee(
                        text(t.artists.join(", "))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .wrapping(text::Wrapping::None),
                        !theme::minimal(),
                    ),
                    row![
                        button(
                            row![
                                icons::icon(Kind::Next, c.text, 13.0),
                                text(tr!("Siguiente", "Next")).size(theme::SIZE_S)
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        )
                        .on_press(Message::Skip(1))
                        .padding([6, 12])
                        .style(theme::pill(false)),
                        crate::search::add_button(t, None),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                ]
                .spacing(4)
                .width(Length::Fill)
                .clip(true),
                icons::waves(),
            ]
            .spacing(18)
            .align_y(Alignment::Center),
        )
        .padding(16)
        .width(Length::Fill)
        .style(theme::highlight_card)
        .into()
    }

    fn queue_row<'a>(&'a self, pos: usize, current: bool, t: &'a Track) -> Element<'a, Message> {
        let drag = self.drag.filter(|d| d.list == DragList::Queue);
        let target = drag.is_some_and(|d| d.to == pos && d.from != pos);
        let lifted = drag.is_some_and(|d| d.from == pos);
        let status: Element<'_, Message> = match self.resolution(t) {
            Some(TrackRes::Resolving) => text(tr!("buscando…", "searching…"))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into(),
            Some(TrackRes::Failed(_)) => text(tr!("sin match", "no match"))
                .size(theme::SIZE_S)
                .style(theme::error_text)
                .into(),
            _ => Space::new().into(),
        };
        let lead: Element<'_, Message> = if current {
            Space::new().width(16).into()
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
            .filter(|_| self.settings.covers_in_list && !theme::minimal())
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
                crate::marquee::marquee(
                    text(&t.title)
                        .size(theme::SIZE_M)
                        .wrapping(text::Wrapping::None)
                        .style(if current {
                            theme::accent_text
                        } else {
                            theme::plain_text
                        }),
                    self.is_hovered(t),
                ),
                crate::marquee::marquee(
                    text(t.artists.join(", "))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .wrapping(text::Wrapping::None),
                    self.is_hovered(t),
                ),
            ]
            .spacing(2)
            .width(Length::Fill)
            .clip(true),
            status,
            icons::waves_if(current),
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
            .on_press(Message::DragStart(DragList::Queue, pos))
            // Soltar encima de una fila (el evento global puede no estar
            // suscripto todavía si el clic fue muy rápido).
            .on_release(Message::DragEnd)
            .on_enter(Message::DragOver(DragList::Queue, pos))
            .interaction(if self.drag.is_some() {
                mouse::Interaction::Grabbing
            } else {
                mouse::Interaction::Pointer
            })
            .into()
    }
}
