//! Pantalla de inicio: buscador, playlists (las escuchadas hace poco
//! primero) y canciones escuchadas hace poco.

use std::collections::HashMap;

use iced::widget::{
    button, column, container, image, operation, responsive, row, scrollable, text, text_input,
    Column, Id, Space,
};
use iced::{Alignment, Element, Length, Task};
use surco_core::{format_duration, Track};
use surco_resolver::{cache_key, Match};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::playlists::Recs;
use crate::search::{SearchMsg, SEARCH_INPUT};
use crate::{theme, Message, Screen, Surco, YtDlpState};

/// Tarjetas de playlists en el inicio.
const TILES: usize = 8;
/// Canciones escuchadas hace poco que se muestran.
const RECENT_SONGS: usize = 8;
/// Historial que se lee para armar esa lista (con repetidas).
const HISTORY_READ: usize = 400;
/// Cuántas "más escuchadas" se juntan.
const TOP_SONGS: usize = 25;
const TILE_WIDTH: f32 = 270.0;
const TILE_COVER: f32 = 52.0;
/// Recomendadas que se muestran en el inicio.
const HOME_RECS: usize = 5;
/// Ancho de la columna derecha (sonando ahora y recomendadas).
const SIDE: f32 = 320.0;
/// Desde este ancho el inicio va en dos columnas.
const TWO_COLUMNS: f32 = 940.0;

/// Lo que muestra el inicio (se recalcula al entrar).
#[derive(Debug, Default)]
pub struct HomeState {
    /// IDs de playlists escuchadas hace poco, la más reciente primero.
    pub recent_playlists: Vec<String>,
    /// Canciones escuchadas hace poco (sin repetir) y su video.
    pub recent_songs: Vec<(Track, Option<String>)>,
    /// Las que más sonaron en el historial reciente (dos veces o más), la
    /// más escuchada primero, con su video.
    pub top_songs: Vec<(Track, Option<String>)>,
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
        // Más escuchadas: cuántas veces sonó cada una (y en qué orden
        // apareció, para desempatar por la más reciente).
        let mut counts: HashMap<String, (usize, usize, Track, Option<String>)> = HashMap::new();
        for (order, p) in played.iter().enumerate() {
            let track = p.track();
            let e =
                counts
                    .entry(cache_key(&track))
                    .or_insert((0, order, track, p.youtube_id.clone()));
            e.0 += 1;
        }
        let mut top: Vec<_> = counts.into_values().filter(|e| e.0 >= 2).collect();
        top.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.home.top_songs = top
            .into_iter()
            .take(TOP_SONGS)
            .map(|(_, _, t, v)| (t, v))
            .collect();
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
            self.home
                .top_songs
                .iter()
                .take(1)
                .filter_map(|(_, v)| v.clone()),
        );
        if let Some(Recs::Ready(list)) = self.home_recs().map(|(_, r)| r) {
            videos.extend(list.iter().take(HOME_RECS).map(|r| r.id.clone()));
        }
        // Portadas de las playlists (inicio y barra lateral).
        videos.extend(
            (0..self.playlists.len()).filter_map(|i| self.playlist_cover(i).map(str::to_owned)),
        );
        let mut tasks: Vec<Task<Message>> = videos.iter().map(|v| self.queue_cover(v)).collect();
        // Recomendadas de la última playlist escuchada (columna derecha).
        if let Some(&p) = self.home_playlists().first() {
            tasks.push(self.load_recs(p, false));
        }
        Task::batch(tasks)
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

    /// Recomendadas de la playlist escuchada más recientemente (índice y
    /// estado).
    fn home_recs(&self) -> Option<(usize, &Recs)> {
        let p = *self.home_playlists().first()?;
        let (playlist, _) = self.playlists.get(p)?;
        Some((p, self.recs.get(&playlist.id)?))
    }

    /// "Escuchar ahora" de las más escuchadas.
    pub(crate) fn play_top(&mut self) -> Task<Message> {
        let tracks: Vec<Track> = self.home.top_songs.iter().map(|(t, _)| t.clone()).collect();
        if tracks.is_empty() {
            return Task::none();
        }
        self.play_list(tracks, 0, true)
    }

    /// Escuchar una canción de "Escuchadas hace poco" (sigue con parecidas).
    pub(crate) fn play_recent(&mut self, i: usize) -> Task<Message> {
        let Some((track, _)) = self.home.recent_songs.get(i).cloned() else {
            return Task::none();
        };
        self.play_list(vec![track], 0, true)
    }

    pub(crate) fn view_home(&self) -> Element<'_, Message> {
        responsive(move |size| {
            let two = size.width >= TWO_COLUMNS;
            let mut main = Column::new().spacing(22);
            if let Some(b) = self.view_top_banner() {
                main = main.push(b);
            }
            if !self.home.recent_songs.is_empty() {
                main = main.push(
                    column![
                        heading(tr!("Canciones que escuchaste", "Songs you played"), false),
                        self.recent_table(),
                    ]
                    .spacing(12),
                );
            }
            let body: Element<'_, Message> = match self.home_side() {
                Some(side) if two => row![main.width(Length::Fill), side.width(SIDE)]
                    .spacing(24)
                    .into(),
                Some(side) => main.push(side).into(),
                None => main.into(),
            };
            crate::smooth::smooth(
                scrollable(
                    column![self.home_top(), body]
                        .spacing(22)
                        .padding(iced::Padding::ZERO.right(16)),
                )
                .height(Length::Fill),
            )
        })
        .into()
    }

    /// Saludo, buscador, avisos y playlists.
    fn home_top(&self) -> Column<'_, Message> {
        let search = container(
            row![
                icons::icon(Kind::Search, theme::colors().text_secondary, 18.0),
                text_input(
                    tr!("¿Qué querés escuchar?", "What do you want to listen to?"),
                    ""
                )
                .on_input(Message::HomeSearch)
                .padding(10)
                .size(theme::SIZE_M)
                .style(theme::input),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .max_width(420);
        let header = row![
            column![
                text(crate::greeting::now())
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                text(tr!("Tu música", "Your music")).size(theme::SIZE_L + 6),
            ]
            .spacing(2)
            .width(Length::Fill),
            search,
        ]
        .spacing(24)
        .align_y(Alignment::Center);

        let mut top = column![header].spacing(22);
        if let Some(banner) = self.view_ytdlp_banner() {
            top = top.push(banner);
        }

        // Playlists.
        if self.playlists.is_empty() {
            top = top.push(
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
            top = top.push(
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

        top
    }

    fn tile(&self, i: usize) -> Element<'_, Message> {
        let Some((p, source)) = self.playlists.get(i) else {
            return Space::new().into();
        };
        let cover: Element<'_, Message> =
            match self.playlist_cover(i).and_then(|v| self.covers.get(v)) {
                _ if theme::minimal() => Space::new().width(0).into(),
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
                    row![detail, icons::waves_if(this_playing)]
                        .spacing(8)
                        .align_y(Alignment::Center),
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
        .style(move |th, st| {
            let mut s = theme::tile(th, st);
            // La playlist que suena, marcada con el acento.
            if this_playing {
                s.border = s.border.width(1.5).color(theme::colors().accent);
            }
            s
        })
        .into()
    }

    /// Encabezado de la tabla de canciones (# Título Artista Duración).
    fn table_header<'a>() -> Element<'a, Message> {
        let h = |s: &'static str| text(s).size(theme::SIZE_S).style(theme::secondary_text);
        column![
            row![
                h("#").width(28),
                h(tr!("Título", "Title")).width(Length::FillPortion(3)),
                h(tr!("Artista", "Artist")).width(Length::FillPortion(2)),
                Space::new().width(28),
                h(tr!("Duración", "Length")).width(56),
            ]
            .spacing(12)
            .padding([0, 8]),
            container(Space::new())
                .width(Length::Fill)
                .height(1)
                .style(theme::rule(theme::colors().border)),
        ]
        .spacing(8)
        .into()
    }

    /// Canciones escuchadas hace poco, en tabla.
    fn recent_table(&self) -> Element<'_, Message> {
        let rows = self
            .home
            .recent_songs
            .iter()
            .enumerate()
            .map(|(i, (t, v))| self.table_row(i, t, v.as_deref()));
        column![
            Self::table_header(),
            Column::with_children(rows).spacing(theme::row_gap())
        ]
        .spacing(6)
        .into()
    }

    fn table_row<'a>(
        &'a self,
        i: usize,
        t: &'a Track,
        video: Option<&'a str>,
    ) -> Element<'a, Message> {
        let playing = self
            .now_track()
            .is_some_and(|n| cache_key(n) == cache_key(t));
        let number: Element<'_, Message> = if playing {
            icons::waves()
        } else {
            text((i + 1).to_string())
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into()
        };
        let duration = if t.duration_ms > 0 {
            format_duration(t.duration_ms)
        } else {
            String::new()
        };
        let row = button(
            row![
                container(number).width(28).clip(true),
                row![
                    cover(self, video, 36.0),
                    crate::marquee::marquee(
                        text(&t.title)
                            .size(theme::SIZE_M)
                            .wrapping(text::Wrapping::None)
                            .style(if playing {
                                theme::accent_text
                            } else {
                                theme::plain_text
                            }),
                        self.is_hovered(t),
                    ),
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(3))
                .clip(true),
                container(crate::marquee::marquee(
                    text(t.artists.join(", "))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .wrapping(text::Wrapping::None),
                    self.is_hovered(t),
                ))
                .width(Length::FillPortion(2))
                .clip(true),
                container(crate::search::add_button(t, None)).width(28),
                text(duration)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .width(56),
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

    /// "Tus más escuchadas": la más escuchada y un botón para escucharlas.
    fn view_top_banner(&self) -> Option<Element<'_, Message>> {
        let (first, video) = self.home.top_songs.first()?;
        let n = self.home.top_songs.len();
        let count = crate::view::songs(n);
        Some(
            container(
                row![
                    column![
                        text(tr!("Tu resumen", "Your summary"))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        text(tr!("Tus más escuchadas", "Your most played")).size(theme::SIZE_L + 2),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                    cover(self, video.as_deref(), 52.0),
                    column![
                        text(&first.title)
                            .size(theme::SIZE_M)
                            .wrapping(text::Wrapping::None),
                        text(count).size(theme::SIZE_S).style(theme::secondary_text),
                    ]
                    .spacing(4)
                    .width(Length::Fixed(200.0))
                    .clip(true),
                    button(text(tr!("Escuchar ahora", "Play now")).size(theme::SIZE_S))
                        .on_press(Message::PlayTop)
                        .padding([8, 18])
                        .style(theme::primary_button),
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            )
            .padding([18, 20])
            .width(Length::Fill)
            .style(theme::highlight_card)
            .into(),
        )
    }

    /// Columna derecha: lo que suena (con la portada grande) y recomendadas.
    fn home_side(&self) -> Option<Column<'_, Message>> {
        let mut col = Column::new().spacing(22);
        let mut any = false;
        if let Some(t) = self.now_track() {
            let video = self.video_of_track(t);
            col = col.push(
                container(
                    column![
                        big_cover(self, video, SIDE - 32.0),
                        column![
                            crate::marquee::marquee(
                                text(&t.title)
                                    .size(theme::SIZE_M + 1)
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
                        .clip(true),
                    ]
                    .spacing(12),
                )
                .padding(16)
                .width(Length::Fill)
                .style(theme::card),
            );
            any = true;
        }
        if let Some((p, Recs::Ready(list))) = self.home_recs() {
            if !list.is_empty() {
                let name = self
                    .playlists
                    .get(p)
                    .map(|(pl, _)| pl.name.as_str())
                    .unwrap_or("");
                let rows = list.iter().take(HOME_RECS).enumerate().map(|(i, r)| {
                    button(
                        row![
                            cover(self, Some(r.id.as_str()), 40.0),
                            column![
                                crate::marquee::marquee(
                                    text(r.title.clone())
                                        .size(theme::SIZE_S + 1)
                                        .wrapping(text::Wrapping::None),
                                    false,
                                ),
                                text(r.channel.clone().unwrap_or_default())
                                    .size(theme::SIZE_S)
                                    .style(theme::secondary_text)
                                    .wrapping(text::Wrapping::None),
                            ]
                            .spacing(2)
                            .width(Length::Fill)
                            .clip(true),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .on_press(Message::RecPlay(p, i))
                    .padding([4, 6])
                    .width(Length::Fill)
                    .style(theme::song_row(false))
                    .into()
                });
                col = col.push(
                    column![
                        column![
                            heading(tr!("Recomendado para vos", "Recommended for you"), false),
                            text(trf!("Basado en «{}»", "Based on “{}”", name))
                                .size(theme::SIZE_S)
                                .style(theme::secondary_text),
                        ]
                        .spacing(2),
                        Column::with_children(rows).spacing(2),
                    ]
                    .spacing(10),
                );
                any = true;
            }
        }
        any.then_some(col)
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

/// Portada de un video (o un cuadro vacío mientras no está).
pub fn cover<'a>(app: &'a Surco, video: Option<&str>, size: f32) -> Element<'a, Message> {
    if theme::minimal() {
        return Space::new().width(0).height(0).into();
    }
    match video.and_then(|v| app.covers.get(v)) {
        Some(h) => image(h.clone())
            .width(size)
            .height(size)
            .border_radius(if size > 100.0 { 8.0 } else { 4.0 })
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => container(Space::new())
            .width(size)
            .height(size)
            .style(theme::cover_placeholder)
            .into(),
    }
}

/// Como [`cover`], pero con la portada grande si ya está (si no, la chica).
pub fn big_cover<'a>(app: &'a Surco, video: Option<&str>, size: f32) -> Element<'a, Message> {
    if theme::minimal() {
        return Space::new().width(0).height(0).into();
    }
    match video.and_then(|v| app.large_covers.get(v)) {
        Some(h) => image(h.clone())
            .width(size)
            .height(size)
            .border_radius(8.0)
            .into(),
        None => cover(app, video, size),
    }
}
