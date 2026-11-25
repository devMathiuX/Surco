//! Vistas: barra lateral, importar, lista de la playlist (con búsqueda y
//! recomendadas), ajustes y selector de video.

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, stack, text, text_input,
    toggler, tooltip, Column, Id, Space,
};
use iced::{mouse, Alignment, Element, Length};
use surco_core::{format_duration, Playlist, Track};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::playlists::{OWN_SOURCE, RENAME_INPUT};
use crate::settings::{Change, Precache};
use crate::{skeleton, theme, DragList, Message, Screen, Surco, FILTER_INPUT, IMPORT_INPUT};

impl Surco {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let main: Element<'_, Message> = match self.screen {
            Screen::Settings => self.view_settings(),
            Screen::Queue => self.view_queue(),
            Screen::Search => self.view_search(),
            Screen::Import => self.view_import(),
            Screen::Home => self.view_home(),
            Screen::Library if self.radio_waiting => column![
                skeleton::header(self.pulse),
                skeleton::song_rows(10, self.pulse)
            ]
            .spacing(24)
            .into(),
            Screen::Library => match self.selected.and_then(|i| self.playlists.get(i)) {
                Some((playlist, source)) => {
                    self.view_playlist(self.selected.unwrap_or(0), playlist, source)
                }
                None => self.view_home(),
            },
        };
        let main = container(main)
            .padding([24, 32])
            .width(Length::Fill)
            .height(Length::Fill);

        let app: Element<'_, Message> = column![
            row![self.view_sidebar(), main].height(Length::Fill),
            self.view_player_bar()
        ]
        .into();
        if let Some(menu) = &self.song_menu {
            return stack![app, self.view_song_menu(menu)].into();
        }
        if let Some(i) = self.playlist_menu {
            return stack![app, self.view_playlist_menu(i)].into();
        }
        match self.view_volume_popup() {
            Some(popup) => stack![app, popup].into(),
            None => app,
        }
    }

    pub(crate) fn view_sidebar(&self) -> Element<'_, Message> {
        let drag = self.drag.filter(|d| d.list == DragList::Sidebar);
        let mut list = Column::new().spacing(2);
        for (i, (p, _)) in self.playlists.iter().enumerate() {
            let selected = self.screen == Screen::Library && self.selected == Some(i);
            let target = drag.is_some_and(|d| d.to == i && d.from != i);
            let lifted = drag.is_some_and(|d| d.from == i);
            let hovered = self.sidebar_hover == Some(i);
            let playing = self.active.as_ref().is_some_and(|a| a.origin == Some(i));
            let label = text(&p.name)
                .size(theme::SIZE_S)
                .wrapping(text::Wrapping::None)
                .width(Length::Fill);
            let content: Element<'_, Message> = if playing {
                row![
                    label,
                    icons::glyph(Kind::Volume, theme::colors().accent, 13.0)
                ]
                .align_y(Alignment::Center)
                .into()
            } else {
                label.into()
            };
            list = list.push(
                mouse_area(
                    container(content)
                        .padding([6, 8])
                        .width(Length::Fill)
                        .clip(true)
                        .style(theme::drag_row(selected, hovered, target, lifted)),
                )
                .on_press(Message::DragStart(DragList::Sidebar, i))
                // Soltar encima de una fila (el evento global puede no estar
                // suscripto todavía si el clic fue muy rápido).
                .on_release(Message::DragEnd)
                .on_right_press(Message::PlaylistMenu(Some(i)))
                .on_enter(Message::SidebarHover(Some(i)))
                .on_exit(Message::SidebarHover(None))
                .on_move(move |_| Message::DragOver(DragList::Sidebar, i))
                .interaction(if drag.is_some() {
                    mouse::Interaction::Grabbing
                } else {
                    mouse::Interaction::Pointer
                }),
            );
        }
        let nav = |kind: Kind, label: &'static str, screen: Screen, msg: Message| {
            let on = self.screen == screen;
            button(
                row![
                    icons::icon(
                        kind,
                        if on {
                            theme::colors().accent
                        } else {
                            theme::colors().text_secondary
                        },
                        16.0
                    ),
                    text(label).size(theme::SIZE_S),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .on_press(msg)
            .width(Length::Fill)
            .padding([6, 8])
            .style(theme::list_button(on))
        };
        let queue_len = self
            .active
            .as_ref()
            .map(|a| a.queue.len().saturating_sub(a.queue.pos() + 1))
            .unwrap_or(0);
        let nav = column![
            nav(Kind::Home, "Inicio", Screen::Home, Message::GoHome),
            nav(
                Kind::Search,
                "Buscar",
                Screen::Search,
                Message::Search(crate::search::SearchMsg::Open)
            ),
            nav(
                Kind::Queue,
                if queue_len > 0 {
                    "Cola"
                } else {
                    "Cola (vacía)"
                },
                Screen::Queue,
                Message::ToggleQueue
            ),
            nav(
                Kind::Import,
                "Importar",
                Screen::Import,
                Message::OpenImport
            ),
        ]
        .spacing(2);

        let gear = tooltip(
            button(icons::icon(
                Kind::Settings,
                if self.screen == Screen::Settings {
                    theme::colors().accent
                } else {
                    theme::colors().text_secondary
                },
                18.0,
            ))
            .on_press(Message::ToggleSettings)
            .padding(4)
            .style(theme::icon_button),
            container(text("Ajustes").size(theme::SIZE_S))
                .padding(6)
                .style(theme::card),
            tooltip::Position::Bottom,
        );
        let logo: Element<'_, Message> = match &self.logo {
            Some(h) => image(h.clone()).width(26).height(26).into(),
            None => Space::new().into(),
        };
        let brand = button(
            row![logo, text("Surco").size(theme::SIZE_L)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .on_press(Message::GoHome)
        .padding(0)
        .style(theme::list_button(false));
        let library_header = row![
            text("Tu biblioteca")
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .width(Length::Fill),
            tooltip(
                button(icons::icon(Kind::Add, theme::colors().text_secondary, 14.0))
                    .on_press(Message::NewPlaylist)
                    .padding(4)
                    .style(theme::icon_button),
                container(text("Nueva playlist").size(theme::SIZE_S))
                    .padding(6)
                    .style(theme::card),
                tooltip::Position::Bottom,
            ),
        ]
        .align_y(Alignment::Center)
        .padding(iced::Padding::ZERO.left(8));
        let mut col = column![
            row![brand, Space::new().width(Length::Fill), gear].align_y(Alignment::Center),
            nav,
            column![library_header, scrollable(list).height(Length::Fill)].spacing(6),
        ]
        .spacing(16)
        .height(Length::Fill);
        if let Some(n) = &self.notice {
            col = col.push(text(&n.text).size(theme::SIZE_S).style(if n.error {
                theme::error_text
            } else {
                theme::secondary_text
            }));
        }
        container(col)
            .padding(16)
            .width(220)
            .height(Length::Fill)
            .style(theme::surface)
            .into()
    }

    /// Pantalla "Importar": pegar un link o elegir un CSV.
    pub(crate) fn view_import(&self) -> Element<'_, Message> {
        let input = text_input("Pegá el link de la playlist", &self.import_input)
            .id(Id::new(IMPORT_INPUT))
            .on_input(Message::ImportInputChanged)
            .on_submit(Message::Import)
            .padding(10)
            .size(theme::SIZE_M)
            .style(theme::input);
        let import = button(text("Importar").size(theme::SIZE_S))
            .on_press_maybe((!self.importing).then_some(Message::Import))
            .padding([10, 16])
            .style(theme::primary_button);
        let pick_csv = button(text("Elegir CSV…").size(theme::SIZE_S))
            .on_press_maybe((!self.importing).then_some(Message::PickCsv))
            .padding([6, 12])
            .style(theme::ghost_button);
        let hint = |t: &'static str| text(t).size(theme::SIZE_S).style(theme::secondary_text);
        let mut col = column![
            column![
                text("Importar").size(theme::SIZE_L),
                hint("Traé tus playlists de Spotify, YouTube o YouTube Music pegando el link (Ctrl+L)."),
            ]
            .spacing(4),
            row![input, import].spacing(8).align_y(Alignment::Center),
            column![
                hint("• Spotify: Compartir > Copiar enlace a la playlist."),
                hint("• YouTube y YouTube Music: el link de la playlist, o un video que esté dentro de una."),
                hint("• Un archivo CSV, por ejemplo exportado desde exportify.net."),
            ]
            .spacing(4),
            pick_csv,
        ]
        .spacing(16)
        .max_width(760);
        if let Some(n) = self.notice.as_ref().filter(|n| n.error) {
            col = col.push(text(&n.text).size(theme::SIZE_S).style(theme::error_text));
        }
        if self.importing {
            col = col.push(
                column![
                    text("Importando…")
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                    skeleton::song_rows(6, self.pulse),
                ]
                .spacing(10),
            );
        }
        col.into()
    }

    pub(crate) fn view_playlist<'a>(
        &'a self,
        index: usize,
        p: &'a Playlist,
        source: &str,
    ) -> Element<'a, Message> {
        let total_ms: u64 = p.tracks.iter().map(|t| t.duration_ms).sum();
        let origin = match source {
            "radio" => "canciones parecidas, del Mix de YouTube".to_owned(),
            OWN_SOURCE => "playlist tuya".to_owned(),
            _ => format!("importada desde {source}"),
        };
        let mut info = format!(
            "{} · {} · {origin}",
            songs(p.tracks.len()),
            format_duration(total_ms)
        );
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.origin == Some(index))
            && self.precache.target > 0
        {
            info.push_str(&format!(
                " · precargadas {} de {}{}",
                self.precache.ready,
                self.precache.target,
                if self.precache.running.is_some() {
                    " (bajando…)"
                } else {
                    ""
                }
            ));
        }
        let name: Element<'_, Message> = match &self.renaming {
            Some((r, value)) if *r == index => row![
                text_input("Nombre de la playlist", value)
                    .id(Id::new(RENAME_INPUT))
                    .on_input(Message::RenameInput)
                    .on_submit(Message::RenameCommit)
                    .padding(6)
                    .size(theme::SIZE_L)
                    .style(theme::input),
                button(text("Guardar").size(theme::SIZE_S))
                    .on_press(Message::RenameCommit)
                    .padding([6, 12])
                    .style(theme::primary_button),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            _ => row![
                text(&p.name)
                    .size(theme::SIZE_L)
                    .wrapping(text::Wrapping::None),
                tooltip(
                    button(icons::icon(
                        Kind::Edit,
                        theme::colors().text_secondary,
                        16.0
                    ))
                    .on_press(Message::StartRename(index))
                    .padding(6)
                    .style(theme::icon_button),
                    container(text("Renombrar").size(theme::SIZE_S))
                        .padding(6)
                        .style(theme::card),
                    tooltip::Position::Bottom,
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        };
        let small = |label: String, msg: Message| {
            button(text(label).size(theme::SIZE_S))
                .on_press(msg)
                .padding([4, 10])
                .style(theme::ghost_button)
        };
        let mut actions = row![].spacing(8);
        if let Some((files, bytes)) = self.downloads.get(&p.id) {
            actions = actions.push(small(
                format!(
                    "Borrar descargas ({}, {})",
                    songs(*files),
                    format_bytes(*bytes)
                ),
                Message::DeleteDownloads(index),
            ));
        }
        actions = actions.push(small("Quitar".into(), Message::RemovePlaylist(index)));
        let header = column![
            row![container(name).width(Length::Fill), actions]
                .spacing(16)
                .align_y(Alignment::Center),
            text(info)
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(4);

        let query = self.filter.as_deref().unwrap_or("");
        let shown: Vec<(usize, &Track)> = p
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| matches_filter(t, query))
            .collect();
        // Solo se arman las filas que se ven (más un margen): con playlists de
        // miles de canciones, armarlas todas pasa los 80 MB de RAM. Las demás
        // se reemplazan por espacio vacío del mismo alto.
        let (first, last) = visible_rows(self.list_view, shown.len());
        let above = first as f32 * ROW_HEIGHT;
        let below = (shown.len() - last) as f32 * ROW_HEIGHT;
        let rows = Column::with_children(shown[first..last].iter().map(|&(i, t)| {
            let playing = self.now_in_playlist() == Some((index, i));
            let badge: Element<'_, Message> = match self.resolution(t) {
                Some(TrackRes::Resolving) => text("buscando…")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into(),
                Some(TrackRes::Done(m)) if m.is_doubtful() => {
                    button(text("dudosa").size(theme::SIZE_S))
                        .on_press(Message::OpenChooser(Some((index, i))))
                        .padding([2, 8])
                        .style(theme::ghost_button)
                        .into()
                }
                Some(TrackRes::Failed(e)) => tooltip(
                    text("sin match")
                        .size(theme::SIZE_S)
                        .style(theme::error_text),
                    container(text(e).size(theme::SIZE_S))
                        .padding(8)
                        .max_width(360)
                        .style(theme::surface),
                    tooltip::Position::Left,
                )
                .into(),
                _ => Space::new().into(),
            };
            let number = container(if playing {
                icons::glyph(Kind::Volume, theme::colors().accent, 16.0)
            } else {
                text(i + 1)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into()
            })
            .width(40);
            let cover: Element<'_, Message> = if self.settings.covers_in_list {
                match self.video_of((index, i)).and_then(|v| self.covers.get(v)) {
                    Some(h) => image(h.clone())
                        .width(LIST_COVER)
                        .height(LIST_COVER)
                        .border_radius(4.0)
                        .into(),
                    // Buscándose o bajando la portada: late.
                    None if matches!(self.resolution(t), Some(TrackRes::Resolving))
                        || self.video_of((index, i)).is_some() =>
                    {
                        skeleton::block(LIST_COVER, LIST_COVER, self.pulse)
                    }
                    None => container(Space::new())
                        .width(LIST_COVER)
                        .height(LIST_COVER)
                        .style(theme::cover_placeholder)
                        .into(),
                }
            } else {
                Space::new().width(0).into()
            };
            let row = button(
                row![
                    number,
                    cover,
                    column![
                        text(&t.title)
                            .size(theme::SIZE_M)
                            .wrapping(text::Wrapping::None),
                        text(t.artists.join(", "))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(2)
                    .width(Length::Fill)
                    .clip(true),
                    badge,
                    crate::search::add_button(t, Some((index, i))),
                    text(format_duration(t.duration_ms))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .on_press(Message::PlayTrack(index, i))
            .padding([6, 8])
            .width(Length::Fill)
            .height(ROW_HEIGHT)
            .style(theme::list_button(playing));
            crate::search::with_menu(row, t, Some((index, i)))
        }));
        let rows = column![
            Space::new().height(above),
            rows,
            Space::new().height(below),
            self.view_recs(index, p)
        ]
        .padding(iced::Padding::ZERO.right(16).bottom(16));

        let mut col = column![header].spacing(20);
        // Buscador dentro de la playlist, siempre a mano (Ctrl+F lo enfoca).
        if !p.tracks.is_empty() {
            let mut search = row![
                icons::icon(Kind::Search, theme::colors().text_secondary, 18.0),
                text_input("Buscar en la playlist", query)
                    .id(Id::new(FILTER_INPUT))
                    .on_input(Message::FilterChanged)
                    .padding(8)
                    .size(theme::SIZE_S)
                    .style(theme::input),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            if !query.is_empty() {
                search = search.push(
                    button(icons::icon(
                        Kind::Close,
                        theme::colors().text_secondary,
                        14.0,
                    ))
                    .on_press(Message::CloseFilter)
                    .padding(8)
                    .style(theme::icon_button),
                );
            }
            col = col.push(search);
        }
        if !query.trim().is_empty() && !p.tracks.iter().any(|t| matches_filter(t, query)) {
            col = col.push(
                text(format!("Nada coincide con «{}».", query.trim()))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            );
        }
        if p.tracks.is_empty() {
            col = col.push(
                column![
                    text("Está vacía. Buscá canciones y agregalas con el + de cada una (o con clic derecho), o sumá de las recomendadas.")
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                    // Por si en realidad querían pegar el link de una playlist.
                    row![
                        text("¿Querías traer una playlist de Spotify o YouTube?")
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        button(text("Importar una playlist").size(theme::SIZE_S))
                            .on_press(Message::OpenImport)
                            .padding([4, 10])
                            .style(theme::ghost_button),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                ]
                .spacing(10),
            );
        }
        if let Some(chooser) = self.view_chooser(index) {
            col = col.push(chooser);
        }
        col.push(
            scrollable(rows)
                .id(Id::new(LIST_SCROLL))
                .on_scroll(|v| Message::ListScrolled(v.absolute_offset().y, v.bounds().height))
                .height(Length::Fill),
        )
        .into()
    }

    /// Pantalla de ajustes: tema, portadas, precarga y radio automática.
    pub(crate) fn view_settings(&self) -> Element<'_, Message> {
        let section = |title: &'static str, hint: &'static str| {
            column![
                text(title).size(theme::SIZE_M),
                text(hint).size(theme::SIZE_S).style(theme::secondary_text),
            ]
            .spacing(2)
        };

        // Temas: botón con dos muestras de color (fondo y acento).
        let themes = row(theme::THEMES.iter().enumerate().map(|(i, t)| {
            let sw = |c| {
                container(Space::new())
                    .width(14)
                    .height(14)
                    .style(theme::swatch(c))
            };
            button(
                row![
                    sw(t.background),
                    sw(t.accent),
                    text(t.name).size(theme::SIZE_S)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Setting(Change::Theme(i)))
            .padding([6, 10])
            .style(theme::choice_button(self.settings.theme == i))
            .into()
        }))
        .spacing(8)
        .wrap();

        let precache = row(Precache::ALL.iter().map(|&p| {
            button(text(p.label()).size(theme::SIZE_S))
                .on_press(Message::Setting(Change::Precache(p)))
                .padding([6, 10])
                .style(theme::choice_button(self.settings.precache == p))
                .into()
        }))
        .spacing(8);
        let usage = self
            .provider
            .as_ref()
            .and_then(|p| p.audio_cache())
            .map(|c| c.usage())
            .unwrap_or_default();
        let mut per_playlist = Column::new().spacing(4);
        for (i, (p, _)) in self.playlists.iter().enumerate() {
            let Some((files, bytes)) = self.downloads.get(&p.id) else {
                continue;
            };
            per_playlist = per_playlist.push(
                row![
                    text(&p.name)
                        .size(theme::SIZE_S)
                        .wrapping(text::Wrapping::None)
                        .width(Length::Fill),
                    text(format!("{}, {}", songs(*files), format_bytes(*bytes)))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                    button(text("Borrar").size(theme::SIZE_S))
                        .on_press(Message::DeleteDownloads(i))
                        .padding([4, 10])
                        .style(theme::ghost_button),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            );
        }
        let cache_row = row![
            text(format!(
                "En disco: {} canciones, {}",
                usage.files,
                format_bytes(usage.bytes)
            ))
            .size(theme::SIZE_S)
            .style(theme::secondary_text),
            button(text("Vaciar").size(theme::SIZE_S))
                .on_press_maybe((usage.files > 0).then_some(Message::ClearAudioCache))
                .padding([4, 10])
                .style(theme::ghost_button),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let downloads: Element<'_, Message> = if self.downloads.is_empty() {
            Space::new().into()
        } else {
            column![
                text("Descargas por playlist (una canción puede estar en varias)")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                container(per_playlist).max_width(560),
            ]
            .spacing(8)
            .into()
        };

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
            text("Ajustes").size(theme::SIZE_L),
            column![
                section("Tema", "Cómo se ve Surco."),
                themes,
            ]
            .spacing(10),
            column![
                section(
                    "Portadas en la lista",
                    "Muestra la portada al lado de cada canción ya encontrada en YouTube."
                ),
                switch(
                    "Mostrar portadas",
                    self.settings.covers_in_list,
                    Change::CoversInList
                ),
            ]
            .spacing(10),
            column![
                section(
                    "Precargar canciones",
                    "Baja el audio antes de que suene, así no hay que esperar a que cargue. La siguiente siempre se prepara en memoria para que no haya cortes."
                ),
                precache,
                cache_row,
                downloads,
            ]
            .spacing(10),
            column![
                section(
                    "Canciones parecidas",
                    "Con el botón de radio de la barra de abajo armás una lista de canciones parecidas a la que suena."
                ),
                switch(
                    "Al terminar, seguir con canciones parecidas",
                    self.settings.autoplay,
                    Change::Autoplay
                ),
            ]
            .spacing(10),
            self.view_spotify_keys(),
            text(match &self.ytdlp {
                crate::YtDlpState::Found(p) => format!("yt-dlp: {}", p.display()),
                _ => "yt-dlp: no encontrado".to_owned(),
            })
            .size(theme::SIZE_S)
            .style(theme::secondary_text),
        ]
        .spacing(28)
        .max_width(760)
        .padding(iced::Padding::ZERO.right(16));
        scrollable(body).height(Length::Fill).into()
    }

    /// Claves de Spotify: cuáles se usan y cargar las propias.
    fn view_spotify_keys(&self) -> Element<'_, Message> {
        use crate::spotify_keys::{msg, KeySource, KeysMsg};
        let status = match self.spotify_source {
            KeySource::Own => "Usando tus claves.",
            KeySource::Config => "Usando las claves de config.toml.",
            KeySource::Builtin => "Usando las claves incluidas en Surco.",
            KeySource::None => "Sin claves propias (no hacen falta).",
        };
        let form = row![
            text_input("Client ID", &self.keys_form.id)
                .on_input(|s| msg(KeysMsg::Id(s)))
                .padding(8)
                .size(theme::SIZE_S)
                .style(theme::input),
            text_input("Client Secret", &self.keys_form.secret)
                .on_input(|s| msg(KeysMsg::Secret(s)))
                .on_submit(msg(KeysMsg::Save))
                .secure(true)
                .padding(8)
                .size(theme::SIZE_S)
                .style(theme::input),
            button(text("Guardar").size(theme::SIZE_S))
                .on_press_maybe(
                    (!self.keys_form.id.trim().is_empty()
                        && !self.keys_form.secret.trim().is_empty())
                    .then(|| msg(KeysMsg::Save)),
                )
                .padding([6, 12])
                .style(theme::primary_button),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let mut col = column![
            column![
                text("Spotify").size(theme::SIZE_M),
                text("Surco lee las playlists enteras igual que la web de Spotify, sin cuenta, Premium ni claves. Las claves de la Web API son opcionales: solo se usan si eso falla (y Spotify pide Premium para crearlas en developer.spotify.com).")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(2),
            text(status).size(theme::SIZE_S),
            form,
        ]
        .spacing(10);
        if self.spotify_source == KeySource::Own {
            col = col.push(
                button(text("Borrar mis claves").size(theme::SIZE_S))
                    .on_press(msg(KeysMsg::Clear))
                    .padding([4, 10])
                    .style(theme::ghost_button),
            );
        }
        col.into()
    }

    /// Selector de video para una canción (los candidatos con su puntaje).
    pub(crate) fn view_chooser(&self, playlist: usize) -> Option<Element<'_, Message>> {
        let at = self.choosing.filter(|(p, _)| *p == playlist)?;
        let track = self.track(at)?;
        let Some(TrackRes::Done(m)) = self.resolution(track) else {
            return None;
        };
        let mut list = Column::new().spacing(2);
        for c in &m.candidates {
            let chosen = c.candidate.id == m.youtube_id;
            let dur = c
                .candidate
                .duration_secs
                .map(|d| format_duration((d * 1000.0) as u64))
                .unwrap_or_else(|| "--:--".into());
            list = list.push(
                button(
                    row![
                        column![
                            text(&c.candidate.title)
                                .size(theme::SIZE_S)
                                .wrapping(text::Wrapping::None),
                            text(
                                c.candidate
                                    .channel
                                    .as_deref()
                                    .unwrap_or("canal desconocido")
                            )
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        ]
                        .width(Length::Fill),
                        text(dur).size(theme::SIZE_S).style(theme::secondary_text),
                        text(format!("{:>3}", c.score))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .width(32),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::ChooseVideo(c.candidate.id.clone()))
                .padding([4, 8])
                .width(Length::Fill)
                .style(theme::list_button(chosen)),
            );
        }
        let header = row![
            text(format!(
                "Elegí el video para «{}» ({} canciones de YouTube; el número es el puntaje)",
                track.title,
                m.candidates.len()
            ))
            .size(theme::SIZE_S)
            .width(Length::Fill),
            button(text("Cerrar").size(theme::SIZE_S))
                .on_press(Message::OpenChooser(None))
                .padding([2, 8])
                .style(theme::ghost_button),
        ]
        .align_y(Alignment::Center);
        Some(
            container(column![header, scrollable(list).height(Length::Shrink)].spacing(8))
                .padding(12)
                .max_height(260)
                .style(theme::surface)
                .into(),
        )
    }
}

/// "1 canción", "12 canciones".
pub(crate) fn songs(n: usize) -> String {
    if n == 1 {
        "1 canción".to_owned()
    } else {
        format!("{n} canciones")
    }
}

/// Tamaño de las portadas en la lista.
const LIST_COVER: f32 = 36.0;
/// Alto fijo de cada fila de canción (para saber cuáles se ven).
pub(crate) const ROW_HEIGHT: f32 = 50.0;
/// Filas extra que se arman arriba y abajo de las que se ven.
const OVERSCAN: usize = 12;
/// Alto que se supone visible antes del primer desplazamiento.
const DEFAULT_VIEW_HEIGHT: f32 = 1400.0;
/// Id de la lista de canciones de la playlist.
pub(crate) const LIST_SCROLL: &str = "playlist-list";

/// Rango `[first, last)` de filas a armar, según el desplazamiento y el alto
/// visible de la lista.
pub(crate) fn visible_rows((offset, height): (f32, f32), len: usize) -> (usize, usize) {
    let height = if height > 0.0 {
        height
    } else {
        DEFAULT_VIEW_HEIGHT
    };
    let top = (offset.max(0.0) / ROW_HEIGHT) as usize;
    let count = (height / ROW_HEIGHT).ceil() as usize + 2 * OVERSCAN;
    let first = top.saturating_sub(OVERSCAN).min(len.saturating_sub(count));
    (first, (first + count).min(len))
}

/// `1536000` → `1,5 MB`.
pub(crate) fn format_bytes(b: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = b as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0).replace('.', ",")
    } else {
        format!("{mb:.0} MB")
    }
}

/// ¿La canción coincide con la búsqueda? (sin distinguir mayúsculas ni tildes)
pub fn matches_filter(track: &Track, query: &str) -> bool {
    let q = fold(query);
    if q.is_empty() {
        return true;
    }
    let hay = fold(&format!(
        "{} {} {}",
        track.title,
        track.artists.join(" "),
        track.album.as_deref().unwrap_or("")
    ));
    q.split_whitespace().all(|w| hay.contains(w))
}

fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_visible_rows_are_built() {
        // Arriba de todo, sin saber el alto: unas 52 filas.
        let (first, last) = visible_rows((0.0, 0.0), 1837);
        assert_eq!(first, 0);
        assert!(last > 28 && last < 60, "{last}");
        // A mitad de la lista, con 500 px visibles: la fila 1000 está y hay margen.
        let (first, last) = visible_rows((1000.0 * ROW_HEIGHT, 500.0), 1837);
        assert_eq!(first, 1000 - OVERSCAN);
        assert_eq!(last, first + 10 + 2 * OVERSCAN);
        // Al final no se pasa del largo, y una lista corta va entera.
        assert_eq!(visible_rows((1e9, 500.0), 1837).1, 1837);
        assert_eq!(visible_rows((0.0, 500.0), 5), (0, 5));
        assert_eq!(visible_rows((300.0, 500.0), 0), (0, 0));
    }

    #[test]
    fn filter_ignores_case_and_accents() {
        let t = Track {
            title: "Canción Animal".into(),
            artists: vec!["Soda Stereo".into()],
            album: None,
            duration_ms: 0,
            isrc: None,
            spotify_id: None,
        };
        assert!(matches_filter(&t, "cancion"));
        assert!(matches_filter(&t, "SODA animal"));
        assert!(!matches_filter(&t, "cerati"));
        assert!(matches_filter(&t, "  "));
    }
}
