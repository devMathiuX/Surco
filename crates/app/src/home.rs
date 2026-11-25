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
const TILE_WIDTH: f32 = 270.0;
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
        // Portadas de las playlists (inicio y barra lateral).
        videos.extend(
            (0..self.playlists.len()).filter_map(|i| self.playlist_cover(i).map(str::to_owned)),
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
    pub(crate) fn playlist_cover(&self, p: usize) -> Option<&str> {
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
            text_input(
                tr!("¿Qué querés escuchar?", "What do you want to listen to?"),
                ""
            )
            .on_input(Message::HomeSearch)
            .padding(12)
            .size(theme::SIZE_M)
            .style(theme::input),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let mut col = column![search].spacing(18);

        if let Some(banner) = self.view_ytdlp_banner() {
            col = col.push(banner);
        }

        // Playlists.
        if self.playlists.is_empty() {
            col = col.push(
                container(
                    column![
                        text(tr!("Todavía no tenés playlists", "You don't have any playlists yet")).size(theme::SIZE_M),
                        text(tr!("Importá una de Spotify o YouTube, o armá la tuya con canciones del buscador.", "Import one from Spotify or YouTube, or build your own with songs from search."))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        row![
                            button(text(tr!("Importar playlist", "Import playlist")).size(theme::SIZE_S))
                                .on_press(Message::OpenImport)
                                .padding([6, 14])
                                .style(theme::primary_button),
                            button(text(tr!("Crear playlist", "Create playlist")).size(theme::SIZE_S))
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
                    heading(
                        if self.home.recent_playlists.is_empty() {
                            tr!("Tus playlists", "Your playlists")
                        } else {
                            tr!("Escuchadas hace poco", "Recently played")
                        },
                        true
                    ),
                    tiles.wrap().vertical_spacing(10)
                ]
                .spacing(12),
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
            .spacing(theme::row_gap());
            col = col.push(
                column![
                    heading(tr!("Canciones que escuchaste", "Songs you played"), false),
                    list
                ]
                .spacing(12),
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
        let detail: Element<'_, Message> = if theme::colors().look == theme::Look::Brutal {
            container(text(detail).size(theme::SIZE_S).color(iced::Color::BLACK))
                .padding([1, 6])
                .style(theme::badge)
                .into()
        } else {
            text(detail)
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into()
        };
        // Play directo: si ya suena esta playlist, pausa.
        let this_playing = self.active.as_ref().is_some_and(|a| a.origin == Some(i))
            && matches!(self.status.state, surco_core::PlayerState::Playing);
        let play = button(
            container(icons::icon(
                if this_playing {
                    Kind::Pause
                } else {
                    Kind::Play
                },
                theme::tile_play_icon(),
                14.0,
            ))
            .center(14),
        )
        .on_press_maybe((!p.tracks.is_empty()).then(|| {
            if self.active.as_ref().is_some_and(|a| a.origin == Some(i)) {
                Message::TogglePause
            } else {
                Message::PlayTrack(i, 0)
            }
        }))
        .padding(8)
        .style(theme::tile_play);
        button(
            row![
                cover,
                column![
                    text(&p.name)
                        .size(theme::SIZE_S + 1)
                        .font(if theme::colors().look == theme::Look::Brutal {
                            theme::BOLD
                        } else {
                            iced::Font::DEFAULT
                        })
                        .wrapping(text::Wrapping::None),
                    detail,
                ]
                .spacing(4)
                .width(Length::Fill)
                .clip(true),
                play,
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SelectPlaylist(Some(i)))
        .padding(8)
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
        let row = button(
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
                icons::waves_if(playing),
                crate::search::add_button(t, None),
                text(duration)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::PlayRecent(i))
        .padding([5, 8])
        .width(Length::Fill)
        .style(theme::song_row(playing));
        crate::search::with_menu(row, t, None)
    }

    /// Aviso si falta yt-dlp (con el botón para bajarlo) o si el audio no arrancó.
    pub(crate) fn view_ytdlp_banner(&self) -> Option<Element<'_, Message>> {
        let content: Element<'_, Message> = match &self.ytdlp {
            YtDlpState::Found(_) if self.fatal.is_none() => self.view_deno_banner()?,
            YtDlpState::Found(_) => text(trf!(
                "No se pudo abrir la salida de audio: {}",
                "Couldn't open the audio output: {}",
                self.fatal.as_ref()?
            ))
            .size(theme::SIZE_S)
            .style(theme::error_text)
            .into(),
            YtDlpState::Missing => row![
                text(tr!(
                    "Surco necesita yt-dlp para buscar y escuchar música de YouTube.",
                    "Surco needs yt-dlp to search and play music from YouTube."
                ))
                .size(theme::SIZE_S)
                .width(Length::Fill),
                button(text(tr!("Descargar yt-dlp", "Download yt-dlp")).size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .padding([6, 14])
                    .style(theme::primary_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            YtDlpState::Downloading => text(tr!("Descargando yt-dlp…", "Downloading yt-dlp…"))
                .size(theme::SIZE_S)
                .into(),
            YtDlpState::Failed(e) => row![
                text(trf!(
                    "No se pudo descargar yt-dlp: {e}",
                    "Couldn't download yt-dlp: {e}"
                ))
                .size(theme::SIZE_S)
                .style(theme::error_text)
                .width(Length::Fill),
                button(text(tr!("Reintentar", "Retry")).size(theme::SIZE_S))
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

    /// Aviso si falta deno, con el botón para instalarlo.
    fn view_deno_banner(&self) -> Option<Element<'_, Message>> {
        use crate::DenoState;
        let hint = || {
            text(tr!(
                "Falta deno: yt-dlp lo necesita para que YouTube no rechace las canciones.",
                "deno is missing: yt-dlp needs it so YouTube doesn't reject songs."
            ))
            .size(theme::SIZE_S)
            .width(Length::Fill)
        };
        let install = |label: &'static str| {
            button(text(label).size(theme::SIZE_S))
                .on_press(Message::DownloadDeno)
                .padding([6, 14])
                .style(theme::primary_button)
        };
        Some(match &self.deno {
            DenoState::Ok => return None,
            DenoState::Missing => row![hint(), install(tr!("Instalar deno", "Install deno"))]
                .spacing(12)
                .align_y(Alignment::Center)
                .into(),
            DenoState::Downloading => text(tr!(
                "Descargando deno (unos 40 MB)…",
                "Downloading deno (about 40 MB)…"
            ))
            .size(theme::SIZE_S)
            .into(),
            DenoState::Failed(e) => row![
                text(trf!(
                    "No se pudo instalar deno: {e}",
                    "Couldn't install deno: {e}"
                ))
                .size(theme::SIZE_S)
                .style(theme::error_text)
                .width(Length::Fill),
                install(tr!("Reintentar", "Retry")),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        })
    }
}

/// Título de una sección. En Brutal va en negrita, y el primero subrayado
/// con el acento.
pub fn heading<'a>(label: &'a str, first: bool) -> Element<'a, Message> {
    let c = theme::colors();
    if c.look != theme::Look::Brutal {
        return text(label).size(theme::SIZE_M + 1).into();
    }
    if !first {
        return text(label).size(theme::SIZE_L + 4).font(theme::BOLD).into();
    }
    column![
        text(label).size(theme::SIZE_M + 3).font(theme::BOLD),
        // Ancho aproximado del texto (la regla no se puede medir contra él).
        container(Space::new())
            .width(label.chars().count() as f32 * 10.5)
            .height(4)
            .style(theme::rule(c.accent)),
    ]
    .spacing(0)
    .into()
}
