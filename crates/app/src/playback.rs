//! Reproducir canciones de una playlist: resolverlas a YouTube (de forma
//! perezosa: la actual + las próximas 3), pasar sola a la siguiente y elegir
//! otro video a mano cuando el match es dudoso.

use std::sync::Arc;

use iced::Task;
use surco_core::{PlayerState, Track};
use surco_player::{Command, Source, Status};
use surco_resolver::{cache_key, Match, Resolver};

use crate::{Message, Notice, Surco, RUNTIME};

/// Cuántas canciones por delante se resuelven de antemano.
pub const PREFETCH: usize = 3;
/// Errores seguidos antes de dejar de saltar a la siguiente.
const MAX_FAILURES_IN_A_ROW: u32 = 3;

/// Estado de resolución de una canción (por clave del resolver).
#[derive(Debug, Clone)]
pub enum TrackRes {
    Resolving,
    Done(Match),
    Failed(String),
}

impl Surco {
    pub(crate) fn track(&self, (p, i): (usize, usize)) -> Option<&Track> {
        self.playlists.get(p)?.0.tracks.get(i)
    }

    pub(crate) fn resolution(&self, track: &Track) -> Option<&TrackRes> {
        self.resolutions.get(&cache_key(track))
    }

    /// Reproduce la canción `i` de la playlist `p`.
    pub(crate) fn play_track(&mut self, p: usize, i: usize) -> Task<Message> {
        let Some(track) = self.track((p, i)).cloned() else {
            return Task::none();
        };
        self.now = Some((p, i));
        let mut tasks = Vec::new();
        match self.resolution(&track) {
            Some(TrackRes::Done(m)) => {
                let id = m.youtube_id.clone();
                self.send(Command::Load(Source::YouTube(id)));
            }
            Some(TrackRes::Resolving) => {}
            // Sin resolver o con un error anterior: (re)intentar.
            _ => tasks.push(self.spawn_resolve(track)),
        }
        // Pre-resolver las próximas.
        for j in i + 1..=i + PREFETCH {
            if let Some(next) = self.track((p, j)).cloned() {
                if self.resolution(&next).is_none() {
                    tasks.push(self.spawn_resolve(next));
                }
            }
        }
        Task::batch(tasks)
    }

    /// Lanza la resolución en el runtime; el resultado vuelve como mensaje.
    fn spawn_resolve(&mut self, track: Track) -> Task<Message> {
        let (Some(resolver), Some(rt)) = (self.resolver.clone(), RUNTIME.get()) else {
            self.notice = Some(Notice {
                error: true,
                text: "Falta yt-dlp para buscar en YouTube.".into(),
            });
            return Task::none();
        };
        let key = cache_key(&track);
        self.resolutions.insert(key.clone(), TrackRes::Resolving);
        let job = rt.spawn(async move { resolver.resolve(&track).await });
        Task::perform(
            async move {
                match job.await {
                    Ok(r) => r.map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            },
            move |result| Message::Resolved(key.clone(), result),
        )
    }

    pub(crate) fn on_resolved(
        &mut self,
        key: String,
        result: Result<Match, String>,
    ) -> Task<Message> {
        let is_now = self
            .now
            .and_then(|n| self.track(n))
            .is_some_and(|t| cache_key(t) == key);
        match result {
            Ok(m) => {
                if is_now {
                    self.send(Command::Load(Source::YouTube(m.youtube_id.clone())));
                }
                self.resolutions.insert(key, TrackRes::Done(m));
                Task::none()
            }
            Err(e) => {
                self.resolutions.insert(key, TrackRes::Failed(e.clone()));
                if is_now {
                    return self.skip_after_failure(e);
                }
                Task::none()
            }
        }
    }

    /// Nuevo estado del motor: detecta fin de canción y errores.
    pub(crate) fn on_status(&mut self, status: Status) -> Task<Message> {
        let old = std::mem::replace(&mut self.status, status);
        if self.now.is_none() {
            return Task::none();
        }
        match &self.status.state {
            PlayerState::Playing => self.failures_in_a_row = 0,
            PlayerState::Idle
                if matches!(old.state, PlayerState::Playing | PlayerState::Paused) =>
            {
                return self.advance(1);
            }
            PlayerState::Error(e) if !matches!(old.state, PlayerState::Error(_)) => {
                let e = e.clone();
                return self.skip_after_failure(e);
            }
            _ => {}
        }
        Task::none()
    }

    /// Aviso discreto y a la siguiente, salvo que ya fallaron varias seguidas
    /// (por ejemplo, YouTube bloqueó la red): ahí frena.
    fn skip_after_failure(&mut self, error: String) -> Task<Message> {
        self.failures_in_a_row += 1;
        let title = self
            .now
            .and_then(|n| self.track(n))
            .map(|t| t.title.clone())
            .unwrap_or_default();
        if self.failures_in_a_row >= MAX_FAILURES_IN_A_ROW {
            self.notice = Some(Notice {
                error: true,
                text: format!(
                    "Frené después de {MAX_FAILURES_IN_A_ROW} errores seguidos. Último: {error}"
                ),
            });
            self.now = None;
            return Task::none();
        }
        self.notice = Some(Notice {
            error: true,
            text: format!("Salteé «{title}»: {error}"),
        });
        self.advance(1)
    }

    /// Siguiente (`1`) o anterior (`-1`) en la playlist actual.
    pub(crate) fn advance(&mut self, dir: isize) -> Task<Message> {
        let Some((p, i)) = self.now else {
            return Task::none();
        };
        match i
            .checked_add_signed(dir)
            .filter(|j| self.track((p, *j)).is_some())
        {
            Some(j) => self.play_track(p, j),
            None => {
                // Fin de la playlist (o principio, con "anterior").
                if dir > 0 {
                    self.now = None;
                }
                Task::none()
            }
        }
    }

    /// El usuario eligió otro video para la canción del selector.
    pub(crate) fn choose_video(&mut self, video: &str) -> Task<Message> {
        let Some(at) = self.choosing.take() else {
            return Task::none();
        };
        let (Some(track), Some(resolver)) = (self.track(at).cloned(), self.resolver.clone()) else {
            return Task::none();
        };
        if let Some(m) = resolver.choose(&track, video) {
            self.resolutions
                .insert(cache_key(&track), TrackRes::Done(m));
            if self.now == Some(at) {
                self.send(Command::Load(Source::YouTube(video.to_owned())));
            }
        }
        Task::none()
    }

    /// yt-dlp quedó disponible: armar el resolver.
    pub(crate) fn set_resolver(&mut self, resolver: Option<Arc<Resolver>>) {
        self.resolver = resolver;
    }
}
