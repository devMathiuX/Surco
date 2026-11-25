//! Retomar la última canción al abrir: la cola y la posición se guardan
//! mientras suena, y su audio queda en la caché de disco (no se borra al
//! cerrar). Al abrir, se carga en pausa donde estaba: darle a play suena
//! al instante.

use std::time::Duration;

use iced::Task;
use serde::{Deserialize, Serialize};
use surco_core::Track;
use surco_player::{Command, Source};

use crate::playback::Active;
use crate::{Message, Surco};

/// La cola (cambia con cada canción).
const QUEUE: &str = "resume_queue";
/// La posición en milisegundos (cambia mientras suena).
const POSITION: &str = "resume_position";
/// Cada cuánto se guarda la posición mientras suena.
const SAVE_EVERY: Duration = Duration::from_secs(5);
/// Canciones sueltas (buscador, radio) que se guardan como máximo.
const MAX_LOOSE: usize = 300;

/// Lo guardado de la cola.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Saved {
    /// Playlist de la biblioteca: sus canciones no se guardan aparte.
    playlist: Option<String>,
    /// Canciones de una cola suelta (o lo agregado a una playlist).
    #[serde(default)]
    tracks: Vec<Track>,
    index: usize,
    #[serde(default)]
    endless: bool,
    video: String,
}

/// Lo último guardado, para no escribir de más.
#[derive(Debug, Default)]
pub struct ResumeState {
    queue: Option<Saved>,
    position: Option<Duration>,
}

impl Surco {
    fn saved_resume(&self) -> Option<(Saved, Duration)> {
        let store = self.store.as_ref()?;
        let queue: Saved = serde_json::from_str(&store.setting(QUEUE).ok()??).ok()?;
        let at = store
            .setting(POSITION)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_millis)
            .unwrap_or_default();
        Some((queue, at))
    }

    /// Al abrir: carga en pausa la última canción, donde había quedado.
    /// Tiene que ir antes de borrar la precarga de la vez anterior.
    pub(crate) fn restore_resume(&mut self) -> Task<Message> {
        let Some((saved, at)) = self.saved_resume() else {
            return Task::none();
        };
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            cache.keep(Some(&saved.video));
        }
        let origin = saved
            .playlist
            .as_ref()
            .and_then(|id| self.playlists.iter().position(|(p, _)| &p.id == id));
        let tracks = match origin {
            Some(p) => {
                let mut t = self.playlists[p].0.tracks.clone();
                t.extend(saved.tracks.iter().cloned());
                t
            }
            None if saved.playlist.is_none() => saved.tracks.clone(),
            // La playlist ya no existe.
            None => return Task::none(),
        };
        if saved.index >= tracks.len() {
            return Task::none();
        }
        let mut active = Active::new(origin, tracks, saved.index, self.shuffle, self.repeat);
        active.endless = saved.endless;
        self.active = Some(active);
        tracing::info!(video = %saved.video, ?at, "retomando la última canción");
        self.send(Command::LoadAt {
            source: Source::YouTube(saved.video.clone()),
            at,
            paused: true,
        });
        self.resume = ResumeState {
            queue: Some(saved.clone()),
            position: Some(at),
        };
        let cover = self.fetch_cover(&saved.video);
        let prefetch = self.prefetch();
        self.update_preload();
        Task::batch([cover, prefetch])
    }

    /// Guarda la canción que suena (si cambió) y la posición (cada
    /// [`SAVE_EVERY`]). Se llama con cada estado nuevo del motor.
    pub(crate) fn save_resume(&mut self) {
        let Some(Source::YouTube(video)) = &self.status.source else {
            return;
        };
        let Some(a) = &self.active else {
            return;
        };
        // Solo si lo que suena es la canción actual de la cola.
        if self.now_track().and_then(|t| self.video_of_track(t)) != Some(video.as_str()) {
            return;
        }
        let Some(index) = a.queue.current() else {
            return;
        };
        let playlist = a
            .origin
            .and_then(|p| self.playlists.get(p))
            .filter(|(p, _)| a.tracks.starts_with(&p.tracks));
        let saved = match playlist {
            Some((p, _)) => Saved {
                playlist: Some(p.id.clone()),
                tracks: a.tracks[p.tracks.len()..].to_vec(),
                index,
                endless: a.endless,
                video: video.clone(),
            },
            None => {
                // Cola suelta muy larga: se guarda un tramo alrededor de la actual.
                let start = index.saturating_sub(MAX_LOOSE / 2);
                let end = (start + MAX_LOOSE).min(a.tracks.len());
                Saved {
                    playlist: None,
                    tracks: a.tracks[start..end].to_vec(),
                    index: index - start,
                    endless: a.endless,
                    video: video.clone(),
                }
            }
        };
        // Siempre: si se cambió el provider (yt-dlp nuevo), su caché no la sabe.
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            cache.keep(Some(video));
        }
        let Some(store) = &self.store else {
            return;
        };
        if self.resume.queue.as_ref() != Some(&saved) {
            if let Ok(json) = serde_json::to_string(&saved) {
                if let Err(e) = store.set_setting(QUEUE, &json) {
                    tracing::warn!(error = %e, "no se pudo guardar la cola para retomar");
                }
            }
            self.resume.queue = Some(saved);
            self.resume.position = None;
        }
        let at = self.status.position;
        let moved = self
            .resume
            .position
            .is_none_or(|last| at.abs_diff(last) >= SAVE_EVERY);
        if moved {
            let _ = store.set_setting(POSITION, &at.as_millis().to_string());
            self.resume.position = Some(at);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn saved_queue_round_trips() {
        let s = Saved {
            playlist: Some("spotify:abc".into()),
            tracks: Vec::new(),
            index: 7,
            endless: false,
            video: "aqz-KE-bpKQ".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&json).unwrap(), s);
        // Lo viejo sin campos opcionales también se lee.
        let old: Saved =
            serde_json::from_str(r#"{"playlist":null,"index":0,"video":"aqz-KE-bpKQ"}"#).unwrap();
        assert!(old.tracks.is_empty());
    }
}
