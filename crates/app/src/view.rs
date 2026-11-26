//! Vistas: barra lateral, importar, lista de la playlist (con búsqueda y
//! recomendadas), ajustes y selector de video.

use iced::widget::{
    button, column, container, image, mouse_area, row, scrollable, stack, text, text_input,
    tooltip, Column, Id, Space,
};
use iced::{mouse, Alignment, Color, Element, Length};
use surco_core::{format_duration, Playlist, Track};

use crate::i18n::{self, Lang};
use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::playlists::{OWN_SOURCE, RENAME_INPUT};
use crate::settings::{Change, Precache};
use crate::{skeleton, theme, DragList, Message, Screen, Surco, FILTER_INPUT, IMPORT_INPUT};

/// Portada de las playlists de la barra lateral.
const SIDEBAR_COVER: f32 = 28.0;

impl Surco {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let main: Element<'_, Message> = match self.screen {
            Screen::Welcome => {
                // El diálogo de herramientas también va encima de la bienvenida:
                // se bajan mientras se elige el tema.
                let welcome = self.with_backdrop(self.view_welcome());
                return match self.view_tools_popup() {
                    Some(popup) => stack![welcome, popup].into(),
                    None => welcome,
                };
            }
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

        let app: Element<'_, Message> = if theme::colors().look == theme::Look::Glass {
            // Vidrio: dos paneles flotando sobre el fondo.
            column![
                container(row![self.view_sidebar(), main])
                    .height(Length::Fill)
                    .clip(true)
                    .style(theme::glass_panel),
                self.view_player_bar()
            ]
            .spacing(10)
            .padding(12)
            .into()
        } else {
            let (line, width) = theme::divider().unwrap_or((Color::TRANSPARENT, 0.0));
            let vertical = container(Space::new())
                .width(if theme::colors().look == theme::Look::Brutal {
                    2.0
                } else {
                    width
                })
                .height(Length::Fill)
                .style(theme::rule(
                    if theme::colors().look == theme::Look::Brutal {
                        theme::colors().border
                    } else {
                        line
                    },
                ));
            let horizontal = container(Space::new())
                .width(Length::Fill)
                .height(width)
                .style(theme::rule(line));
            column![
                row![self.view_sidebar(), vertical, main].height(Length::Fill),
                horizontal,
                self.view_player_bar()
            ]
            .into()
        };
        let app = self.with_backdrop(app);
        let layer = if let Some(card) = self.view_tools_popup() {
            Some(card)
        } else if let Some(card) = self.view_update() {
            Some(card)
        } else if let Some(i) = self.confirm_remove {
            Some(self.view_confirm_remove(i))
        } else if let Some(card) = self
            .chooser_popup
            .then(|| self.view_chooser_popup())
            .flatten()
        {
            Some(card)
        } else if let Some(menu) = &self.song_menu {
            Some(self.view_song_menu(menu))
        } else if let Some(i) = self.playlist_menu {
            Some(self.view_playlist_menu(i))
        } else {
            self.view_volume_popup()
        };
        // Siempre la misma forma (app + capa), aunque no haya menú: si el
        // árbol cambia al abrir un menú, las listas pierden el scroll y
        // vuelven arriba.
        let layer = layer.unwrap_or_else(|| Space::new().into());
        let app: Element<'_, Message> = stack![app, layer].into();
        crate::pointer::Pointer::new(app, Message::Pointer).into()
    }

    /// Temas Vidrio y Brillo: fondo dibujado por código detrás de todo, en
    /// cuadritos para que al redibujar una parte no se repinte el fondo
    /// entero (ver [`crate::backdrop`]).
    fn with_backdrop<'a>(&self, app: Element<'a, Message>) -> Element<'a, Message> {
        if !crate::backdrop::active() {
            return app;
        }
        let back = iced::widget::responsive(|size| {
            let Some(t) = crate::backdrop::tiles(size.width, size.height) else {
                return Space::new().into();
            };
            let tile = crate::backdrop::TILE as f32;
            let mut handles = t.handles.into_iter();
            let rows = (0..t.rows).map(|_| {
                iced::widget::Row::with_children((0..t.cols).filter_map(|_| {
                    let h = handles.next()?;
                    Some(
                        // Sin escalar: la última fila puede quedar más baja
                        // que el cuadrito y se recorta.
                        image(h)
                            .width(tile)
                            .height(tile)
                            .content_fit(iced::ContentFit::None)
                            .filter_method(image::FilterMethod::Nearest)
                            .into(),
                    )
                }))
                .into()
            });
            container(Column::with_children(rows))
                .width(Length::Fill)
                .height(Length::Fill)
                .clip(true)
                .into()
        });
        stack![back, app].into()
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
            let cover: Element<'_, Message> =
                match self.playlist_cover(i).and_then(|v| self.covers.get(v)) {
                    _ if theme::minimal() => Space::new().width(0).into(),
                    Some(h) => image(h.clone())
                        .width(SIDEBAR_COVER)
                        .height(SIDEBAR_COVER)
                        .border_radius(if theme::radius() > 0.0 { 4.0 } else { 0.0 })
                        .into(),
                    None => container(Space::new())
                        .width(SIDEBAR_COVER)
                        .height(SIDEBAR_COVER)
                        .style(theme::cover_placeholder)
                        .into(),
                };
            let mut content = row![cover, label].spacing(10).align_y(Alignment::Center);
            if playing {
                content = content.push(icons::glyph(Kind::Volume, theme::colors().accent, 13.0));
            }
            list = list.push(
                mouse_area(
                    container(content)
                        .padding([4, 6])
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
        let brutal = theme::colors().look == theme::Look::Brutal;
        let nav = |kind: Kind, label: &'static str, screen: Screen, msg: Message| {
            let on = self.screen == screen;
            button(
                row![
                    icons::icon(
                        kind,
                        match (on, theme::colors().look) {
                            (true, theme::Look::Brutal) => theme::colors().on_accent,
                            (true, _) => theme::colors().accent,
                            _ => theme::colors().text_secondary,
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
            .padding(if brutal { [8, 10] } else { [6, 8] })
            .style(theme::list_button(on))
        };
        let queue_len = self
            .active
            .as_ref()
            .map(|a| a.queue.len().saturating_sub(a.queue.pos() + 1))
            .unwrap_or(0);
        let nav = column![
            nav(
                Kind::Home,
                tr!("Inicio", "Home"),
                Screen::Home,
                Message::GoHome
            ),
            nav(
                Kind::Search,
                tr!("Buscar", "Search"),
                Screen::Search,
                Message::Search(crate::search::SearchMsg::Open)
            ),
            nav(
                Kind::Queue,
                if queue_len > 0 {
                    tr!("Cola", "Queue")
                } else {
                    tr!("Cola (vacía)", "Queue (empty)")
                },
                Screen::Queue,
                Message::ToggleQueue
            ),
            nav(
                Kind::Import,
                tr!("Importar", "Import"),
                Screen::Import,
                Message::OpenImport
            ),
        ]
        .spacing(theme::nav_gap());

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
            .padding(if brutal { 6 } else { 4 })
            .style(if brutal {
                theme::brand_button
            } else {
                theme::icon_button
            }),
            container(text(tr!("Ajustes", "Settings")).size(theme::SIZE_S))
                .padding(6)
                .style(theme::card),
            tooltip::Position::Bottom,
        );
        let logo = icons::logo(28.0);
        let name = if brutal {
            text("Surco")
                .size(theme::SIZE_L + 2)
                .font(theme::BOLD)
                .color(theme::colors().accent)
        } else {
            text("Surco").size(theme::SIZE_L)
        };
        let brand = button(row![logo, name].spacing(8).align_y(Alignment::Center))
            .on_press(Message::GoHome)
            .padding(if brutal { [6, 10] } else { [0, 0] })
            .style(if brutal {
                theme::brand_button
            } else {
                theme::plain_button
            });
        let library_title: Element<'_, Message> = if brutal {
            text(tr!("Tu biblioteca", "Your library"))
                .size(theme::SIZE_M + 1)
                .font(theme::BOLD)
                .width(Length::Fill)
                .into()
        } else {
            text(tr!("Tu biblioteca", "Your library"))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .width(Length::Fill)
                .into()
        };
        let library_header = row![
            library_title,
            tooltip(
                button(icons::icon(Kind::Add, theme::add_icon(), 14.0))
                    .on_press(Message::NewPlaylist)
                    .padding(4)
                    .style(theme::add_button),
                container(text(tr!("Nueva playlist", "New playlist")).size(theme::SIZE_S))
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
            container(Space::new())
                .width(Length::Fill)
                .height(if brutal { 2 } else { 1 })
                .style(theme::rule(theme::colors().border)),
            column![
                library_header,
                crate::smooth::smooth(scrollable(list).height(Length::Fill))
            ]
            .spacing(if brutal { 12 } else { 6 }),
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
            .style(theme::sidebar)
            .into()
    }

    /// Pantalla "Importar": pegar un link o elegir un CSV, con una tarjeta
    /// por origen, las últimas importadas y una guía rápida.
    pub(crate) fn view_import(&self) -> Element<'_, Message> {
        let c = theme::colors();
        let hint = |t: &'static str| text(t).size(theme::SIZE_S).style(theme::secondary_text);
        let header = column![
            text(tr!("Importar tus playlists", "Import your playlists")).size(theme::SIZE_L + 6),
            hint(tr!(
                "Traé tus playlists de Spotify, YouTube o YouTube Music pegando el link, o subiendo un archivo.",
                "Bring your playlists from Spotify, YouTube or YouTube Music by pasting the link, or from a file."
            )),
            container(
                row![
                    hint(tr!("O presioná", "Or press")),
                    container(text("Ctrl+L").size(12).font(iced::Font::MONOSPACE))
                        .padding([1, 6])
                        .style(theme::chip),
                    hint(tr!("desde cualquier lugar", "from anywhere")),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([4, 8])
            .style(theme::inset),
        ]
        .spacing(8);

        let input = text_input(
            tr!(
                "Pegá el link de la playlist de Spotify, YouTube, etc.",
                "Paste the Spotify, YouTube… playlist link"
            ),
            &self.import_input,
        )
        .id(Id::new(IMPORT_INPUT))
        .on_input(Message::ImportInputChanged)
        .on_submit(Message::Import)
        .padding(12)
        .size(theme::SIZE_M)
        .style(theme::input);
        let import = button(
            row![
                icons::icon(Kind::Import, c.on_accent, 16.0),
                text(tr!("Importar playlist", "Import playlist")).size(theme::SIZE_S),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press_maybe((!self.importing).then_some(Message::Import))
        .padding([12, 18])
        .style(theme::primary_button);
        let mut field =
            column![row![input, import].spacing(12).align_y(Alignment::Center)].spacing(12);
        if let Some(n) = self.notice.as_ref().filter(|n| n.error) {
            field = field.push(text(&n.text).size(theme::SIZE_S).style(theme::error_text));
        }
        if self.importing {
            field = field.push(
                column![
                    hint(tr!("Importando…", "Importing…")),
                    skeleton::song_rows(4, self.pulse),
                ]
                .spacing(10),
            );
        }
        let field = container(field)
            .padding(20)
            .width(Length::Fill)
            .style(theme::card);

        // Una tarjeta por origen.
        let source = |kind: Kind,
                      title: &'static str,
                      sub: &'static str,
                      tip: &'static str,
                      action: Element<'static, Message>| {
            container(
                column![
                    container(icons::icon(kind, c.accent, 22.0))
                        .padding(8)
                        .style(theme::chip),
                    column![text(title).size(theme::SIZE_M + 1), hint(sub)].spacing(2),
                    container(Space::new())
                        .width(Length::Fill)
                        .height(1)
                        .style(theme::rule(c.border)),
                    row![hint(tip).width(Length::Fill), action]
                        .spacing(10)
                        .align_y(Alignment::Center),
                ]
                .spacing(12),
            )
            .padding(18)
            .width(Length::Fill)
            .style(theme::card)
        };
        let small = |label: &'static str, msg: Option<Message>| -> Element<'static, Message> {
            button(text(label).size(theme::SIZE_S))
                .on_press_maybe(msg)
                .padding([6, 12])
                .style(theme::ghost_button)
                .into()
        };
        let busy = self.importing;
        let cards = row![
            source(
                Kind::Queue,
                "Spotify",
                tr!(
                    "Compartir > Copiar enlace a la playlist",
                    "Share > Copy link to playlist"
                ),
                tr!(
                    "Playlists públicas, sin cuenta ni claves.",
                    "Public playlists, no account or keys."
                ),
                small(
                    tr!("Pegar enlace", "Paste link"),
                    (!busy).then_some(Message::ImportPaste)
                ),
            ),
            source(
                Kind::Play,
                "YouTube / YouTube Music",
                tr!(
                    "Enlace de la playlist o de un video de la lista",
                    "Playlist link, or a video in the list"
                ),
                tr!("También mixes y álbumes.", "Mixes and albums too."),
                small(
                    tr!("Pegar enlace", "Paste link"),
                    (!busy).then_some(Message::ImportPaste)
                ),
            ),
            source(
                Kind::Import,
                tr!("Archivo CSV", "CSV file"),
                tr!(
                    "Exportado desde exportify.net, TuneMyMusic, etc.",
                    "Exported from exportify.net, TuneMyMusic, etc."
                ),
                tr!(
                    "Con columnas de título y artista.",
                    "With title and artist columns."
                ),
                small(
                    tr!("Elegir archivo…", "Choose file…"),
                    (!busy).then_some(Message::PickCsv)
                ),
            ),
        ]
        .spacing(16);

        // Últimas importadas (las más nuevas primero).
        let imported: Vec<usize> = (0..self.playlists.len())
            .rev()
            .filter(|&i| {
                let src = self.playlists[i].1.as_str();
                src != OWN_SOURCE && src != "radio"
            })
            .take(4)
            .collect();
        let history: Element<'_, Message> = if imported.is_empty() {
            hint(tr!(
                "Todavía no importaste ninguna playlist.",
                "You haven't imported any playlist yet."
            ))
            .into()
        } else {
            Column::with_children(imported.into_iter().map(|i| {
                let (p, src) = &self.playlists[i];
                button(
                    row![
                        crate::home::cover(self, self.playlist_cover(i), 40.0),
                        column![
                            text(&p.name)
                                .size(theme::SIZE_M)
                                .wrapping(text::Wrapping::None),
                            text(format!("{} · {}", source_name(src), songs(p.tracks.len())))
                                .size(theme::SIZE_S)
                                .style(theme::secondary_text),
                        ]
                        .spacing(2)
                        .width(Length::Fill)
                        .clip(true),
                        container(
                            text(tr!("✓ Importada", "✓ Imported"))
                                .size(12)
                                .style(theme::accent_text),
                        )
                        .padding([3, 10])
                        .style(theme::chip),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::SelectPlaylist(Some(i)))
                .padding([6, 8])
                .width(Length::Fill)
                .style(theme::song_row(false))
                .into()
            }))
            .spacing(4)
            .into()
        };
        let step = |n: &'static str, label: &'static str| {
            row![
                container(text(n).size(12).color(c.on_accent))
                    .center(22)
                    .style(theme::badge_round),
                text(label).size(theme::SIZE_S),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        };
        let bottom = row![
            container(
                column![
                    text(tr!("Últimas importaciones", "Recent imports")).size(theme::SIZE_M + 1),
                    history
                ]
                .spacing(12)
            )
            .padding(18)
            .width(Length::FillPortion(2))
            .style(theme::card),
            container(
                column![
                    text(tr!("Guía rápida", "Quick guide")).size(theme::SIZE_M + 1),
                    step(
                        "1",
                        tr!("Copiá el enlace de tu app", "Copy the link from your app")
                    ),
                    step("2", tr!("Pegalo acá arriba", "Paste it up here")),
                    step(
                        "3",
                        tr!(
                            "¡Listo! Surco busca cada canción en YouTube",
                            "Done! Surco finds each song on YouTube"
                        )
                    ),
                ]
                .spacing(14)
            )
            .padding(18)
            .width(Length::FillPortion(1))
            .style(theme::card),
        ]
        .spacing(16);

        crate::smooth::smooth(
            scrollable(
                column![header, field, cards, bottom]
                    .spacing(22)
                    .padding(iced::Padding::ZERO.right(16)),
            )
            .height(Length::Fill),
        )
    }

    pub(crate) fn view_playlist<'a>(
        &'a self,
        index: usize,
        p: &'a Playlist,
        source: &str,
    ) -> Element<'a, Message> {
        let total_ms: u64 = p.tracks.iter().map(|t| t.duration_ms).sum();
        let origin = match source {
            "radio" => tr!(
                "canciones parecidas, del Mix de YouTube",
                "similar songs, from the YouTube Mix"
            )
            .to_owned(),
            OWN_SOURCE => tr!("playlist tuya", "your playlist").to_owned(),
            _ if !i18n::en() => format!("importada desde {source}"),
            // Los nombres de origen se guardan en español (los da el importador).
            _ => {
                let source = match source {
                    "reproductor web de Spotify" => "the Spotify web player",
                    "Web API de Spotify" => "the Spotify Web API",
                    "página embed" => "the Spotify embed page",
                    other => other,
                };
                format!("imported from {source}")
            }
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
            info.push_str(&trf!(
                " · precargadas {} de {}{}",
                " · preloaded {} of {}{}",
                self.precache.ready,
                self.precache.target,
                if self.precache.running.is_some() {
                    tr!(" (bajando…)", " (downloading…)")
                } else {
                    ""
                }
            ));
        }
        let name: Element<'_, Message> = match &self.renaming {
            Some((r, value)) if *r == index => row![
                text_input(tr!("Nombre de la playlist", "Playlist name"), value)
                    .id(Id::new(RENAME_INPUT))
                    .on_input(Message::RenameInput)
                    .on_submit(Message::RenameCommit)
                    .padding(6)
                    .size(theme::SIZE_L)
                    .style(theme::input),
                button(text(tr!("Guardar", "Save")).size(theme::SIZE_S))
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
                    container(text(tr!("Renombrar", "Rename")).size(theme::SIZE_S))
                        .padding(6)
                        .style(theme::card),
                    tooltip::Position::Bottom,
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        };
        let tip = |content: Element<'a, Message>, label: &'static str| -> Element<'a, Message> {
            tooltip(
                content,
                container(text(label).size(theme::SIZE_S))
                    .padding(6)
                    .style(theme::card),
                tooltip::Position::Bottom,
            )
            .into()
        };
        let mut actions = row![].spacing(4).align_y(Alignment::Center);
        actions = actions.push(tip(
            button(icons::icon(
                Kind::Close,
                theme::colors().text_secondary,
                16.0,
            ))
            .on_press(Message::AskRemovePlaylist(Some(index)))
            .padding(7)
            .style(theme::icon_button)
            .into(),
            tr!("Quitar playlist", "Remove playlist"),
        ));
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
                Some(TrackRes::Resolving) => text(tr!("buscando…", "searching…"))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into(),
                Some(TrackRes::Done(m)) if m.is_doubtful() => {
                    button(text(tr!("dudosa", "unsure")).size(theme::SIZE_S))
                        .on_press(Message::OpenChooser(Some((index, i))))
                        .padding([2, 8])
                        .style(theme::ghost_button)
                        .into()
                }
                Some(TrackRes::Failed(e)) => tooltip(
                    text(tr!("sin match", "no match"))
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
            let number = container(text(i + 1).size(theme::SIZE_S).style(if playing {
                theme::accent_text
            } else {
                theme::secondary_text
            }))
            .width(40);
            let cover: Element<'_, Message> = if self.settings.covers_in_list && !theme::minimal() {
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
                        crate::marquee::marquee(
                            text(&t.title)
                                .size(theme::SIZE_M)
                                .wrapping(text::Wrapping::None),
                            self.is_hovered(t),
                        ),
                        text(t.artists.join(", "))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(2)
                    .width(Length::Fill)
                    .clip(true),
                    badge,
                    icons::waves_if(playing),
                    crate::search::add_button(t, Some((index, i))),
                    text(format_duration(t.duration_ms))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .on_press(Message::PlayTrack(index, i))
            .padding([2, 8])
            .width(Length::Fill)
            .height(ROW_HEIGHT - theme::row_gap() as f32)
            .style(theme::song_row(playing));
            let row = column![row, Space::new().height(theme::row_gap())];
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
                text_input(
                    tr!("Buscar en la playlist", "Search in the playlist"),
                    query
                )
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
                text(trf!(
                    "Nada coincide con «{}».",
                    "Nothing matches “{}”.",
                    query.trim()
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            );
        }
        if p.tracks.is_empty() {
            col = col.push(
                column![
                    text(tr!("Está vacía. Buscá canciones y agregalas con el + de cada una (o con clic derecho), o sumá de las recomendadas.", "It's empty. Search for songs and add them with the + on each one (or right-click), or add some of the recommended ones."))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                    // Por si en realidad querían pegar el link de una playlist.
                    row![
                        text(tr!("¿Querías traer una playlist de Spotify o YouTube?", "Did you want to bring a playlist from Spotify or YouTube?"))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        button(text(tr!("Importar una playlist", "Import a playlist")).size(theme::SIZE_S))
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
        if !self.chooser_popup {
            if let Some(chooser) = self.view_chooser(index) {
                col = col.push(chooser);
            }
        }
        // Solo avisa cuando cambian las filas que hay que armar: rearmar
        // toda la vista en cada cuadro del scroll lo vuelve entrecortado.
        let range = visible_rows(self.list_view, usize::MAX);
        col.push(crate::smooth::smooth_filtered(
            scrollable(rows)
                .id(Id::new(LIST_SCROLL))
                .on_scroll(|v| Message::ListScrolled(v.absolute_offset().y, v.bounds().height))
                .height(Length::Fill),
            move |m| match m {
                Message::ListScrolled(offset, height) => {
                    visible_rows((*offset, *height), usize::MAX) != range
                }
                _ => true,
            },
        ))
        .into()
    }

    /// Botones de idioma (ajustes y bienvenida).
    pub(crate) fn languages_row(&self) -> Element<'_, Message> {
        row(Lang::ALL.iter().map(|&l| {
            button(text(l.name()).size(theme::SIZE_S))
                .on_press(Message::Setting(Change::Language(l)))
                .padding([6, 10])
                .style(theme::choice_button(self.settings.language == l))
                .into()
        }))
        .spacing(8)
        .into()
    }

    /// Opciones de precarga (ajustes y bienvenida).
    pub(crate) fn precache_row(&self) -> Element<'_, Message> {
        row(Precache::ALL.iter().map(|&p| {
            button(text(p.label()).size(theme::SIZE_S))
                .on_press(Message::Setting(Change::Precache(p)))
                .padding([6, 10])
                .style(theme::choice_button(self.settings.precache == p))
                .into()
        }))
        .spacing(8)
        .into()
    }

    /// − 100 % + del tamaño de la interfaz (ajustes y bienvenida).
    pub(crate) fn scale_row(&self) -> Element<'_, Message> {
        row![
            button(text("−").size(theme::SIZE_M))
                .on_press_maybe(
                    (self.settings.ui_scale > crate::settings::SCALE_MIN)
                        .then_some(Message::Setting(Change::Scale(-1)))
                )
                .padding([2, 12])
                .style(theme::ghost_button),
            text(format!("{} %", self.settings.ui_scale))
                .size(theme::SIZE_S)
                .width(56)
                .align_x(iced::alignment::Horizontal::Center),
            button(text("+").size(theme::SIZE_M))
                .on_press_maybe(
                    (self.settings.ui_scale < crate::settings::SCALE_MAX)
                        .then_some(Message::Setting(Change::Scale(1)))
                )
                .padding([2, 12])
                .style(theme::ghost_button),
            button(text(tr!("Normal", "Reset")).size(theme::SIZE_S))
                .on_press_maybe(
                    (self.settings.ui_scale != 100).then_some(Message::Setting(Change::Scale(0)))
                )
                .padding([4, 10])
                .style(theme::ghost_button),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    /// Tema personalizado: el fondo y el acento, con un campo `#RRGGBB` y
    /// unas muestras para elegir rápido.
    pub(crate) fn view_custom_theme(&self) -> Element<'_, Message> {
        const BACKGROUNDS: [u32; 9] = [
            0x0E0E10, 0x14161C, 0x1B2030, 0x201A28, 0x10201A, 0x2A1C16, 0xF7F4EE, 0xEEF2F7,
            0xFFFFFF,
        ];
        const ACCENTS: [u32; 9] = [
            0xF2A65A, 0xFF6B6B, 0xFFD166, 0x06D6A0, 0x4CC9F0, 0x7C9CFF, 0xB388FF, 0xFF6FAE,
            0x2F52C4,
        ];
        let line = |which: usize, label: &'static str, current: Color, presets: [u32; 9]| {
            let input_ok = theme::parse_hex(&self.custom_hex[which]).is_some();
            let presets = row(presets.into_iter().map(|v| {
                let c = Color::from_rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8);
                button(
                    container(Space::new())
                        .width(20)
                        .height(20)
                        .style(theme::swatch(c)),
                )
                .on_press(Message::Setting(Change::CustomColor(
                    which,
                    theme::to_hex(c),
                )))
                .padding(3)
                .style(theme::choice_button(
                    theme::to_hex(current) == theme::to_hex(c),
                ))
                .into()
            }))
            .spacing(4)
            .wrap();
            let hsv = self.custom_hsv[which];
            // Al soltar se guarda el color que quedó.
            let save = Message::Setting(Change::CustomColor(which, theme::to_hex(current)));
            let bar = |name: &'static str,
                       stops: Vec<Color>,
                       range: std::ops::RangeInclusive<f32>,
                       value: f32,
                       set: fn(theme::Hsv, f32) -> theme::Hsv| {
                let slider = iced::widget::slider(range, value, move |x| {
                    Message::Setting(Change::CustomHsv(which, set(hsv, x)))
                })
                .step(0.001_f32)
                .on_release(save.clone())
                .height(18)
                .style(theme::color_slider);
                row![
                    text(name)
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .width(80),
                    stack![
                        container(
                            container(Space::new())
                                .width(Length::Fill)
                                .height(12)
                                .style(theme::color_bar(stops)),
                        )
                        .padding([0, 2])
                        .center_y(18),
                        slider,
                    ]
                    .width(Length::Fill),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
            };
            let hues: Vec<Color> = (0..=6)
                .map(|i| {
                    theme::Hsv {
                        h: i as f32 * 60.0,
                        s: 1.0,
                        v: 1.0,
                    }
                    .to_color()
                })
                .collect();
            let editor = column![
                bar(tr!("Tono", "Hue"), hues, 0.0..=360.0, hsv.h, |c, x| {
                    theme::Hsv { h: x, ..c }
                }),
                bar(
                    tr!("Saturación", "Saturation"),
                    vec![
                        theme::Hsv { s: 0.0, ..hsv }.to_color(),
                        theme::Hsv { s: 1.0, ..hsv }.to_color(),
                    ],
                    0.0..=1.0,
                    hsv.s,
                    |c, x| theme::Hsv { s: x, ..c },
                ),
                bar(
                    tr!("Brillo", "Brightness"),
                    vec![
                        theme::Hsv { v: 0.0, ..hsv }.to_color(),
                        theme::Hsv { v: 1.0, ..hsv }.to_color(),
                    ],
                    0.0..=1.0,
                    hsv.v,
                    |c, x| theme::Hsv { v: x, ..c },
                ),
            ]
            .spacing(10)
            .width(Length::Fill);
            container(
                column![
                    row![
                        container(Space::new())
                            .width(44)
                            .height(44)
                            .style(theme::swatch(current)),
                        column![
                            text(label).size(theme::SIZE_M),
                            text_input("#RRGGBB", &self.custom_hex[which])
                                .on_input(move |s| Message::Setting(Change::CustomColor(which, s)))
                                .padding(6)
                                .size(theme::SIZE_S)
                                .width(100)
                                .style(if input_ok {
                                    theme::input
                                } else {
                                    theme::input_error
                                }),
                        ]
                        .spacing(6),
                        container(presets)
                            .width(Length::Fill)
                            .align_right(Length::Fill),
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                    editor,
                ]
                .spacing(14),
            )
            .padding(14)
            .width(Length::Fill)
            .style(theme::inset)
        };
        let (bg, accent) = self.settings.custom;
        column![
            line(0, tr!("Fondo", "Background"), bg, BACKGROUNDS),
            line(1, tr!("Acento", "Accent"), accent, ACCENTS),
        ]
        .spacing(10)
        .into()
    }

    /// Selector de video arriba de la lista de la playlist.
    pub(crate) fn view_chooser(&self, playlist: usize) -> Option<Element<'_, Message>> {
        let at = self.choosing.filter(|(p, _)| *p == playlist)?;
        Some(
            container(self.chooser_body(at)?)
                .padding(12)
                .max_height(260)
                .style(theme::surface)
                .into(),
        )
    }

    /// Selector de video flotando sobre el botón de la barra de abajo.
    fn view_chooser_popup(&self) -> Option<Element<'_, Message>> {
        const WIDTH: f32 = 520.0;
        const HEIGHT: f32 = 300.0;
        let at = self.choosing?;
        let card = container(self.chooser_body(at)?)
            .padding(14)
            .width(WIDTH)
            .max_height(HEIGHT)
            .style(theme::dialog);
        Some(self.anchored_at(
            card,
            iced::Size::new(WIDTH, HEIGHT),
            Message::OpenChooser(None),
            true,
        ))
    }

    /// Confirmación antes de quitar una playlist.
    fn view_confirm_remove(&self, index: usize) -> Element<'_, Message> {
        let name = self
            .playlists
            .get(index)
            .map(|(p, _)| p.name.as_str())
            .unwrap_or_default();
        let card = container(
            column![
                text(trf!("¿Quitar «{}»?", "Remove “{}”?", name)).size(theme::SIZE_M),
                text(tr!(
                    "Se borra de tu biblioteca. No se puede deshacer.",
                    "It's removed from your library. This can't be undone."
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
                row![
                    Space::new().width(Length::Fill),
                    button(text(tr!("Cancelar", "Cancel")).size(theme::SIZE_S))
                        .on_press(Message::AskRemovePlaylist(None))
                        .padding([6, 14])
                        .style(theme::ghost_button),
                    button(text(tr!("Quitar", "Remove")).size(theme::SIZE_S))
                        .on_press(Message::RemovePlaylist(index))
                        .padding([6, 14])
                        .style(theme::danger_button),
                ]
                .spacing(8),
            ]
            .spacing(12),
        )
        .padding(20)
        .width(380)
        .style(theme::dialog);
        let backdrop = iced::widget::mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::scrim),
        )
        .on_press(Message::AskRemovePlaylist(None));
        iced::widget::opaque(stack![
            backdrop,
            container(iced::widget::opaque(card))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        ])
    }

    /// Lista de candidatos para elegir el video de la canción `at`.
    fn chooser_body(&self, at: (usize, usize)) -> Option<Element<'_, Message>> {
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
                                    .unwrap_or(tr!("canal desconocido", "unknown channel"))
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
            text(trf!(
                "Elegí el video para «{}» ({} canciones de YouTube; el número es el puntaje)",
                "Pick the video for “{}” ({} YouTube songs; the number is the score)",
                track.title,
                m.candidates.len()
            ))
            .size(theme::SIZE_S)
            .width(Length::Fill),
            button(text(tr!("Cerrar", "Close")).size(theme::SIZE_S))
                .on_press(Message::OpenChooser(None))
                .padding([2, 8])
                .style(theme::ghost_button),
        ]
        .align_y(Alignment::Center);
        Some(
            column![
                header,
                crate::smooth::smooth(scrollable(list).height(Length::Shrink))
            ]
            .spacing(8)
            .into(),
        )
    }
}

/// "1 canción", "12 canciones".
pub(crate) fn songs(n: usize) -> String {
    crate::i18n::songs(n)
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

/// Nombre corto del origen de una playlist importada.
fn source_name(src: &str) -> &str {
    let lower = src.to_lowercase();
    if lower.contains("spotify") {
        "Spotify"
    } else if lower.contains("youtube") {
        "YouTube"
    } else if lower.contains("csv") {
        "CSV"
    } else {
        src
    }
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
