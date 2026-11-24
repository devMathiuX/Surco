//! Reproducir una playlist: cola (shuffle/repetir), resolución perezosa a
//! YouTube (la actual + las próximas 3), pre-carga de la siguiente para que
//! no haya silencio, saltar canciones que fallan y elegir otro video a mano.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use surco_core::{PlayerState, RepeatMode, Track};
use surco_player::{Command, Queue, Source, Status};
use surco_resolver::{cache_key, Match, Resolver};

use crate::{covers, Message, Notice, Surco, RUNTIME};

/// Cuántas canciones por delante se resuelven de antemano.
pub const PREFETCH: usize = 3;
/// Errores seguidos antes de dejar de saltar a la siguiente.
const MAX_FAILURES_IN_A_ROW: u32 = 3;
/// "Anterior" con más de esto reproducido vuelve al principio de la canción.
const RESTART_THRESHOLD: Duration = Duration::from_secs(3);

/// Estado de resolución de una canción (por clave del resolver).
#[derive(Debug, Clone)]
pub enum TrackRes {
    Resolving,
    Done(Match),
    Failed(String),
}

/// Cola activa: sobre qué playlist y en qué orden.
#[derive(Debug, Clone)]
pub struct Active {
    pub playlist: usize,
    pub queue: Queue,
}

impl Surco {
    pub(crate) fn track(&self, (p, i): (usize, usize)) -> Option<&Track> {
        self.playlists.get(p)?.0.tracks.get(i)
    }

    pub(crate) fn resolution(&self, track: &Track) -> Option<&TrackRes> {
        self.resolutions.get(&cache_key(track))
    }

    /// Video elegido para una canción, si ya está resuelta.
    pub(crate) fn video_of(&self, at: (usize, usize)) -> Option<&str> {
        match self.resolution(self.track(at)?)? {
            TrackRes::Done(m) => Some(&m.youtube_id),
            _ => None,
        }
    }

    /// Canción sonando (playlist, índice).
    pub(crate) fn now(&self) -> Option<(usize, usize)> {
        let a = self.active.as_ref()?;
        Some((a.playlist, a.queue.current()?))
    }

    /// El usuario eligió la canción `i` de la playlist `p`.
    pub(crate) fn play_track(&mut self, p: usize, i: usize) -> Task<Message> {
        let Some(len) = self.playlists.get(p).map(|(pl, _)| pl.tracks.len()) else {
            return Task::none();
        };
        self.failures_in_a_row = 0;
        match &mut self.active {
            Some(a) if a.playlist == p && a.queue.len() == len => a.queue.jump(i),
            _ => {
                self.active = Some(Active {
                    playlist: p,
                    queue: Queue::new(len, i, self.shuffle, self.repeat, seed()),
                })
            }
        }
        self.start_current()
    }

    /// Carga la canción actual de la cola (resolviéndola si hace falta).
    fn start_current(&mut self) -> Task<Message> {
        let Some(at) = self.now() else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        match self
            .track(at)
            .map(|t| (t.clone(), self.resolution(t).cloned()))
        {
            Some((_, Some(TrackRes::Done(m)))) => {
                self.send(Command::Load(Source::YouTube(m.youtube_id.clone())));
                tasks.push(self.fetch_cover(&m.youtube_id));
            }
            Some((_, Some(TrackRes::Resolving))) => {}
            Some((track, _)) => tasks.push(self.spawn_resolve(track)),
            None => {}
        }
        tasks.push(self.prefetch());
        self.update_preload();
        Task::batch(tasks)
    }

    /// Resuelve de antemano las próximas canciones de la cola.
    fn prefetch(&mut self) -> Task<Message> {
        let Some(a) = &self.active else {
            return Task::none();
        };
        let p = a.playlist;
        let upcoming = a.queue.upcoming(PREFETCH);
        let tracks: Vec<Track> = upcoming
            .into_iter()
            .filter_map(|i| self.track((p, i)).cloned())
            .filter(|t| self.resolution(t).is_none())
            .collect();
        Task::batch(tracks.into_iter().map(|t| self.spawn_resolve(t)))
    }

    /// Le dice al motor qué pre-cargar: la siguiente de la cola, si ya está resuelta.
    pub(crate) fn update_preload(&mut self) {
        let next = self.active.as_ref().and_then(|a| {
            let i = a.queue.peek_next()?;
            self.video_of((a.playlist, i))
        });
        let source = next.map(|v| Source::YouTube(v.to_owned()));
        if source != self.preloaded {
            self.preloaded = source.clone();
            self.send(Command::Preload(source));
        }
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
            .now()
            .and_then(|n| self.track(n))
            .is_some_and(|t| cache_key(t) == key);
        match result {
            Ok(m) => {
                let mut task = Task::none();
                if is_now {
                    self.send(Command::Load(Source::YouTube(m.youtube_id.clone())));
                    task = self.fetch_cover(&m.youtube_id);
                }
                self.resolutions.insert(key, TrackRes::Done(m));
                self.update_preload();
                task
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

    /// Nuevo estado del motor: pasos automáticos, fin de canción y errores.
    pub(crate) fn on_status(&mut self, status: Status) -> Task<Message> {
        let old = std::mem::replace(&mut self.status, status);
        if self.status.position.abs_diff(old.position) > Duration::from_secs(2) {
            if let Some(m) = &mut self.media {
                m.force_refresh();
            }
        }
        if self.active.is_none() {
            return Task::none();
        }
        // El motor pasó solo a la pre-cargada (sin corte).
        if self.status.auto_advances > old.auto_advances {
            self.preloaded = None;
            self.failures_in_a_row = 0;
            if let Some(a) = &mut self.active {
                a.queue.advance();
            }
            let cover = self
                .now()
                .and_then(|n| self.video_of(n))
                .map(str::to_owned)
                .map(|v| self.fetch_cover(&v))
                .unwrap_or_else(Task::none);
            let prefetch = self.prefetch();
            self.update_preload();
            return Task::batch([cover, prefetch]);
        }
        match &self.status.state {
            PlayerState::Playing => self.failures_in_a_row = 0,
            // Terminó sin pre-carga lista: siguiente "a la antigua".
            PlayerState::Idle
                if matches!(old.state, PlayerState::Playing | PlayerState::Paused) =>
            {
                return self.advance_auto();
            }
            PlayerState::Error(e) if !matches!(old.state, PlayerState::Error(_)) => {
                let e = e.clone();
                return self.skip_after_failure(e);
            }
            _ => {}
        }
        Task::none()
    }

    fn advance_auto(&mut self) -> Task<Message> {
        let next = self.active.as_mut().and_then(|a| a.queue.advance());
        match next {
            Some(_) => self.start_current(),
            None => self.finish(),
        }
    }

    /// Fin de la playlist.
    fn finish(&mut self) -> Task<Message> {
        self.active = None;
        self.preloaded = None;
        self.send(Command::Preload(None));
        Task::none()
    }

    /// Aviso discreto y a la siguiente, salvo que ya fallaron varias seguidas
    /// (por ejemplo, YouTube bloqueó la red): ahí frena.
    fn skip_after_failure(&mut self, error: String) -> Task<Message> {
        self.failures_in_a_row += 1;
        let title = self
            .now()
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
            return self.finish();
        }
        self.notice = Some(Notice {
            error: true,
            text: format!("Salteé «{title}»: {error}"),
        });
        // A mano (no automático) para no repetir en bucle con "repetir una".
        match self.active.as_mut().and_then(|a| a.queue.skip(true)) {
            Some(_) => self.start_current(),
            None => self.finish(),
        }
    }

    /// Siguiente (`1`) o anterior (`-1`), a mano.
    pub(crate) fn skip(&mut self, dir: isize) -> Task<Message> {
        if dir < 0 && self.status.position > RESTART_THRESHOLD {
            self.send(Command::Seek(Duration::ZERO));
            return Task::none();
        }
        self.failures_in_a_row = 0;
        match self.active.as_mut().and_then(|a| a.queue.skip(dir > 0)) {
            Some(_) => self.start_current(),
            // Principio o fin de la playlist: no hacer nada.
            None => Task::none(),
        }
    }

    pub(crate) fn toggle_shuffle(&mut self) -> Task<Message> {
        self.shuffle = !self.shuffle;
        if let Some(a) = &mut self.active {
            a.queue.set_shuffle(self.shuffle);
        }
        self.update_preload();
        self.prefetch()
    }

    pub(crate) fn cycle_repeat(&mut self) -> Task<Message> {
        self.repeat = match self.repeat {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        if let Some(a) = &mut self.active {
            a.queue.set_repeat(self.repeat);
        }
        self.update_preload();
        self.prefetch()
    }

    /// Play/pausa. Sin nada cargado, arranca la playlist que se está viendo.
    pub(crate) fn toggle_pause(&mut self) -> Task<Message> {
        let nothing_loaded =
            self.status.source.is_none() && matches!(self.status.state, PlayerState::Idle);
        match (nothing_loaded, self.selected) {
            (true, Some(p)) if self.active.is_none() => self.play_track(p, 0),
            _ => {
                self.send(Command::TogglePause);
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
        let Some(m) = resolver.choose(&track, video) else {
            return Task::none();
        };
        self.resolutions
            .insert(cache_key(&track), TrackRes::Done(m));
        let mut task = Task::none();
        if self.now() == Some(at) {
            self.send(Command::Load(Source::YouTube(video.to_owned())));
            task = self.fetch_cover(video);
        }
        self.update_preload();
        task
    }

    /// Baja (o lee de la caché) la portada de un video.
    pub(crate) fn fetch_cover(&mut self, video: &str) -> Task<Message> {
        if self.covers.contains_key(video) {
            return Task::none();
        }
        let Some(rt) = RUNTIME.get() else {
            return Task::none();
        };
        let client = self.http.clone();
        let id = video.to_owned();
        let job = rt.spawn(covers::load(client, id.clone()));
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            move |r| Message::Cover(id.clone(), r),
        )
    }

    /// yt-dlp quedó disponible: armar el resolver.
    pub(crate) fn set_resolver(&mut self, resolver: Option<Arc<Resolver>>) {
        self.resolver = resolver;
    }
}

/// Semilla para el shuffle (no necesita ser criptográfica).
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}
