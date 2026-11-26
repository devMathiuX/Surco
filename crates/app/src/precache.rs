//! Precarga a disco: baja el audio de las próximas canciones (o de toda la
//! playlist) antes de que suenen, de a una por vez. Así no hay que esperar
//! a que carguen y no depende de la red en ese momento.

use std::collections::HashSet;

use iced::Task;
use surco_core::Track;
use surco_resolver::{cache_key, Match};

use crate::playback::TrackRes;
use crate::settings::Precache;
use crate::{Message, Surco, RUNTIME};

/// Canciones que se bajan con "las próximas".
pub const NEXT_COUNT: usize = 5;

/// Pausa de la precarga cuando YouTube rechaza una descarga (403): seguir
/// bajando en ese momento solo hace que también rechace lo que suena.
const BACKOFF: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Debug, Default)]
pub struct PrecacheState {
    /// Canción (clave) que se está bajando.
    pub running: Option<String>,
    /// Canciones que fallaron (no se reintentan en esta sesión).
    failed: HashSet<String>,
    /// Videos que ya están en disco.
    done: HashSet<String>,
    /// Progreso para mostrar: listas / pedidas.
    pub ready: usize,
    pub target: usize,
    /// En pausa por un rechazo de YouTube (ver [`BACKOFF`]).
    paused: bool,
}

impl Surco {
    /// Cuántas canciones por delante hay que tener resueltas.
    pub(crate) fn lookahead(&self) -> usize {
        match self.settings.precache {
            Precache::Off => 0,
            Precache::Next => NEXT_COUNT,
            Precache::Playlist => usize::MAX,
        }
    }

    /// Si cambió la cola o los ajustes, arranca la próxima descarga.
    pub(crate) fn pump_precache(&mut self) -> Task<Message> {
        if self.precache.paused || !std::mem::take(&mut self.precache_dirty) {
            return Task::none();
        }
        let n = self.lookahead();
        let (Some(provider), Some(resolver), Some(rt)) =
            (self.provider.clone(), self.resolver.clone(), RUNTIME.get())
        else {
            return Task::none();
        };
        let Some(cache) = provider.audio_cache() else {
            return Task::none();
        };
        let tracks: Vec<Track> = match (&self.active, n > 0) {
            (Some(a), true) => a
                .upcoming(n.min(a.queue.len()))
                .into_iter()
                .cloned()
                .collect(),
            _ => Vec::new(),
        };

        let mut ready = 0;
        let mut todo = None;
        for t in &tracks {
            let key = cache_key(t);
            match self.resolutions.get(&key) {
                Some(TrackRes::Done(m)) => {
                    let v = &m.youtube_id;
                    if self.precache.done.contains(v) || cache.contains(v) {
                        self.precache.done.insert(v.clone());
                        ready += 1;
                        continue;
                    }
                }
                // La está buscando la cola: después se baja.
                Some(TrackRes::Resolving) | Some(TrackRes::Failed(_)) => continue,
                None => {}
            }
            if todo.is_none() && !self.precache.failed.contains(&key) {
                todo = Some(t.clone());
            }
        }
        self.precache.ready = ready;
        self.precache.target = tracks.len();
        if self.precache.running.is_some() {
            return Task::none();
        }
        let Some(track) = todo else {
            return Task::none();
        };
        let key = cache_key(&track);
        self.precache.running = Some(key.clone());
        let client = self.http.clone();
        let job = rt.spawn(async move {
            let m = resolver.resolve(&track).await.map_err(|e| e.to_string())?;
            provider
                .precache(&client, &m.youtube_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(m)
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            move |r| Message::Precached(key.clone(), r),
        )
    }

    pub(crate) fn on_precached(
        &mut self,
        key: String,
        result: Result<Match, String>,
    ) -> Task<Message> {
        self.precache.running = None;
        self.precache_dirty = true;
        match result {
            Ok(m) => {
                self.precache.done.insert(m.youtube_id.clone());
                let video = m.youtube_id.clone();
                if !matches!(self.resolutions.get(&key), Some(TrackRes::Done(_))) {
                    self.persist_match(&key, &m);
                    self.resolutions.insert(key, TrackRes::Done(m));
                    self.update_preload();
                }
                if self.settings.covers_in_list {
                    return self.queue_cover(&video);
                }
            }
            // YouTube rechazó la descarga: no es culpa de esta canción, así
            // que se reintenta después de una pausa.
            Err(e) if is_rejection(&e) => {
                tracing::warn!(%key, error = %e, secs = BACKOFF.as_secs(), "YouTube rechazó la precarga; en pausa");
                self.precache.paused = true;
                return Task::perform(tokio::time::sleep(BACKOFF), |_| Message::PrecacheResume);
            }
            Err(e) => {
                tracing::warn!(%key, error = %e, "no se pudo precargar");
                self.precache.failed.insert(key);
            }
        }
        Task::none()
    }

    /// Termina la pausa de la precarga.
    pub(crate) fn resume_precache(&mut self) {
        self.precache.paused = false;
        self.precache_dirty = true;
    }

    /// Al arrancar: borra la precarga de la vez anterior (por si la app no
    /// se cerró bien). Solo queda la canción guardada para retomar. También
    /// borra el audio de las "playlists descargadas" de versiones viejas.
    pub(crate) fn drop_temporary_audio(&self) {
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            let before = cache.usage();
            cache.clear();
            let after = cache.usage();
            if before.files > after.files {
                tracing::info!(
                    files = before.files - after.files,
                    "precarga de la vez anterior borrada"
                );
            }
        }
    }

    /// Vacía la caché de audio (botón de ajustes).
    pub(crate) fn clear_audio_cache(&mut self) {
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            cache.clear();
        }
        self.precache.done.clear();
        self.precache.failed.clear();
        self.precache_dirty = true;
    }
}

/// Si el error es un rechazo de YouTube a la descarga (403) y no un problema
/// de la canción.
fn is_rejection(error: &str) -> bool {
    error.contains("HTTP 403") || error.contains("venció")
}
