//! Buscador (canciones, artistas y playlists de YouTube) e importación de
//! playlists de YouTube / YouTube Music.
//!
//! Lo que se escucha desde el buscador va a una cola "sin fin": cuando se
//! acerca al final se le agregan canciones parecidas (ver `radio.rs`).

use iced::widget::{
    button, column, container, operation, row, scrollable, text, text_input, Column, Id, Space,
};
use iced::{Alignment, Element, Length, Task};
use surco_core::{format_duration, Playlist, Track};
use surco_provider::{Listing, SearchResult};
use surco_resolver::{cache_key, Match};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::radio::track_from;
use crate::{theme, Message, Notice, Screen, Surco, RUNTIME};

pub const SEARCH_INPUT: &str = "search-input";
const SONGS: usize = 15;
const ARTISTS: usize = 6;
const PLAYLISTS: usize = 8;
/// Canciones de un artista que se muestran.
const ARTIST_SONGS: usize = 20;
/// Tope de canciones al importar una playlist de YouTube.
const PLAYLIST_MAX: usize = 2000;
/// Espera desde la última tecla antes de buscar solo.
const TYPING_PAUSE: std::time::Duration = std::time::Duration::from_millis(450);
/// Letras mínimas para buscar mientras se escribe.
const MIN_QUERY: usize = 2;

/// Resultados de una búsqueda. Cada parte puede fallar por separado.
#[derive(Debug, Clone, Default)]
pub struct Results {
    pub songs: Vec<SearchResult>,
    pub artists: Vec<Listing>,
    pub playlists: Vec<Listing>,
    pub errors: Vec<String>,
}

/// Lista abierta desde los resultados (un artista o una playlist).
#[derive(Debug, Clone)]
pub struct Preview {
    pub title: String,
    pub subtitle: String,
    /// Link de la playlist (para guardarla en la biblioteca); `None` si es un artista.
    pub url: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Default)]
pub struct SearchState {
    pub query: String,
    /// Cuenta las teclas: la búsqueda automática solo corre si no hubo otra después.
    pub typed: u64,
    /// Búsqueda en curso (texto).
    pub searching: Option<String>,
    pub results: Option<(String, Results)>,
    pub opening: bool,
    pub preview: Option<Preview>,
}

#[derive(Debug, Clone)]
pub enum SearchMsg {
    Open,
    Input(String),
    /// Pasó un rato sin escribir (número de tecla).
    Paused(u64),
    Submit,
    Done(String, Results),
    /// Reproducir la canción `i` de los resultados (cola sin fin).
    PlaySong(usize),
    OpenArtist(usize),
    OpenPlaylist(usize),
    Opened(Result<(Preview, Vec<SearchResult>), String>),
    ClosePreview,
    /// Reproducir la lista abierta desde la canción `i`.
    PlayPreview(usize),
    SavePreview,
    /// Llegó una playlist importada desde un link de YouTube.
    Imported(Result<(String, String, Vec<SearchResult>), String>),
}

/// Si es un link a una playlist de YouTube o YouTube Music, su ID (`list=`).
pub fn youtube_playlist_id(input: &str) -> Option<String> {
    let s = input.trim();
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let host = rest.split(['/', '?']).next()?.to_ascii_lowercase();
    let host_ok = host == "youtu.be" || host == "youtube.com" || host.ends_with(".youtube.com");
    if !host_ok {
        return None;
    }
    let query = s.split_once('?')?.1;
    query
        .split(['&', '#'])
        .find_map(|kv| kv.strip_prefix("list="))
        .filter(|id| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(str::to_owned)
}

/// "Soda Stereo - Topic", "SodaStereoVEVO" y "Soda Stereo" son el mismo artista.
fn artist_name(l: &Listing) -> String {
    crate::radio::clean_channel(&l.title)
}

fn dedup_artists(list: Vec<Listing>) -> Vec<Listing> {
    let mut seen = std::collections::HashSet::new();
    list.into_iter()
        .filter(|l| {
            let key: String = artist_name(l)
                .chars()
                .filter(|c| !c.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect();
            seen.insert(key)
        })
        .collect()
}

/// Canciones "de verdad": ni shorts ni videos de una hora.
fn song_like(r: &SearchResult) -> bool {
    r.duration
        .is_none_or(|d| (60..=15 * 60).contains(&d.as_secs()))
}

impl Surco {
    pub(crate) fn on_search(&mut self, msg: SearchMsg) -> Task<Message> {
        match msg {
            SearchMsg::Open => {
                self.screen = Screen::Search;
                return operation::focus(Id::new(SEARCH_INPUT));
            }
            SearchMsg::Input(q) => {
                self.search.query = q;
                self.search.typed += 1;
                let n = self.search.typed;
                let Some(rt) = RUNTIME.get() else {
                    return Task::none();
                };
                let wait = rt.spawn(tokio::time::sleep(TYPING_PAUSE));
                return Task::perform(
                    async move {
                        let _ = wait.await;
                    },
                    move |()| Message::Search(SearchMsg::Paused(n)),
                );
            }
            SearchMsg::Paused(n) => {
                let q = self.search.query.trim();
                let already = self.search.searching.as_deref() == Some(q)
                    || self.search.results.as_ref().is_some_and(|(r, _)| r == q);
                if n == self.search.typed && q.chars().count() >= MIN_QUERY && !already {
                    return self.run_search();
                }
            }
            SearchMsg::Submit => return self.run_search(),
            SearchMsg::Done(q, results) => {
                if self.search.searching.as_deref() == Some(q.as_str()) {
                    self.search.searching = None;
                }
                if q == self.search.query.trim() {
                    let covers: Vec<String> = results.songs.iter().map(|r| r.id.clone()).collect();
                    self.search.results = Some((q, results));
                    return Task::batch(
                        covers
                            .iter()
                            .map(|v| self.queue_cover(v))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            SearchMsg::PlaySong(i) => {
                let Some(r) = self
                    .search
                    .results
                    .as_ref()
                    .and_then(|(_, res)| res.songs.get(i))
                    .cloned()
                else {
                    return Task::none();
                };
                let track = self.adopt(&r);
                return self.play_list(vec![track], 0, true);
            }
            SearchMsg::OpenArtist(i) => {
                let Some(a) = self
                    .search
                    .results
                    .as_ref()
                    .and_then(|(_, res)| res.artists.get(i))
                else {
                    return Task::none();
                };
                let name = artist_name(a);
                return self.open_preview(move |y| async move {
                    let songs = y
                        .search(&name, ARTIST_SONGS)
                        .await
                        .map_err(|e| e.to_string())?;
                    let tracks: Vec<Track> = songs
                        .iter()
                        .filter(|r| song_like(r))
                        .map(track_from)
                        .collect();
                    Ok((
                        Preview {
                            title: name.clone(),
                            subtitle: tr!(
                                "Canciones más escuchadas en YouTube",
                                "Most played songs on YouTube"
                            )
                            .into(),
                            url: None,
                            tracks,
                        },
                        songs,
                    ))
                });
            }
            SearchMsg::OpenPlaylist(i) => {
                let Some(p) = self
                    .search
                    .results
                    .as_ref()
                    .and_then(|(_, res)| res.playlists.get(i))
                    .cloned()
                else {
                    return Task::none();
                };
                return self.open_preview(move |y| async move {
                    let (title, songs) = y
                        .playlist(&p.url, PLAYLIST_MAX)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok((
                        Preview {
                            title,
                            subtitle: trf!(
                                "Playlist de YouTube{}",
                                "YouTube playlist{}",
                                p.channel.map(|c| format!(" · {c}")).unwrap_or_default()
                            ),
                            url: Some(p.url),
                            tracks: songs.iter().map(track_from).collect(),
                        },
                        songs,
                    ))
                });
            }
            SearchMsg::Opened(result) => {
                self.search.opening = false;
                match result {
                    Ok((p, songs)) => {
                        for r in &songs {
                            self.adopt(r);
                        }
                        let covers: Vec<String> = p
                            .tracks
                            .iter()
                            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
                            .collect();
                        self.search.preview = Some(p);
                        return Task::batch(
                            covers
                                .iter()
                                .map(|v| self.queue_cover(v))
                                .collect::<Vec<_>>(),
                        );
                    }
                    Err(e) => {
                        self.notice = Some(Notice {
                            error: true,
                            text: trf!("No se pudo abrir: {e}", "Couldn't open it: {e}"),
                        })
                    }
                }
            }
            SearchMsg::ClosePreview => self.search.preview = None,
            SearchMsg::PlayPreview(i) => {
                if let Some(p) = &self.search.preview {
                    let tracks = p.tracks.clone();
                    return self.play_list(tracks, i, true);
                }
            }
            SearchMsg::SavePreview => {
                if let Some(p) = self.search.preview.clone() {
                    let id = p
                        .url
                        .as_deref()
                        .and_then(youtube_playlist_id)
                        .unwrap_or_else(|| p.title.clone());
                    self.save_youtube_playlist(&id, &p.title, p.tracks);
                    return self.on_playlist_opened();
                }
            }
            SearchMsg::Imported(result) => {
                self.importing = false;
                match result {
                    Ok((id, title, songs)) => {
                        if songs.is_empty() {
                            self.notice = Some(Notice {
                                error: true,
                                text: tr!(
                                    "Esa playlist de YouTube no tiene videos disponibles.",
                                    "That YouTube playlist has no available videos."
                                )
                                .into(),
                            });
                            return Task::none();
                        }
                        let tracks = songs.iter().map(|r| self.adopt(r)).collect();
                        self.import_input.clear();
                        self.notice = None;
                        self.save_youtube_playlist(&id, &title, tracks);
                        return self.on_playlist_opened();
                    }
                    Err(e) => {
                        self.notice = Some(Notice {
                            error: true,
                            text: trf!(
                                "No se pudo importar la playlist de YouTube: {e}",
                                "Couldn't import the YouTube playlist: {e}"
                            ),
                        })
                    }
                }
            }
        }
        Task::none()
    }

    /// Un video como canción ya resuelta (sin buscarla de nuevo).
    pub(crate) fn adopt(&mut self, r: &SearchResult) -> Track {
        let track = track_from(r);
        let key = cache_key(&track);
        if !matches!(self.resolutions.get(&key), Some(TrackRes::Done(_))) {
            let m = Match {
                youtube_id: r.id.clone(),
                score: 100,
                manual: true,
                candidates: Vec::new(),
            };
            self.persist_match(&key, &m);
            self.resolutions.insert(key, TrackRes::Done(m));
        }
        track
    }

    fn run_search(&mut self) -> Task<Message> {
        let q = self.search.query.trim().to_owned();
        if q.is_empty() {
            return Task::none();
        }
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            self.notice = Some(Notice {
                error: true,
                text: tr!(
                    "Falta yt-dlp para buscar en YouTube.",
                    "yt-dlp is missing, so YouTube can't be searched."
                )
                .into(),
            });
            return Task::none();
        };
        self.search.searching = Some(q.clone());
        self.search.preview = None;
        let query = q.clone();
        let job = rt.spawn(async move {
            let y = provider.ytdlp();
            let (songs, artists, playlists) = tokio::join!(
                y.search(&query, SONGS),
                y.search_channels(&query, ARTISTS),
                y.search_playlists(&query, PLAYLISTS)
            );
            let mut r = Results::default();
            match songs {
                Ok(s) => r.songs = s.into_iter().filter(song_like).collect(),
                Err(e) => r.errors.push(e.to_string()),
            }
            match artists {
                Ok(a) => r.artists = dedup_artists(a),
                Err(e) => r.errors.push(e.to_string()),
            }
            match playlists {
                Ok(p) => r.playlists = p,
                Err(e) => r.errors.push(e.to_string()),
            }
            r
        });
        Task::perform(
            async move {
                job.await.unwrap_or_else(|e| Results {
                    errors: vec![e.to_string()],
                    ..Results::default()
                })
            },
            move |r| Message::Search(SearchMsg::Done(q.clone(), r)),
        )
    }

    /// Abre un artista o una playlist de los resultados.
    fn open_preview<F, Fut>(&mut self, load: F) -> Task<Message>
    where
        F: FnOnce(surco_provider::YtDlp) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(Preview, Vec<SearchResult>), String>>
            + Send
            + 'static,
    {
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            return Task::none();
        };
        self.search.opening = true;
        let y = provider.ytdlp().clone();
        let job = rt.spawn(load(y));
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            |r| Message::Search(SearchMsg::Opened(r)),
        )
    }

    /// Importar desde un link de playlist de YouTube / YouTube Music.
    pub(crate) fn import_youtube(&mut self, url: String, id: String) -> Task<Message> {
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            self.notice = Some(Notice {
                error: true,
                text: tr!(
                    "Falta yt-dlp para importar playlists de YouTube.",
                    "yt-dlp is missing, so YouTube playlists can't be imported."
                )
                .into(),
            });
            return Task::none();
        };
        self.importing = true;
        self.notice = None;
        let job = rt.spawn(async move {
            provider
                .ytdlp()
                .playlist(&url, PLAYLIST_MAX)
                .await
                .map(|(title, songs)| (id, title, songs))
                .map_err(|e| e.to_string())
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            |r| Message::Search(SearchMsg::Imported(r)),
        )
    }

    /// Guarda una playlist de YouTube en la biblioteca y la abre.
    fn save_youtube_playlist(&mut self, list_id: &str, title: &str, tracks: Vec<Track>) {
        let playlist = Playlist {
            id: format!("yt:{list_id}"),
            name: title.to_owned(),
            tracks,
        };
        // Las canciones de la vista previa todavía no tienen el match guardado.
        for t in &playlist.tracks {
            if let Some(TrackRes::Done(m)) = self.resolutions.get(&cache_key(t)) {
                let m = m.clone();
                self.persist_match(&cache_key(t), &m);
            }
        }
        self.persist_playlist(&playlist, "YouTube");
        let index = match self.playlists.iter().position(|(p, _)| p.id == playlist.id) {
            Some(i) => {
                self.playlists[i] = (playlist, "YouTube".into());
                if let Some(a) = self.active.as_mut().filter(|a| a.origin == Some(i)) {
                    a.origin = None;
                }
                i
            }
            None => {
                self.playlists.push((playlist, "YouTube".into()));
                self.playlists.len() - 1
            }
        };
        self.recs.remove(&self.playlists[index].0.id);
        self.open_playlist(index);
    }

    pub(crate) fn view_search(&self) -> Element<'_, Message> {
        let input = row![
            icons::icon(Kind::Search, theme::colors().text_secondary, 18.0),
            text_input(
                tr!(
                    "Buscá canciones, artistas o playlists",
                    "Search songs, artists or playlists"
                ),
                &self.search.query
            )
            .id(Id::new(SEARCH_INPUT))
            .on_input(|q| Message::Search(SearchMsg::Input(q)))
            .on_submit(Message::Search(SearchMsg::Submit))
            .padding(10)
            .size(theme::SIZE_M)
            .style(theme::input),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let body: Element<'_, Message> = if let Some(p) = &self.search.preview {
            self.view_preview(p)
        } else if self.search.opening {
            column![
                crate::skeleton::header(self.pulse),
                crate::skeleton::song_rows(8, self.pulse)
            ]
            .spacing(20)
            .into()
        } else if self
            .search
            .searching
            .as_deref()
            .is_some_and(|q| self.search.results.as_ref().is_none_or(|(r, _)| r != q))
        {
            crate::skeleton::song_rows(8, self.pulse)
        } else if let Some((q, r)) = &self.search.results {
            self.view_results(q, r)
        } else {
            text(tr!("Escribí y los resultados aparecen solos. Lo que escuches desde acá no se corta: cuando se termina, siguen canciones parecidas.", "Type and results show up on their own. What you play from here doesn't stop: when it ends, similar songs follow."))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into()
        };
        column![input, body].spacing(20).into()
    }

    fn song_row<'a>(
        &'a self,
        track: Track,
        video: Option<&'a str>,
        on_play: Message,
    ) -> Element<'a, Message> {
        let cover: Element<'_, Message> = match video.and_then(|v| self.covers.get(v)) {
            Some(h) => iced::widget::image(h.clone())
                .width(40)
                .height(40)
                .border_radius(4.0)
                .into(),
            None => container(Space::new())
                .width(40)
                .height(40)
                .style(theme::cover_placeholder)
                .into(),
        };
        let playing =
            video.is_some_and(|v| self.now_track().and_then(|t| self.video_of_track(t)) == Some(v));
        let add = add_button(&track, None);
        let menu = song_menu(&track, None);
        let row = button(
            row![
                cover,
                column![
                    text(track.title.clone())
                        .size(theme::SIZE_M)
                        .wrapping(text::Wrapping::None),
                    text(track.artists.join(", "))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(2)
                .width(Length::Fill)
                .clip(true),
                add,
                text(format_duration(track.duration_ms))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(on_play)
        .padding([4, 8])
        .width(Length::Fill)
        .style(theme::list_button(playing));
        iced::widget::mouse_area(row).on_right_press(menu).into()
    }

    fn view_results<'a>(&'a self, q: &'a str, r: &'a Results) -> Element<'a, Message> {
        let mut col = Column::new().spacing(22);
        if r.songs.is_empty() && r.artists.is_empty() && r.playlists.is_empty() {
            let msg = match r.errors.first() {
                Some(e) => trf!("No se pudo buscar: {e}", "Search failed: {e}"),
                None => trf!("Nada para «{q}».", "Nothing for “{q}”."),
            };
            return text(msg)
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into();
        }
        if !r.artists.is_empty() {
            let chips = row(r.artists.iter().enumerate().map(|(i, a)| {
                button(text(artist_name(a)).size(theme::SIZE_S))
                    .on_press(Message::Search(SearchMsg::OpenArtist(i)))
                    .padding([6, 12])
                    .style(theme::ghost_button)
                    .into()
            }))
            .spacing(8)
            .wrap();
            col = col.push(column![section(tr!("Artistas", "Artists")), chips].spacing(10));
        }
        if !r.songs.is_empty() {
            let list = Column::with_children(r.songs.iter().enumerate().map(|(i, s)| {
                self.song_row(
                    track_from(s),
                    Some(s.id.as_str()),
                    Message::Search(SearchMsg::PlaySong(i)),
                )
            }))
            .spacing(2);
            col = col.push(column![section(tr!("Canciones", "Songs")), list].spacing(10));
        }
        if !r.playlists.is_empty() {
            let list = Column::with_children(r.playlists.iter().enumerate().map(|(i, p)| {
                button(
                    column![
                        text(&p.title)
                            .size(theme::SIZE_M)
                            .wrapping(text::Wrapping::None),
                        text(p.channel.as_deref().unwrap_or("Playlist"))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                    ]
                    .spacing(2),
                )
                .on_press(Message::Search(SearchMsg::OpenPlaylist(i)))
                .padding([6, 8])
                .width(Length::Fill)
                .style(theme::list_button(false))
                .into()
            }))
            .spacing(2);
            col = col.push(column![section("Playlists"), list].spacing(10));
        }
        scrollable(col.padding(iced::Padding::ZERO.right(16)))
            .height(Length::Fill)
            .into()
    }

    fn view_preview<'a>(&'a self, p: &'a Preview) -> Element<'a, Message> {
        let mut actions = row![
            button(text(tr!("← Resultados", "← Results")).size(theme::SIZE_S))
                .on_press(Message::Search(SearchMsg::ClosePreview))
                .padding([6, 10])
                .style(theme::ghost_button),
            button(text(tr!("Reproducir", "Play")).size(theme::SIZE_S))
                .on_press_maybe(
                    (!p.tracks.is_empty()).then_some(Message::Search(SearchMsg::PlayPreview(0)))
                )
                .padding([6, 14])
                .style(theme::primary_button),
        ]
        .spacing(8);
        if p.url.is_some() {
            actions = actions.push(
                button(
                    text(tr!("Guardar en la biblioteca", "Save to library")).size(theme::SIZE_S),
                )
                .on_press(Message::Search(SearchMsg::SavePreview))
                .padding([6, 10])
                .style(theme::ghost_button),
            );
        }
        let list = Column::with_children(p.tracks.iter().enumerate().map(|(i, t)| {
            self.song_row(
                t.clone(),
                self.video_of_track(t),
                Message::Search(SearchMsg::PlayPreview(i)),
            )
        }))
        .spacing(2);
        column![
            column![
                text(&p.title).size(theme::SIZE_L),
                text(trf!(
                    "{} · {} canciones",
                    "{} · {} songs",
                    p.subtitle,
                    p.tracks.len()
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            ]
            .spacing(4),
            actions,
            scrollable(list.padding(iced::Padding::ZERO.right(16))).height(Length::Fill),
        ]
        .spacing(14)
        .into()
    }
}

fn section(title: &str) -> Element<'_, Message> {
    text(title).size(theme::SIZE_M).into()
}

/// Mensaje que abre el menú "Agregar a…" de una canción.
pub fn song_menu(track: &Track, from: Option<(usize, usize)>) -> Message {
    Message::SongMenu(Some(Box::new(crate::playlists::SongMenu {
        track: track.clone(),
        from,
    })))
}

/// Una fila de canción que también abre su menú con clic derecho.
pub fn with_menu<'a>(
    row: impl Into<Element<'a, Message>>,
    track: &Track,
    from: Option<(usize, usize)>,
) -> Element<'a, Message> {
    iced::widget::mouse_area(row)
        .on_right_press(song_menu(track, from))
        .into()
}

/// Botón "+" de una canción: abre el menú para agregarla a la cola o a una
/// playlist (`from`: playlist y posición, para poder quitarla de ahí).
pub fn add_button<'a>(track: &Track, from: Option<(usize, usize)>) -> Element<'a, Message> {
    iced::widget::tooltip(
        button(icons::glyph(
            Kind::Add,
            theme::colors().text_secondary,
            14.0,
        ))
        .on_press(song_menu(track, from))
        .padding(6)
        .style(theme::icon_button),
        container(text(tr!("Agregar a…", "Add to…")).size(theme::SIZE_S))
            .padding(6)
            .style(theme::card),
        iced::widget::tooltip::Position::Left,
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_youtube_playlist_links() {
        for (url, id) in [
            (
                "https://www.youtube.com/playlist?list=PLMoz13LLcTU-x",
                Some("PLMoz13LLcTU-x"),
            ),
            (
                "https://music.youtube.com/playlist?list=OLAK5uy_abc&si=xyz",
                Some("OLAK5uy_abc"),
            ),
            (
                "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ",
                Some("RDdQw4w9WgXcQ"),
            ),
            ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", None),
            (
                "https://open.spotify.com/playlist/37i9dQZF1DX4UtSsGT1Sbe",
                None,
            ),
            ("https://evil.com/?list=x&youtube.com/", None),
        ] {
            assert_eq!(youtube_playlist_id(url).as_deref(), id, "{url}");
        }
    }

    #[test]
    fn merges_topic_channels() {
        let l = |t: &str| Listing {
            id: t.into(),
            title: t.into(),
            url: String::new(),
            channel: None,
            followers: None,
        };
        let out = dedup_artists(vec![
            l("Soda Stereo"),
            l("Soda Stereo - Topic"),
            l("SodaStereoVEVO"),
            l("Cerati"),
        ]);
        assert_eq!(out.len(), 2);
    }
}
