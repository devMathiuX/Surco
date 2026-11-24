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
/// Portadas que se bajan a la vez.
const COVER_JOBS: usize = 4;

/// Cola de descargas de portadas.
#[derive(Debug, Default)]
pub struct CoverJobs {
    queue: std::collections::VecDeque<String>,
    /// En la cola o bajándose.
    pending: std::collections::HashSet<String>,
    failed: std::collections::HashSet<String>,
    running: usize,
}
/// Canciones que se buscan a la vez en segundo plano (para las portadas).
const BACKGROUND_JOBS: usize = 2;

/// Búsqueda en segundo plano de las canciones de la playlist abierta, para
/// que aparezcan sus portadas (y después suenen sin esperar).
#[derive(Debug, Default)]
pub struct Background {
    queue: std::collections::VecDeque<Track>,
    /// Claves que se están buscando.
    running: std::collections::HashSet<String>,
}

impl Background {
    pub fn is_running(&self) -> bool {
        !self.queue.is_empty() || !self.running.is_empty()
    }
}

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

/// Cola de reproducción.
///
/// Tiene sus propias canciones: las de la playlist de la que salió (con los
/// mismos índices) más las que se agregan a mano o llegan del buscador.
#[derive(Debug, Clone)]
pub struct Active {
    /// Playlist de la biblioteca de la que salió (`None`: buscador o radio).
    pub origin: Option<usize>,
    pub tracks: Vec<Track>,
    pub queue: Queue,
    /// Sin fin: al acercarse al final se agregan canciones parecidas.
    pub endless: bool,
}

impl Active {
    pub fn new(
        origin: Option<usize>,
        tracks: Vec<Track>,
        start: usize,
        shuffle: bool,
        repeat: RepeatMode,
    ) -> Self {
        let len = tracks.len();
        Self {
            origin,
            tracks,
            queue: Queue::new(len, start, shuffle, repeat, seed()),
            endless: false,
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.tracks.get(self.queue.current()?)
    }

    /// Canciones que vienen después de la actual.
    pub fn upcoming(&self, n: usize) -> Vec<&Track> {
        self.queue
            .upcoming(n)
            .into_iter()
            .filter_map(|i| self.tracks.get(i))
            .collect()
    }
}

impl Surco {
    pub(crate) fn track(&self, (p, i): (usize, usize)) -> Option<&Track> {
        self.playlists.get(p)?.0.tracks.get(i)
    }

    pub(crate) fn resolution(&self, track: &Track) -> Option<&TrackRes> {
        self.resolutions.get(&cache_key(track))
    }

    /// Video elegido para una canción de una playlist, si ya está resuelta.
    pub(crate) fn video_of(&self, at: (usize, usize)) -> Option<&str> {
        self.video_of_track(self.track(at)?)
    }

    pub(crate) fn video_of_track(&self, track: &Track) -> Option<&str> {
        match self.resolution(track)? {
            TrackRes::Done(m) => Some(&m.youtube_id),
            _ => None,
        }
    }

    /// Canción que suena (la actual de la cola).
    pub(crate) fn now_track(&self) -> Option<&Track> {
        self.active.as_ref()?.current()
    }

    /// La que suena, como (playlist, índice) si es una canción de esa playlist.
    pub(crate) fn now_in_playlist(&self) -> Option<(usize, usize)> {
        let a = self.active.as_ref()?;
        let p = a.origin?;
        let i = a.queue.current()?;
        let len = self.playlists.get(p)?.0.tracks.len();
        (i < len).then_some((p, i))
    }

    /// El usuario eligió la canción `i` de la playlist `p`.
    pub(crate) fn play_track(&mut self, p: usize, i: usize) -> Task<Message> {
        let Some(tracks) = self.playlists.get(p).map(|(pl, _)| pl.tracks.clone()) else {
            return Task::none();
        };
        self.failures_in_a_row = 0;
        match &mut self.active {
            // Misma playlist: se conserva la cola (con lo agregado a mano).
            Some(a)
                if a.origin == Some(p)
                    && a.tracks.starts_with(&tracks)
                    && a.queue.order().contains(&i) =>
            {
                a.queue.jump(i)
            }
            _ => self.active = Some(Active::new(Some(p), tracks, i, self.shuffle, self.repeat)),
        }
        self.start_current()
    }

    /// Reproduce una lista suelta (buscador, playlist de YouTube sin guardar).
    /// `endless`: al terminar sigue con parecidas.
    pub(crate) fn play_list(
        &mut self,
        tracks: Vec<Track>,
        start: usize,
        endless: bool,
    ) -> Task<Message> {
        if tracks.is_empty() {
            return Task::none();
        }
        self.failures_in_a_row = 0;
        let mut a = Active::new(None, tracks, start, false, self.repeat);
        a.endless = endless;
        self.active = Some(a);
        self.start_current()
    }

    /// Carga la canción actual de la cola (resolviéndola si hace falta).
    pub(crate) fn start_current(&mut self) -> Task<Message> {
        let Some(track) = self.now_track().cloned() else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        match Some(track).map(|t| {
            let r = self.resolution(&t).cloned();
            (t, r)
        }) {
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
        if self.settings.covers_in_list {
            tasks.push(self.queue_list_covers());
        }
        Task::batch(tasks)
    }

    /// Resuelve de antemano las próximas canciones de la cola.
    pub(crate) fn prefetch(&mut self) -> Task<Message> {
        let Some(a) = &self.active else {
            return Task::none();
        };
        let tracks: Vec<Track> = a
            .upcoming(PREFETCH)
            .into_iter()
            .filter(|t| self.resolution(t).is_none())
            .cloned()
            .collect();
        Task::batch(tracks.into_iter().map(|t| self.spawn_resolve(t)))
    }

    /// Le dice al motor qué pre-cargar: la siguiente de la cola, si ya está resuelta.
    pub(crate) fn update_preload(&mut self) {
        let next = self.active.as_ref().and_then(|a| {
            let i = a.queue.peek_next()?;
            self.video_of_track(a.tracks.get(i)?)
        });
        let source = next.map(|v| Source::YouTube(v.to_owned()));
        if source != self.preloaded {
            self.preloaded = source.clone();
            self.send(Command::Preload(source));
        }
        // Cambió la cola o una resolución: revisar la precarga a disco.
        self.precache_dirty = true;
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
        self.background.running.remove(&key);
        let is_now = self.now_track().is_some_and(|t| cache_key(t) == key);
        match result {
            Ok(m) => {
                self.persist_match(&key, &m);
                let mut task = Task::none();
                if is_now {
                    self.send(Command::Load(Source::YouTube(m.youtube_id.clone())));
                    task = self.fetch_cover(&m.youtube_id);
                }
                let cover = if self.settings.covers_in_list {
                    self.queue_cover(&m.youtube_id)
                } else {
                    Task::none()
                };
                self.resolutions.insert(key, TrackRes::Done(m));
                self.update_preload();
                let recs = self.maybe_load_recs();
                Task::batch([task, cover, recs])
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
            // Un video suelto (panel de prueba) terminó: radio automática.
            let ended = matches!(self.status.state, PlayerState::Idle)
                && matches!(old.state, PlayerState::Playing | PlayerState::Paused);
            if ended && self.settings.autoplay {
                return self.start_radio(true);
            }
            return Task::none();
        }
        // El motor pasó solo a la pre-cargada (sin corte).
        if self.status.auto_advances > old.auto_advances {
            self.preloaded = None;
            self.failures_in_a_row = 0;
            if let Some(a) = &mut self.active {
                a.queue.advance();
            }
            self.record_history();
            let cover = self
                .now_track()
                .and_then(|t| self.video_of_track(t))
                .map(str::to_owned)
                .map(|v| self.fetch_cover(&v))
                .unwrap_or_else(Task::none);
            let prefetch = self.prefetch();
            self.update_preload();
            return Task::batch([cover, prefetch]);
        }
        match &self.status.state {
            PlayerState::Playing => {
                self.failures_in_a_row = 0;
                self.record_history();
            }
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
            // Se terminó la cola: seguir con parecidas a la última.
            None if self.settings.autoplay => {
                let radio = self.start_radio(true);
                let done = self.finish();
                Task::batch([radio, done])
            }
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
            .now_track()
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
        self.persist_modes();
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
        self.persist_modes();
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
        let key = cache_key(&track);
        self.persist_match(&key, &m);
        self.resolutions.insert(key, TrackRes::Done(m));
        let mut task = Task::none();
        if self.now_in_playlist() == Some(at) {
            self.send(Command::Load(Source::YouTube(video.to_owned())));
            task = self.fetch_cover(video);
        }
        self.update_preload();
        task
    }

    /// Portada de lo que suena: va primera en la cola de portadas.
    pub(crate) fn fetch_cover(&mut self, video: &str) -> Task<Message> {
        self.enqueue_cover(video, true)
    }

    /// Portada para la lista (se baja cuando haya lugar).
    pub(crate) fn queue_cover(&mut self, video: &str) -> Task<Message> {
        self.enqueue_cover(video, false)
    }

    fn enqueue_cover(&mut self, video: &str, first: bool) -> Task<Message> {
        if self.covers.contains_key(video) || self.cover_jobs.failed.contains(video) {
            return Task::none();
        }
        if self.cover_jobs.pending.insert(video.to_owned()) {
            if first {
                self.cover_jobs.queue.push_front(video.to_owned());
            } else {
                self.cover_jobs.queue.push_back(video.to_owned());
            }
        } else if first {
            // Ya estaba en la cola: adelantarla.
            if let Some(i) = self.cover_jobs.queue.iter().position(|v| v == video) {
                if let Some(v) = self.cover_jobs.queue.remove(i) {
                    self.cover_jobs.queue.push_front(v);
                }
            }
        }
        self.pump_covers()
    }

    /// Lanza descargas de portadas hasta `COVER_JOBS` a la vez.
    pub(crate) fn pump_covers(&mut self) -> Task<Message> {
        let Some(rt) = RUNTIME.get() else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        while self.cover_jobs.running < COVER_JOBS {
            let Some(id) = self.cover_jobs.queue.pop_front() else {
                break;
            };
            self.cover_jobs.running += 1;
            let client = self.http.clone();
            let job = rt.spawn(covers::load(client, id.clone()));
            tasks.push(Task::perform(
                async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
                move |r| Message::Cover(id.clone(), r),
            ));
        }
        Task::batch(tasks)
    }

    /// Llegó (o falló) una portada.
    pub(crate) fn on_cover(
        &mut self,
        id: String,
        result: Result<iced::widget::image::Handle, String>,
    ) -> Task<Message> {
        self.cover_jobs.running = self.cover_jobs.running.saturating_sub(1);
        self.cover_jobs.pending.remove(&id);
        match result {
            Ok(handle) => {
                self.covers.insert(id, handle);
            }
            Err(e) => {
                tracing::debug!(%id, error = %e, "sin portada");
                self.cover_jobs.failed.insert(id);
            }
        }
        self.pump_covers()
    }

    /// Con "portadas en la lista", pide las de la playlist abierta (las
    /// canciones ya resueltas; las demás muestran un cuadro vacío).
    pub(crate) fn queue_list_covers(&mut self) -> Task<Message> {
        if !self.settings.covers_in_list {
            return Task::none();
        }
        let Some((p, _)) = self.selected.and_then(|i| self.playlists.get(i)) else {
            return Task::none();
        };
        let videos: Vec<String> = p
            .tracks
            .iter()
            .filter_map(|t| match self.resolution(t) {
                Some(TrackRes::Done(m)) => Some(m.youtube_id.clone()),
                _ => None,
            })
            .collect();
        Task::batch(
            videos
                .iter()
                .map(|v| self.queue_cover(v))
                .collect::<Vec<_>>(),
        )
    }

    /// Encola la búsqueda de las canciones sin video de la playlist abierta
    /// (solo con "portadas en la lista": es para eso).
    pub(crate) fn resolve_in_background(&mut self) -> Task<Message> {
        self.background.queue.clear();
        if !self.settings.covers_in_list || self.resolver.is_none() {
            return Task::none();
        }
        let Some((p, _)) = self.selected.and_then(|i| self.playlists.get(i)) else {
            return Task::none();
        };
        let todo: Vec<Track> = p
            .tracks
            .iter()
            .filter(|t| self.resolution(t).is_none())
            .cloned()
            .collect();
        self.background.queue.extend(todo);
        self.pump_background()
    }

    /// Lanza búsquedas de fondo hasta `BACKGROUND_JOBS`. Espera mientras se
    /// busca la que tiene que sonar, para no demorarla.
    pub(crate) fn pump_background(&mut self) -> Task<Message> {
        if self.background.queue.is_empty()
            || self
                .now_track()
                .is_some_and(|t| matches!(self.resolution(t), Some(TrackRes::Resolving)))
        {
            return Task::none();
        }
        let mut tasks = Vec::new();
        while self.background.running.len() < BACKGROUND_JOBS {
            let Some(t) = self.background.queue.pop_front() else {
                break;
            };
            if self.resolution(&t).is_some() {
                continue;
            }
            self.background.running.insert(cache_key(&t));
            tasks.push(self.spawn_resolve(t));
        }
        Task::batch(tasks)
    }

    /// yt-dlp quedó disponible: armar el resolver.
    pub(crate) fn set_resolver(&mut self, resolver: Option<Arc<Resolver>>) {
        self.resolver = resolver;
    }
}

/// Semilla para el shuffle (no necesita ser criptográfica).
pub(crate) fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
}
