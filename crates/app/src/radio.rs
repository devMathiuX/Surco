//! Radio: canciones parecidas a la que suena, sacadas del "Mix" de YouTube.
//!
//! Se arma una playlist "Radio" con esos videos (ya resueltos, sin buscar)
//! y se reproduce como cualquier otra. Con "seguir con canciones parecidas"
//! prendido, al terminar la cola se arma sola a partir de la última canción.

use iced::Task;
use surco_core::{PlayerState, Playlist, RepeatMode, Track};
use surco_player::Source;
use surco_provider::SearchResult;
use surco_resolver::{cache_key, Match};

use crate::playback::{Active, TrackRes};
use crate::{Message, Notice, Surco, RUNTIME};

/// ID de la playlist de radio (hay una sola; una radio nueva la reemplaza).
pub const RADIO_ID: &str = "radio";
/// Videos que se piden al Mix.
const MIX_SIZE: usize = 30;
/// Cola sin fin: con menos de estas por delante se piden más.
const ENDLESS_AHEAD: usize = 3;

/// Resultado de pedir el Mix: video semilla, nombre para mostrar y videos.
pub type RadioResult = Result<(String, String, Vec<SearchResult>), String>;

impl Surco {
    /// Video y nombre de lo que suena (o sonó por última vez).
    fn radio_seed(&self) -> Option<(String, String)> {
        if let Some(track) = self.now_track() {
            let video = self.video_of_track(track)?.to_owned();
            return Some((video, track.title.clone()));
        }
        match &self.status.source {
            Some(Source::YouTube(v)) => Some((
                v.clone(),
                self.status.title.clone().unwrap_or_else(|| v.clone()),
            )),
            _ => None,
        }
    }

    /// Botón "Radio": canciones parecidas a la que suena. `autostart` es la
    /// radio automática al terminar la cola (arranca con la primera nueva).
    pub(crate) fn start_radio(&mut self, autostart: bool) -> Task<Message> {
        let Some((video, name)) = self.radio_seed() else {
            if !autostart {
                self.notice = Some(Notice {
                    error: false,
                    text: tr!(
                        "Poné una canción para armar una radio con parecidas.",
                        "Play a song to build a radio of similar ones."
                    )
                    .into(),
                });
            }
            return Task::none();
        };
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            return Task::none();
        };
        if self.radio_loading {
            return Task::none();
        }
        self.radio_loading = true;
        tracing::info!(%video, autostart, "armando radio");
        let job = rt.spawn(async move {
            let list = provider
                .ytdlp()
                .mix(&video, MIX_SIZE)
                .await
                .map_err(|e| e.to_string())?;
            Ok((video, name, list))
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            move |r| Message::RadioLoaded(autostart, r),
        )
    }

    pub(crate) fn on_radio(&mut self, autostart: bool, result: RadioResult) -> Task<Message> {
        self.radio_loading = false;
        let waited = std::mem::take(&mut self.radio_waiting);
        let (seed, name, list) = match result {
            Ok(r) => r,
            Err(e) => {
                self.notice = Some(Notice {
                    error: true,
                    text: trf!(
                        "No se pudo armar la radio: {e}",
                        "Couldn't build the radio: {e}"
                    ),
                });
                return if waited {
                    self.back_from_panel()
                } else {
                    Task::none()
                };
            }
        };
        // Sin título conocido (video suelto): el del propio video en el Mix.
        let name = match list.iter().find(|r| r.id == seed) {
            Some(r) if name == seed => track_from(r).title,
            _ => name,
        };
        let mut tracks = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for r in &list {
            // Al seguir solo, la semilla ya sonó.
            if (autostart && r.id == seed) || !seen.insert(r.id.clone()) {
                continue;
            }
            let track = track_from(r);
            let m = Match {
                youtube_id: r.id.clone(),
                score: 100,
                manual: true,
                candidates: Vec::new(),
            };
            self.persist_match(&cache_key(&track), &m);
            self.resolutions
                .insert(cache_key(&track), TrackRes::Done(m));
            tracks.push(track);
        }
        if tracks.is_empty() {
            self.notice = Some(Notice {
                error: true,
                text: tr!(
                    "YouTube no devolvió canciones parecidas.",
                    "YouTube returned no similar songs."
                )
                .into(),
            });
            return if waited {
                self.back_from_panel()
            } else {
                Task::none()
            };
        }
        let playlist = Playlist {
            id: RADIO_ID.into(),
            name: format!("Radio: {name}"),
            tracks,
        };
        self.persist_playlist(&playlist, "radio");
        let index = match self.playlists.iter().position(|(p, _)| p.id == RADIO_ID) {
            Some(i) => {
                self.playlists[i] = (playlist, "radio".into());
                i
            }
            None => {
                self.playlists.push((playlist, "radio".into()));
                self.playlists.len() - 1
            }
        };
        self.recs.remove(RADIO_ID);
        self.open_playlist(index);
        let opened = self.on_playlist_opened();

        self.notice = None;
        let seed_playing = self.status.source.as_ref() == Some(&Source::YouTube(seed.clone()))
            && matches!(
                self.status.state,
                PlayerState::Playing | PlayerState::Paused | PlayerState::Loading
            );
        if !autostart && seed_playing {
            // La semilla (primera de la radio) ya está sonando: no se corta.
            let tracks = self.playlists[index].0.tracks.clone();
            let first = tracks
                .iter()
                .position(|t| self.video_is(t, &seed))
                .unwrap_or(0);
            self.active = Some(Active::new(Some(index), tracks, first, false, self.repeat));
            self.update_preload();
            return opened;
        }
        self.active = None;
        let play = self.play_track(index, 0);
        Task::batch([play, opened])
    }

    /// Cola sin fin: cuando quedan pocas, agrega parecidas a la última.
    /// Pasa con la cola del buscador (`endless`) y, con "seguir con
    /// canciones parecidas", con cualquier cola sin repetir.
    pub(crate) fn extend_endless(&mut self) -> Task<Message> {
        let Some(a) = &self.active else {
            return Task::none();
        };
        // Repitiendo, se repite lo que hay: nunca se agregan parecidas.
        let wanted = self.repeat == RepeatMode::Off && (a.endless || self.settings.autoplay);
        if !wanted || self.extending || a.upcoming(ENDLESS_AHEAD).len() >= ENDLESS_AHEAD {
            return Task::none();
        }
        // Semilla: la última de la cola que ya tenga video.
        let seed = a
            .queue
            .order()
            .iter()
            .rev()
            .filter_map(|&i| a.tracks.get(i))
            .find_map(|t| self.video_of_track(t))
            .map(str::to_owned);
        let (Some(seed), Some(provider), Some(rt)) = (seed, self.provider.clone(), RUNTIME.get())
        else {
            return Task::none();
        };
        if self.extend_failed.as_deref() == Some(seed.as_str()) {
            return Task::none();
        }
        self.extending = true;
        tracing::info!(%seed, "agregando canciones parecidas a la cola");
        let job = rt.spawn(async move {
            let list = provider
                .ytdlp()
                .mix(&seed, MIX_SIZE)
                .await
                .map_err(|e| e.to_string());
            (seed, list)
        });
        Task::perform(
            async move {
                job.await
                    .unwrap_or_else(|e| (String::new(), Err(e.to_string())))
            },
            |(seed, r)| Message::QueueExtended(seed, r),
        )
    }

    pub(crate) fn on_extended(
        &mut self,
        seed: String,
        result: Result<Vec<SearchResult>, String>,
    ) -> Task<Message> {
        self.extending = false;
        let list = match result {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "no se pudieron traer parecidas");
                self.extend_failed = Some(seed);
                return Task::none();
            }
        };
        let Some(a) = &self.active else {
            return Task::none();
        };
        // Se activó "repetir" mientras se buscaban: no agregar nada.
        if self.repeat != RepeatMode::Off {
            return Task::none();
        }
        let known: std::collections::HashSet<String> = a
            .tracks
            .iter()
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect();
        let fresh: Vec<&SearchResult> = list
            .iter()
            .filter(|r| r.id != seed && !known.contains(&r.id))
            .collect();
        if fresh.is_empty() {
            self.extend_failed = Some(seed);
            return Task::none();
        }
        let mut added = Vec::new();
        for r in fresh {
            let track = track_from(r);
            let m = Match {
                youtube_id: r.id.clone(),
                score: 100,
                manual: true,
                candidates: Vec::new(),
            };
            self.persist_match(&cache_key(&track), &m);
            self.resolutions
                .insert(cache_key(&track), TrackRes::Done(m));
            added.push(track);
        }
        let n = added.len();
        if let Some(a) = &mut self.active {
            for t in added {
                a.tracks.push(t);
                a.queue.push(a.tracks.len() - 1);
                a.auto.push(a.tracks.len() - 1);
            }
        }
        tracing::info!(n, "parecidas agregadas a la cola");
        self.update_preload();
        Task::none()
    }

    fn video_is(&self, track: &Track, video: &str) -> bool {
        matches!(self.resolution(track), Some(TrackRes::Done(m)) if m.youtube_id == video)
    }
}

/// Canción a partir de un video: "Artista - Título (Official Video)" →
/// título y artista limpios; si no hay guion, el artista es el canal.
pub fn track_from(r: &SearchResult) -> Track {
    let (artist, title) = split_title(&r.title, r.channel.as_deref());
    Track {
        title,
        artists: vec![artist],
        album: None,
        duration_ms: r.duration.map(|d| d.as_millis() as u64).unwrap_or(0),
        isrc: None,
        // Clave estable y única por video (ver `cache_key`).
        spotify_id: Some(format!("yt:{}", r.id)),
    }
}

/// Palabras de los paréntesis que no son parte del nombre de la canción.
const NOISE: &[&str] = &[
    "official",
    "oficial",
    "video",
    "vídeo",
    "audio",
    "lyric",
    "letra",
    "visualizer",
    "visualiser",
    "4k",
    "hd",
    "hq",
    "remaster",
    "mv",
    "clip",
];

pub fn split_title(raw: &str, channel: Option<&str>) -> (String, String) {
    let cleaned = strip_noise(raw);
    let dash = [" - ", " – ", " — "]
        .iter()
        .find_map(|sep| cleaned.split_once(sep));
    match dash {
        Some((a, t)) if !a.trim().is_empty() && !t.trim().is_empty() => {
            (a.trim().to_owned(), t.trim().to_owned())
        }
        _ => (
            channel.map(clean_channel).unwrap_or_default(),
            cleaned.trim().to_owned(),
        ),
    }
}

/// Saca los `(…)` y `[…]` que solo dicen "Official Video", "4K", etc.
fn strip_noise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find(['(', '[']) {
        let close = if rest[start..].starts_with('(') {
            ')'
        } else {
            ']'
        };
        let Some(len) = rest[start..].find(close) else {
            break;
        };
        let inner = rest[start + 1..start + len].to_lowercase();
        out.push_str(&rest[..start]);
        if !NOISE.iter().any(|w| inner.contains(w)) {
            out.push_str(&rest[start..=start + len]);
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn clean_channel(c: &str) -> String {
    let mut c = c.trim();
    for suffix in [" - Topic", "VEVO", " (official)", " Official"] {
        if let Some(s) = c.strip_suffix(suffix) {
            c = s.trim();
        }
    }
    c.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn splits_artist_and_title() {
        assert_eq!(
            split_title(
                "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)",
                Some("Rick Astley")
            ),
            ("Rick Astley".into(), "Never Gonna Give You Up".into())
        );
        assert_eq!(
            split_title("Bad Bunny - Tití Me Preguntó (Live) [Video Oficial]", None),
            ("Bad Bunny".into(), "Tití Me Preguntó (Live)".into())
        );
        assert_eq!(
            split_title("Tarzan Boy", Some("Baltimora - Topic")),
            ("Baltimora".into(), "Tarzan Boy".into())
        );
        assert_eq!(
            split_title("Hello", Some("AdeleVEVO")),
            ("Adele".into(), "Hello".into())
        );
    }

    #[test]
    fn radio_tracks_have_unique_keys() {
        let r = SearchResult {
            id: "dQw4w9WgXcQ".into(),
            title: "Rick Astley - Never Gonna Give You Up".into(),
            channel: None,
            duration: Some(Duration::from_secs(214)),
            views: None,
        };
        let t = track_from(&r);
        assert_eq!(cache_key(&t), "yt:dQw4w9WgXcQ");
        assert_eq!(t.duration_ms, 214_000);
    }
}
