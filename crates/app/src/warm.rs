//! Pedir la URL del audio apenas el mouse se queda sobre una canción, antes
//! del clic: así, al tocarla, el reproductor ya la tiene y arranca enseguida.

use std::time::Duration;

use iced::Task;
use surco_core::Track;
use surco_resolver::cache_key;

use crate::playback::TrackRes;
use crate::{Message, Surco, RUNTIME};

/// Cuánto tiene que quedarse el mouse sobre la fila: pasar de largo (o
/// mover la rueda) no dispara nada.
const HOVER_DELAY: Duration = Duration::from_millis(350);

/// Canción bajo el mouse.
#[derive(Debug, Clone)]
pub struct Hover {
    pub track: Track,
    /// Video ya conocido (resultados del buscador).
    pub video: Option<String>,
}

/// Estado del calentado.
#[derive(Debug, Default)]
pub struct Warm {
    /// Canción bajo el mouse (clave del resolver y datos).
    hover: Option<(String, Hover)>,
    /// Video cuya URL se está pidiendo: de a uno, para no lanzar un yt-dlp
    /// por cada fila que se cruza.
    running: Option<String>,
}

impl Surco {
    /// La fila de esta canción tiene el mouse encima.
    pub(crate) fn is_hovered(&self, track: &surco_core::Track) -> bool {
        self.warm
            .hover
            .as_ref()
            .is_some_and(|(k, _)| *k == surco_resolver::cache_key(track))
    }

    pub(crate) fn on_hover(&mut self, hover: Option<Box<Hover>>) -> Task<Message> {
        let Some(h) = hover else {
            self.warm.hover = None;
            return Task::none();
        };
        let key = cache_key(&h.track);
        self.warm.hover = Some((key.clone(), *h));
        Task::perform(tokio::time::sleep(HOVER_DELAY), move |_| {
            Message::HoverSettled(key.clone())
        })
    }

    /// El mouse sigue sobre la misma canción: buscarla si hace falta y
    /// pedir su URL.
    pub(crate) fn on_hover_settled(&mut self, key: String) -> Task<Message> {
        let Some((k, h)) = &self.warm.hover else {
            return Task::none();
        };
        if *k != key {
            return Task::none();
        }
        if let Some(v) = h.video.clone() {
            return self.warm_url(v);
        }
        let track = h.track.clone();
        match self.resolution(&track) {
            Some(TrackRes::Done(m)) => {
                let v = m.youtube_id.clone();
                self.warm_url(v)
            }
            // Ya se está buscando: `warm_after_resolved` sigue cuando llegue.
            Some(TrackRes::Resolving) | Some(TrackRes::Failed(_)) => Task::none(),
            None if self.resolver.is_some() => self.spawn_resolve(track),
            None => Task::none(),
        }
    }

    /// Llegó una búsqueda: si es la de la canción bajo el mouse, pedir su URL.
    pub(crate) fn warm_after_resolved(&mut self, key: &str, video: &str) -> Task<Message> {
        if self.warm.hover.as_ref().is_some_and(|(k, _)| k == key) {
            return self.warm_url(video.to_owned());
        }
        Task::none()
    }

    pub(crate) fn on_warmed(&mut self, video: String) {
        if self.warm.running.as_deref() == Some(video.as_str()) {
            self.warm.running = None;
        }
    }

    /// Pide (y deja en caché) la URL del audio de `video`.
    fn warm_url(&mut self, video: String) -> Task<Message> {
        let (Some(provider), Some(rt)) = (self.provider.clone(), RUNTIME.get()) else {
            return Task::none();
        };
        if self.warm.running.is_some()
            || provider.has_url(&video)
            || provider.cached_audio(&video).is_some()
        {
            return Task::none();
        }
        // Ya suena: el motor la tiene.
        let now = self.now_track().and_then(|t| self.video_of_track(t));
        if now == Some(video.as_str()) {
            return Task::none();
        }
        self.warm.running = Some(video.clone());
        let v = video.clone();
        let job = rt.spawn(async move {
            if let Err(e) = provider.stream_info(&v, false).await {
                tracing::debug!(video = v, error = %e, "no se pudo adelantar la URL");
            }
        });
        Task::perform(job, move |_| Message::Warmed(video.clone()))
    }
}
