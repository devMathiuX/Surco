//! Pantalla de inicio: buscador, playlists (las escuchadas hace poco
//! primero) y canciones escuchadas hace poco.

use iced::widget::{
    button, column, container, image, operation, row, scrollable, text, text_input, Column, Id,
    Space,
};
use iced::{Alignment, Element, Length, Task};
use surco_core::{format_duration, Track};
use surco_resolver::{cache_key, Match};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::search::{SearchMsg, SEARCH_INPUT};
use crate::{theme, Message, Screen, Surco, YtDlpState};

/// Tarjetas de playlists en el inicio.
const TILES: usize = 8;
/// Canciones escuchadas hace poco que se muestran.
const RECENT_SONGS: usize = 8;
/// Historial que se lee para armar esa lista (con repetidas).
const HISTORY_READ: usize = 120;
const TILE_WIDTH: f32 = 250.0;
const TILE_COVER: f32 = 52.0;

/// Lo que muestra el inicio (se recalcula al entrar).
#[derive(Debug, Default)]
pub struct HomeState {
    /// IDs de playlists escuchadas hace poco, la más reciente primero.
    pub recent_playlists: Vec<String>,
    /// Canciones escuchadas hace poco (sin repetir) y su video.
    pub recent_songs: Vec<(Track, Option<String>)>,
}

impl Surco {
    /// Ir al inicio.
    pub(crate) fn go_home(&mut self) -> Task<Message> {
        self.screen = Screen::Home;
        self.filter = None;
        self.refresh_home()
    }

    /// Relee del historial lo escuchado hace poco y pide sus portadas.
    pub(crate) fn refresh_home(&mut self) -> Task<Message> {
        let Some(store) = &self.store else {
            return Task::none();
        };
        self.home.recent_playlists = store.recent_playlists(TILES).unwrap_or_default();
        let played = store.recent(HISTORY_READ).unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let mut songs = Vec::new();
        for p in played {
            let track = p.track();
            if !seen.insert(cache_key(&track)) {
                continue;
            }
            // Que vuelva a sonar el mismo video.
            if let Some(v) = &p.youtube_id {
                if !matches!(self.resolution(&track), Some(TrackRes::Done(_))) {
                    self.resolutions.insert(
                        cache_key(&track),
                        TrackRes::Done(Match {
                            youtube_id: v.clone(),
                            score: 100,
                            manual: true,
                            candidates: Vec::new(),
                        }),
                    );
                }
            }
            songs.push((track, p.youtube_id));
            if songs.len() == RECENT_SONGS {
                break;
            }
        }
        self.home.recent_songs = songs;
        let mut videos: Vec<String> = self
            .home
            .recent_songs
            .iter()
            .filter_map(|(_, v)| v.clone())
            .collect();
        videos.extend(
            self.home_playlists()
                .into_iter()
                .filter_map(|i| self.playlist_cover(i).map(str::to_owned)),
        );
        Task::batch(
            videos
                .iter()
                .map(|v| self.queue_cover(v))
                .collect::<Vec<_>>(),
        )
    }

    /// Playlists del inicio: primero las escuchadas hace poco, después las
    /// demás en el orden de la barra lateral.
    fn home_playlists(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self
            .home
            .recent_playlists
            .iter()
            .filter_map(|id| self.playlists.iter().position(|(p, _)| &p.id == id))
            .collect();
        for i in 0..self.playlists.len() {
            if out.len() >= TILES {
                break;
            }
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out.truncate(TILES);
        out
    }

    /// Video de la primera canción encontrada de una playlist (su portada).
    fn playlist_cover(&self, p: usize) -> Option<&str> {
        let (playlist, _) = self.playlists.get(p)?;
        playlist.tracks.iter().find_map(|t| self.video_of_track(t))
    }

    /// Escribir en el buscador del inicio lleva al buscador.
    pub(crate) fn home_search(&mut self, q: String) -> Task<Message> {
        self.screen = Screen::Search;
        let task = self.on_search(SearchMsg::Input(q));
        Task::batch([
            task,
            operation::focus(Id::new(SEARCH_INPUT)),
            operation::move_cursor_to_end(Id::new(SEARCH_INPUT)),
        ])
    }

    /// Escuchar una canción de "Escuchadas hace poco" (sigue con parecidas).
    pub(crate) fn play_recent(&mut self, i: usize) -> Task<Message> {
        let Some((track, _)) = self.home.recent_songs.get(i).cloned() else {
            return Task::none();
        };
        self.play_list(vec![track], 0, true)
    }

    pub(crate) fn view_home(&self) -> Element<'_, Message> {
        let search = row![
            icons::icon(Kind::Search, theme::colors().text_secondary, 18.0),
            text_input("¿Qué querés escuchar?", "")
                .on_input(Message::HomeSearch)
                .padding(12)
                .size(theme::SIZE_M)
                .style(theme::input),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let mut col = column![text("Inicio").size(theme::SIZE_L), search].spacing(18);

        if let Some(banner) = self.view_ytdlp_banner() {
            col = col.push(banner);
        }

        // Playlists.
        if self.playlists.is_empty() {
            col = col.push(
                container(
                    column![
                        text("Todavía no tenés playlists").size(theme::SIZE_M),
                        text("Importá una de Spotify o YouTube, o armá la tuya con canciones del buscador.")
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        row![
                            button(text("Importar playlist").size(theme::SIZE_S))
                                .on_press(Message::OpenImport)
                                .padding([6, 14])
                                .style(theme::primary_button),
                            button(text("Crear playlist").size(theme::SIZE_S))
                                .on_press(Message::NewPlaylist)
                                .padding([6, 14])
                                .style(theme::ghost_button),
                        ]
                        .spacing(8),
                    ]
                    .spacing(10),
                )
                .padding(16)
                .width(Length::Fill)
                .style(theme::card),
            );
        } else {
            let tiles = row(self.home_playlists().into_iter().map(|i| self.tile(i))).spacing(10);
            col = col.push(
                column![
                    text(if self.home.recent_playlists.is_empty() {
                        "Tus playlists"
                    } else {
                        "Escuchadas hace poco"
                    })
                    .size(theme::SIZE_M),
                    tiles.wrap().vertical_spacing(10)
                ]
                .spacing(10),
            );
        }

        // Canciones.
        if !self.home.recent_songs.is_empty() {
            let list = Column::with_children(
                self.home
                    .recent_songs
                    .iter()
                    .enumerate()
                    .map(|(i, (t, v))| self.recent_row(i, t, v.as_deref())),
            )
            .spacing(2);
            col = col.push(
                column![text("Canciones que escuchaste").size(theme::SIZE_M), list].spacing(10),
            );
        }

        scrollable(col.padding(iced::Padding::ZERO.right(16)).max_width(1100))
            .height(Length::Fill)
            .into()
    }

    fn tile(&self, i: usize) -> Element<'_, Message> {
        let Some((p, source)) = self.playlists.get(i) else {
            return Space::new().into();
        };
        let cover: Element<'_, Message> =
            match self.playlist_cover(i).and_then(|v| self.covers.get(v)) {
                Some(h) => image(h.clone())
                    .width(TILE_COVER)
                    .height(TILE_COVER)
                    .border_radius(4.0)
                    .into(),
                None => container(Space::new())
                    .width(TILE_COVER)
                    .height(TILE_COVER)
                    .style(theme::cover_placeholder)
                    .into(),
            };
        let detail = if source == "radio" {
            "Radio".to_owned()
        } else {
            crate::view::songs(p.tracks.len())
        };
        button(
            row![
                cover,
                column![
                    text(&p.name)
                        .size(theme::SIZE_S + 1)
                        .wrapping(text::Wrapping::None),
                    text(detail)
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                ]
                .spacing(2)
                .width(Length::Fill)
                .clip(true),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SelectPlaylist(Some(i)))
        .padding(6)
        .width(TILE_WIDTH)
        .style(theme::tile)
        .into()
    }

    fn recent_row<'a>(
        &'a self,
        i: usize,
        t: &'a Track,
        video: Option<&'a str>,
    ) -> Element<'a, Message> {
        let cover: Element<'_, Message> = match video.and_then(|v| self.covers.get(v)) {
            Some(h) => image(h.clone())
                .width(36)
                .height(36)
                .border_radius(4.0)
                .into(),
            None => container(Space::new())
                .width(36)
                .height(36)
                .style(theme::cover_placeholder)
                .into(),
        };
        let playing = self
            .now_track()
            .is_some_and(|n| cache_key(n) == cache_key(t));
        let duration = if t.duration_ms > 0 {
            format_duration(t.duration_ms)
        } else {
            String::new()
        };
        button(
            row![
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
                crate::search::add_button(t, None),
                text(duration)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::PlayRecent(i))
        .padding([4, 8])
        .width(Length::Fill)
        .style(theme::list_button(playing))
        .into()
    }

    /// Aviso si falta yt-dlp (con el botón para bajarlo) o si el audio no arrancó.
    pub(crate) fn view_ytdlp_banner(&self) -> Option<Element<'_, Message>> {
        let content: Element<'_, Message> = match &self.ytdlp {
            YtDlpState::Found(_) => text(format!(
                "No se pudo abrir la salida de audio: {}",
                self.fatal.as_ref()?
            ))
            .size(theme::SIZE_S)
            .style(theme::error_text)
            .into(),
            YtDlpState::Missing => row![
                text("Surco necesita yt-dlp para buscar y escuchar música de YouTube.")
                    .size(theme::SIZE_S)
                    .width(Length::Fill),
                button(text("Descargar yt-dlp").size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .padding([6, 14])
                    .style(theme::primary_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            YtDlpState::Downloading => text("Descargando yt-dlp…").size(theme::SIZE_S).into(),
            YtDlpState::Failed(e) => row![
                text(format!("No se pudo descargar yt-dlp: {e}"))
                    .size(theme::SIZE_S)
                    .style(theme::error_text)
                    .width(Length::Fill),
                button(text("Reintentar").size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .padding([6, 14])
                    .style(theme::ghost_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        };
        Some(
            container(content)
                .padding(14)
                .width(Length::Fill)
                .style(theme::card)
                .into(),
        )
    }
}
