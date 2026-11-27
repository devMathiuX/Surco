//! Motor de audio de Surco.
//!
//! El motor corre en un hilo propio (la salida de audio no es `Send` en todas
//! las plataformas). La GUI le manda [`Command`]s y lee el [`Status`] por un
//! canal `watch`, así nunca bloquea.

pub use surco_core as core;

mod decode;
pub mod eq;
mod output;
pub mod queue;

pub use queue::Queue;

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use surco_core::PlayerState;
use surco_provider::{HttpClient, HttpStream, Provider};
use symphonia::core::io::MediaSource;
use tokio::runtime::Handle;
use tokio::sync::watch;

use decode::{AudioSpec, DecodeEvent, DecodeFailure, RingSource, Shared};

/// Cuántas veces se vuelve a pedir la URL a yt-dlp cuando YouTube la rechaza
/// (403) antes de dar la canción por perdida.
const EXPIRED_RETRIES: u32 = 2;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no se pudo iniciar el motor de audio: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Qué reproducir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// ID o URL de un video de YouTube (se resuelve con yt-dlp).
    YouTube(String),
    /// URL directa a un archivo de audio (sin yt-dlp).
    Url(String),
    /// Archivo local.
    File(PathBuf),
}

impl Source {
    /// Interpreta lo que el usuario pegó: URL de YouTube, ID de video, URL
    /// directa o ruta local.
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }
        if let Some(path) = input.strip_prefix("file://") {
            return Some(Self::File(PathBuf::from(path)));
        }
        let lower = input.to_ascii_lowercase();
        if lower.contains("youtube.com/") || lower.contains("youtu.be/") {
            return Some(Self::YouTube(input.to_owned()));
        }
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Some(Self::Url(input.to_owned()));
        }
        if is_video_id(input) {
            return Some(Self::YouTube(input.to_owned()));
        }
        Some(Self::File(PathBuf::from(input)))
    }
}

/// Los IDs de video de YouTube tienen 11 caracteres de `[A-Za-z0-9_-]`.
fn is_video_id(s: &str) -> bool {
    s.len() == 11
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && !s.contains('.')
}

/// Órdenes para el motor.
#[derive(Debug, Clone)]
pub enum Command {
    Load(Source),
    /// Cargar desde `at` y quedar en pausa o sonando (retomar la última
    /// canción al abrir la app).
    LoadAt {
        source: Source,
        at: Duration,
        paused: bool,
    },
    Play,
    Pause,
    TogglePause,
    Seek(Duration),
    /// Volumen 0.0-1.0 (lineal para la UI; el motor aplica una curva).
    SetVolume(f32),
    Stop,
    /// Pre-cargar lo que sigue (o nada, con `None`). Cuando la actual está por
    /// terminar, se encola en la salida para que no haya silencio entre
    /// canciones; al pasar a ella, `Status::auto_advances` aumenta en 1.
    /// Un `Load` de la misma fuente aprovecha lo ya cargado.
    Preload(Option<Source>),
    /// yt-dlp quedó disponible (por ejemplo, después de descargarlo).
    SetProvider(Arc<Provider>),
}

/// Lo que la GUI necesita mostrar.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: PlayerState,
    pub title: Option<String>,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    /// Lo que está cargado.
    pub source: Option<Source>,
    /// Cuántas veces el motor pasó solo a la canción pre-cargada.
    pub auto_advances: u64,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: PlayerState::Idle,
            title: None,
            position: Duration::ZERO,
            duration: None,
            volume: 0.8,
            source: None,
            auto_advances: 0,
        }
    }
}

/// Mango para hablar con el motor. Clonable y barato.
#[derive(Debug, Clone)]
pub struct PlayerHandle {
    tx: mpsc::Sender<Msg>,
    status: watch::Receiver<Status>,
}

impl PlayerHandle {
    /// Manda una orden. Si el motor murió, se ignora (y queda en el log).
    pub fn send(&self, cmd: Command) {
        if self.tx.send(Msg::Cmd(cmd)).is_err() {
            tracing::error!("el motor de audio no está corriendo");
        }
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }
}

/// Opciones del motor.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Sin dispositivo de audio: consume las muestras en tiempo real y las
    /// descarta. Para pruebas y CI.
    pub null_output: bool,
}

/// Lanza el motor. Las tareas async (yt-dlp, red) corren en `rt`.
pub fn spawn(
    provider: Option<Arc<Provider>>,
    rt: Handle,
    options: Options,
) -> Result<PlayerHandle, Error> {
    let (tx, rx) = mpsc::channel();
    let (status_tx, status_rx) = watch::channel(Status::default());
    let engine_tx = tx.clone();
    std::thread::Builder::new()
        .name("surco-player".into())
        .spawn(move || Engine::new(provider, rt, engine_tx, status_tx, options).run(rx))?;
    Ok(PlayerHandle {
        tx,
        status: status_rx,
    })
}

/// Mensajes internos del hilo del motor.
enum Msg {
    Cmd(Command),
    Opened {
        gen: u64,
        source: Box<dyn MediaSource>,
        ext: Option<String>,
        title: Option<String>,
        duration: Option<Duration>,
    },
    Decode {
        gen: u64,
        event: DecodeEvent,
    },
    LoadFailed {
        gen: u64,
        error: String,
    },
    /// La salida de audio dejó de servir (se desconectó el dispositivo o
    /// cambió su frecuencia, como pasa con los auriculares Bluetooth).
    OutputLost {
        output: u64,
        reason: String,
    },
}

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Msg::Cmd(c) => f.debug_tuple("Cmd").field(c).finish(),
            Msg::Opened { gen, .. } => f.debug_struct("Opened").field("gen", gen).finish(),
            Msg::Decode { gen, event } => f
                .debug_struct("Decode")
                .field("gen", gen)
                .field("event", event)
                .finish(),
            Msg::LoadFailed { gen, error } => f
                .debug_struct("LoadFailed")
                .field("gen", gen)
                .field("error", error)
                .finish(),
            Msg::OutputLost { output, reason } => f
                .debug_struct("OutputLost")
                .field("output", output)
                .field("reason", reason)
                .finish(),
        }
    }
}

/// Lo que está cargado ahora.
struct Current {
    source: Source,
    shared: Option<Arc<Shared>>,
    spec: Option<AudioSpec>,
    /// Re-resoluciones hechas por URL vencida.
    retries: u32,
    /// Cuándo se pidió (para medir cuánto tardó en sonar).
    since: std::time::Instant,
}

/// La canción pre-cargada.
struct Next {
    gen: u64,
    source: Source,
    shared: Option<Arc<Shared>>,
    spec: Option<AudioSpec>,
    title: Option<String>,
    duration: Option<Duration>,
    /// Ya está encolada en la salida, detrás de la actual.
    appended: bool,
    /// Para sacarla de la salida si la actual se descarga antes de terminar.
    cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// Salida de audio: dispositivo real o "nula" para pruebas.
struct Output {
    player: rodio::Player,
    _device: Option<rodio::MixerDeviceSink>,
}

struct Engine {
    provider: Option<Arc<Provider>>,
    client: HttpClient,
    rt: Handle,
    tx: mpsc::Sender<Msg>,
    status_tx: watch::Sender<Status>,
    status: Status,
    output: Result<Output, String>,
    /// Cuántas veces se abrió la salida: descarta avisos de salidas viejas.
    output_gen: u64,
    /// Salida nula (pruebas).
    null_output: bool,
    current: Option<Current>,
    next: Option<Next>,
    /// Generación de la carga actual: descarta resultados de cargas viejas.
    gen: u64,
    /// Contador para generar generaciones (actual y pre-carga).
    seq: u64,
    /// Pausa pedida mientras cargaba.
    paused: bool,
}

impl Engine {
    fn new(
        provider: Option<Arc<Provider>>,
        rt: Handle,
        tx: mpsc::Sender<Msg>,
        status_tx: watch::Sender<Status>,
        options: Options,
    ) -> Self {
        let output = open_output(options.null_output, on_output_error(tx.clone(), 1));
        let status = Status::default();
        if let Ok(out) = &output {
            out.player.set_volume(volume_curve(status.volume));
        }
        if let Err(e) = &output {
            tracing::error!(error = %e, "sin salida de audio");
        }
        Self {
            output_gen: 1,
            null_output: options.null_output,
            provider,
            client: surco_provider::stream_client(),
            rt,
            tx,
            status_tx,
            status,
            output,
            current: None,
            next: None,
            gen: 0,
            seq: 0,
            paused: false,
        }
    }

    fn run(mut self, rx: mpsc::Receiver<Msg>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick();
            self.publish();
        }
        self.unload();
        tracing::debug!("motor de audio terminado");
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Cmd(cmd) => self.command(cmd),
            Msg::Opened {
                gen,
                source,
                ext,
                title,
                duration,
            } if gen == self.gen => {
                if title.is_some() {
                    self.status.title = title;
                }
                if duration.is_some() {
                    self.status.duration = duration;
                }
                let start_at =
                    (self.status.position > Duration::ZERO).then_some(self.status.position);
                let tx = self.tx.clone();
                let on_event = move |event| {
                    let _ = tx.send(Msg::Decode { gen, event });
                };
                match decode::spawn(source, ext, start_at, on_event) {
                    Ok(shared) => {
                        if let Some(cur) = &mut self.current {
                            cur.shared = Some(shared);
                        }
                    }
                    Err(e) => self.fail(format!("no se pudo iniciar el decodificador: {e}")),
                }
            }
            Msg::Decode { gen, event } if gen == self.gen => match event {
                DecodeEvent::Ready { spec, duration } => self.start(spec, duration),
                DecodeEvent::Failed(DecodeFailure::Expired) => self.retry_expired(),
                DecodeEvent::Failed(DecodeFailure::Other(e)) => self.fail(e),
            },
            Msg::LoadFailed { gen, error } if gen == self.gen => self.fail(error),
            Msg::Opened {
                gen,
                source,
                ext,
                title,
                duration,
            } if self.next_gen() == Some(gen) => {
                if let Some(n) = &mut self.next {
                    n.title = title;
                    n.duration = duration;
                }
                self.next_opened(gen, source, ext);
            }
            Msg::Decode { gen, event } if self.next_gen() == Some(gen) => match event {
                DecodeEvent::Ready { spec, duration } => {
                    if let Some(n) = &mut self.next {
                        n.spec = Some(spec);
                        if duration.is_some() {
                            n.duration = duration;
                        }
                    }
                    self.maybe_append_next();
                }
                DecodeEvent::Failed(e) => {
                    tracing::warn!(error = %e, "falló la pre-carga");
                    self.drop_next();
                }
            },
            Msg::LoadFailed { gen, error } if self.next_gen() == Some(gen) => {
                tracing::warn!(%error, "falló la pre-carga");
                self.drop_next();
            }
            Msg::OutputLost { output, reason } if output == self.output_gen => {
                self.output_lost(&reason);
            }
            stale => tracing::debug!(?stale, "mensaje de una carga anterior, se ignora"),
        }
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Load(source) if self.next.as_ref().is_some_and(|n| n.source == source) => {
                self.promote_next(false);
            }
            Command::Load(source) => self.command(Command::LoadAt {
                source,
                at: Duration::ZERO,
                paused: false,
            }),
            Command::LoadAt { source, at, paused } => {
                self.unload();
                self.status.position = at;
                self.status.duration = None;
                self.status.title = None;
                self.paused = paused;
                self.status.source = Some(source.clone());
                self.load(source, 0, false);
            }
            Command::Play => self.set_paused(false),
            Command::Pause => self.set_paused(true),
            Command::TogglePause => {
                if matches!(self.status.state, PlayerState::Idle) {
                    // Reproducir de nuevo lo último si terminó.
                    if let Some(source) = self.current.as_ref().map(|c| c.source.clone()) {
                        self.command(Command::Load(source));
                    }
                } else {
                    self.set_paused(!self.paused);
                }
            }
            Command::Seek(to) => self.seek(to),
            Command::SetVolume(v) => {
                let v = v.clamp(0.0, 1.0);
                self.status.volume = v;
                if let Ok(out) = &self.output {
                    out.player.set_volume(volume_curve(v));
                }
            }
            Command::SetProvider(p) => self.provider = Some(p),
            Command::Preload(source) => self.preload(source),
            Command::Stop => {
                self.unload();
                self.drop_next();
                self.current = None;
                self.status = Status {
                    volume: self.status.volume,
                    auto_advances: self.status.auto_advances,
                    ..Status::default()
                };
            }
        }
    }

    /// Resuelve y abre la fuente en segundo plano. Responde con `Msg::Opened`.
    fn load(&mut self, source: Source, retries: u32, refresh: bool) {
        self.seq += 1;
        self.gen = self.seq;
        let gen = self.gen;
        self.status.state = PlayerState::Loading;
        self.current = Some(Current {
            source: source.clone(),
            shared: None,
            spec: None,
            retries,
            since: std::time::Instant::now(),
        });
        // Si al arrancar no había salida (o se perdió), se prueba de nuevo:
        // quizás ya se conectaron los auriculares.
        if !self.ensure_output() {
            let e = self.output.as_ref().err().cloned().unwrap_or_default();
            self.fail(format!("no hay salida de audio: {e}"));
            return;
        }

        let tx = self.tx.clone();
        let provider = self.provider.clone();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let msg = match open_source(source, provider, client, refresh).await {
                Ok(opened) => Msg::Opened {
                    gen,
                    source: opened.source,
                    ext: opened.ext,
                    title: opened.title,
                    duration: opened.duration,
                },
                Err(error) => Msg::LoadFailed { gen, error },
            };
            let _ = tx.send(msg);
        });
    }

    /// La URL venció (403) a mitad de canción: re-resolver y seguir desde donde estaba.
    fn retry_expired(&mut self) {
        let Some(cur) = &self.current else { return };
        if cur.retries >= EXPIRED_RETRIES {
            self.fail("la URL del stream venció y no se pudo renovar".into());
            return;
        }
        let source = cur.source.clone();
        let retries = cur.retries + 1;
        if let (Source::YouTube(video), Some(p)) = (&source, &self.provider) {
            p.invalidate(video);
        }
        tracing::info!("URL vencida, re-resolviendo");
        let position = self.status.position;
        self.unload();
        self.status.position = position;
        self.load(source, retries, true);
    }

    /// Abre la salida si no hay una. Devuelve si quedó abierta.
    fn ensure_output(&mut self) -> bool {
        if self.output.is_ok() {
            return true;
        }
        self.output_gen += 1;
        self.output = open_output(
            self.null_output,
            on_output_error(self.tx.clone(), self.output_gen),
        );
        match &self.output {
            Ok(out) => {
                out.player.set_volume(volume_curve(self.status.volume));
                tracing::warn!("salida de audio recuperada");
                true
            }
            Err(e) => {
                tracing::error!(error = %e, "sigue sin haber salida de audio");
                false
            }
        }
    }

    /// La salida dejó de andar: se abre otra y la canción sigue donde estaba.
    fn output_lost(&mut self, reason: &str) {
        tracing::warn!(%reason, "se perdió la salida de audio, reabriendo");
        let position = self.status.position;
        let resume = matches!(
            self.status.state,
            PlayerState::Playing | PlayerState::Paused | PlayerState::Loading
        );
        self.unload();
        // Primero se cierra la vieja: en macOS el dispositivo puede seguir
        // tomado hasta que se suelta.
        self.output = Err(reason.to_owned());
        if !self.ensure_output() {
            if resume {
                let e = self.output.as_ref().err().cloned().unwrap_or_default();
                self.fail(format!("no hay salida de audio: {e}"));
            }
            return;
        }
        if !resume {
            return;
        }
        if let Some(cur) = &self.current {
            let source = cur.source.clone();
            let retries = cur.retries;
            self.status.position = position;
            self.load(source, retries, false);
        }
    }

    fn start(&mut self, spec: AudioSpec, duration: Option<Duration>) {
        let Some(cur) = &mut self.current else { return };
        let Some(shared) = &cur.shared else { return };
        let Ok(out) = &self.output else { return };
        let Some(ring) = RingSource::new(Arc::clone(shared), spec) else {
            self.fail("formato de audio inválido".into());
            return;
        };
        cur.spec = Some(spec);
        if duration.is_some() {
            self.status.duration = duration;
        }
        out.player.append(ring);
        if self.paused {
            out.player.pause();
            self.status.state = PlayerState::Paused;
        } else {
            out.player.play();
            self.status.state = PlayerState::Playing;
        }
        tracing::info!(
            rate = spec.rate,
            channels = spec.channels,
            ?duration,
            ms = cur.since.elapsed().as_millis() as u64,
            "reproduciendo"
        );
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        let Ok(out) = &self.output else { return };
        match self.status.state {
            PlayerState::Playing | PlayerState::Paused => {
                if paused {
                    out.player.pause();
                    self.status.state = PlayerState::Paused;
                } else {
                    out.player.play();
                    self.status.state = PlayerState::Playing;
                }
            }
            _ => {}
        }
    }

    fn seek(&mut self, to: Duration) {
        let to = match self.status.duration {
            Some(d) => to.min(d.saturating_sub(Duration::from_millis(500))),
            None => to,
        };
        match self.current.as_ref().and_then(|c| c.shared.as_ref()) {
            Some(shared) => {
                shared.request_seek(to);
                self.status.position = to;
            }
            // Todavía cargando: se aplica al arrancar.
            None => self.status.position = to,
        }
    }

    fn tick(&mut self) {
        let Some(cur) = &self.current else { return };
        let (Some(shared), Some(spec)) = (&cur.shared, cur.spec) else {
            return;
        };
        if !matches!(
            self.status.state,
            PlayerState::Playing | PlayerState::Paused
        ) {
            return;
        }
        self.status.position = spec.duration_of(shared.played_samples());
        let empty = self
            .output
            .as_ref()
            .map(|o| o.player.empty())
            .unwrap_or(true);
        if shared.drained() && self.next.as_ref().is_some_and(|n| n.appended) {
            self.promote_next(true);
            return;
        }
        let drained = shared.drained();
        self.maybe_append_next();
        if drained && empty {
            tracing::info!("fin de la canción");
            self.unload();
            self.status.state = PlayerState::Idle;
            self.status.position = Duration::ZERO;
        }
    }

    fn fail(&mut self, error: String) {
        tracing::warn!(%error, "error de reproducción");
        self.unload();
        self.status.state = PlayerState::Error(error);
    }

    /// Detiene el decodificador y saca la canción de la salida (sin olvidar
    /// `current`).
    ///
    /// No usa `Player::clear()` de rodio: marca cuántas fuentes saltear y se
    /// aplica después, así que dos `clear()` seguidos (cargar una canción
    /// mientras otra todavía abría, un 403 que re-resuelve…) podían dejar un
    /// salteo pendiente que se comía la canción siguiente: quedaba "sonando"
    /// en 0:00 sin audio. Cada fuente nuestra termina sola al ver su bandera.
    fn unload(&mut self) {
        if let Some(shared) = self.current.as_mut().and_then(|c| c.shared.take()) {
            shared.stop();
        }
        if let Some(cur) = &mut self.current {
            cur.spec = None;
        }
        // La pre-carga encolada detrás sale de la salida, pero sigue lista.
        if let Some(n) = &mut self.next {
            if let Some(cancel) = n.cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            n.appended = false;
        }
        if let Ok(out) = &self.output {
            out.player.play();
        }
    }

    fn next_gen(&self) -> Option<u64> {
        self.next.as_ref().map(|n| n.gen)
    }

    fn preload(&mut self, source: Option<Source>) {
        if self.next.as_ref().map(|n| &n.source) == source.as_ref() {
            return;
        }
        self.drop_next();
        let Some(source) = source else { return };
        self.seq += 1;
        let gen = self.seq;
        tracing::debug!(?source, "pre-cargando");
        self.next = Some(Next {
            gen,
            source: source.clone(),
            shared: None,
            spec: None,
            title: None,
            duration: None,
            appended: false,
            cancel: None,
        });
        let tx = self.tx.clone();
        let provider = self.provider.clone();
        let client = self.client.clone();
        self.rt.spawn(async move {
            let msg = match open_source(source, provider, client, false).await {
                Ok(opened) => Msg::Opened {
                    gen,
                    source: opened.source,
                    ext: opened.ext,
                    title: opened.title,
                    duration: opened.duration,
                },
                Err(error) => Msg::LoadFailed { gen, error },
            };
            let _ = tx.send(msg);
        });
    }

    fn next_opened(&mut self, gen: u64, source: Box<dyn MediaSource>, ext: Option<String>) {
        let tx = self.tx.clone();
        let on_event = move |event| {
            let _ = tx.send(Msg::Decode { gen, event });
        };
        match decode::spawn(source, ext, None, on_event) {
            Ok(shared) => {
                if let Some(n) = &mut self.next {
                    n.shared = Some(shared);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo pre-cargar");
                self.drop_next();
            }
        }
    }

    /// Encola la pre-carga detrás de la actual cuando a esta solo le queda
    /// lo que ya está en el buffer (el decodificador llegó al final).
    fn maybe_append_next(&mut self) {
        let current_finishing = self
            .current
            .as_ref()
            .and_then(|c| c.shared.as_ref())
            .is_some_and(|s| s.finished());
        let active = matches!(
            self.status.state,
            PlayerState::Playing | PlayerState::Paused
        );
        let Some(n) = &mut self.next else { return };
        if n.appended || !current_finishing || !active {
            return;
        }
        let (Some(shared), Some(spec), Ok(out)) = (&n.shared, n.spec, &self.output) else {
            return;
        };
        if let Some(ring) = RingSource::new(Arc::clone(shared), spec) {
            n.cancel = Some(ring.cancel_handle());
            out.player.append(ring);
            n.appended = true;
            tracing::debug!("pre-carga encolada");
        }
    }

    fn drop_next(&mut self) {
        if let Some(n) = self.next.take() {
            // Si estaba encolada, su fuente termina sola al ver `stop`.
            if let Some(s) = n.shared {
                s.stop();
            }
        }
    }

    /// Pasa a la canción pre-cargada: sola (`auto`, sin corte) o porque se
    /// pidió cargar justo esa.
    fn promote_next(&mut self, auto: bool) {
        let Some(n) = self.next.take() else { return };
        // Sin `clear()`: la fuente vieja termina sola al ver `stop` y la salida
        // sigue con lo que venga detrás. (`clear()` de rodio se aplica de forma
        // diferida y se saltearía la fuente que encolemos enseguida.)
        if let Some(old) = self.current.as_mut().and_then(|c| c.shared.take()) {
            old.stop();
        }
        if auto {
            self.status.auto_advances += 1;
            tracing::info!("siguiente canción sin corte");
        } else {
            self.paused = false;
        }
        self.gen = n.gen;
        self.status.title = n.title;
        self.status.duration = n.duration;
        self.status.position = Duration::ZERO;
        self.status.source = Some(n.source.clone());
        self.current = Some(Current {
            source: n.source,
            shared: n.shared,
            spec: if n.appended { n.spec } else { None },
            retries: 0,
            since: std::time::Instant::now(),
        });
        if n.appended {
            // Ya estaba encolada detrás de la vieja: suena enseguida.
            if let Ok(out) = &self.output {
                out.player.play();
            }
            self.status.state = PlayerState::Playing;
            return;
        }
        // Carga pedida a mano: arrancar con lo que ya hay.
        match (
            n.spec,
            self.current.as_ref().is_some_and(|c| c.shared.is_some()),
        ) {
            (Some(spec), true) => self.start(spec, self.status.duration),
            // Abierta pero todavía sin buffer suficiente, o sin abrir: llega el evento.
            _ => self.status.state = PlayerState::Loading,
        }
    }

    fn publish(&self) {
        self.status_tx.send_if_modified(|s| {
            if *s != self.status {
                *s = self.status.clone();
                true
            } else {
                false
            }
        });
    }
}

/// Curva de volumen perceptual (aprox. logarítmica).
fn volume_curve(v: f32) -> f32 {
    v * v
}

/// Avisa al motor cuando la salida queda inservible. El resto de los errores
/// (cortes por carga, cambio de ruta que el sistema ya resolvió) solo se anotan.
fn on_output_error(
    tx: mpsc::Sender<Msg>,
    output: u64,
) -> impl FnMut(rodio::cpal::Error) + Clone + Send + 'static {
    move |e: rodio::cpal::Error| {
        use rodio::cpal::ErrorKind;
        match e.kind() {
            ErrorKind::StreamInvalidated | ErrorKind::DeviceNotAvailable => {
                let _ = tx.send(Msg::OutputLost {
                    output,
                    reason: e.to_string(),
                });
            }
            ErrorKind::DeviceChanged => tracing::info!(error = %e, "cambió la salida de audio"),
            _ => tracing::debug!(error = %e, "aviso de la salida de audio"),
        }
    }
}

fn open_output<F>(null_output: bool, on_error: F) -> Result<Output, String>
where
    F: FnMut(rodio::cpal::Error) + Clone + Send + 'static,
{
    if null_output {
        let (player, queue) = rodio::Player::new();
        std::thread::Builder::new()
            .name("surco-null-audio".into())
            .spawn(move || null_sink(queue))
            .map_err(|e| e.to_string())?;
        return Ok(Output {
            player,
            _device: None,
        });
    }
    let mut device = output::open(on_error)?;
    device.log_on_drop(false);
    let player = rodio::Player::connect_new(device.mixer());
    Ok(Output {
        player,
        _device: Some(device),
    })
}

/// Consume la cola a velocidad real y descarta las muestras.
///
/// El ritmo sale del reloj y no de cuántas veces durmió el hilo: en macOS (y
/// en máquinas cargadas) `sleep(10 ms)` puede tardar bastante más, y contar
/// ticks hacía que la "reproducción" fuera más lenta que el tiempo real.
fn null_sink(mut queue: rodio::queue::SourcesQueueOutput) {
    use rodio::Source;
    let tick = Duration::from_millis(10);
    // Tope por vuelta, para no "reproducir" de golpe después de una pausa larga del hilo.
    let max_step = Duration::from_millis(100);
    let mut last = Instant::now();
    let mut owed = 0.0_f64;
    loop {
        std::thread::sleep(tick);
        let now = Instant::now();
        let step = now.duration_since(last).min(max_step);
        last = now;
        let rate = queue.sample_rate().get() as f64 * queue.channels().get() as f64;
        owed += step.as_secs_f64() * rate;
        let due = owed.floor();
        owed -= due;
        for _ in 0..due as u64 {
            if queue.next().is_none() {
                break;
            }
        }
    }
}

struct Opened {
    source: Box<dyn MediaSource>,
    ext: Option<String>,
    title: Option<String>,
    duration: Option<Duration>,
}

async fn open_source(
    source: Source,
    provider: Option<Arc<Provider>>,
    client: HttpClient,
    refresh: bool,
) -> Result<Opened, String> {
    match source {
        Source::File(path) => {
            let file =
                std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Opened {
                ext: path.extension().and_then(|e| e.to_str()).map(str::to_owned),
                title: path.file_stem().and_then(|s| s.to_str()).map(str::to_owned),
                source: Box::new(file),
                duration: None,
            })
        }
        Source::Url(url) => {
            let ext = url
                .split(['?', '#'])
                .next()
                .and_then(|u| u.rsplit_once('.'))
                .map(|(_, e)| e.to_owned());
            let stream = HttpStream::open(client, url)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Opened {
                source: Box::new(StreamSource(stream)),
                ext,
                title: None,
                duration: None,
            })
        }
        Source::YouTube(video) => {
            let provider = provider.ok_or("yt-dlp no está instalado")?;
            // Precargada en disco: no hace falta ni yt-dlp ni la red.
            if let Some(path) = provider.cached_audio(&video) {
                match std::fs::File::open(&path) {
                    Ok(file) => {
                        tracing::debug!(video, "audio desde la caché de disco");
                        return Ok(Opened {
                            source: Box::new(file),
                            ext: Some("m4a".into()),
                            title: None,
                            duration: None,
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "no se pudo abrir el audio precargado"),
                }
            }
            let mut refresh = refresh;
            for attempt in 0..=EXPIRED_RETRIES {
                let t0 = std::time::Instant::now();
                let info = provider
                    .stream_info(&video, refresh)
                    .await
                    .map_err(|e| e.to_string())?;
                let url_ms = t0.elapsed().as_millis() as u64;
                if !info.is_supported() {
                    return Err(format!(
                        "YouTube solo ofrece este audio en formato {}, que todavía no está soportado",
                        info.ext
                    ));
                }
                match HttpStream::open_with(
                    client.clone(),
                    info.url.clone(),
                    info.user_agent.clone(),
                )
                .await
                {
                    Ok(stream) => {
                        tracing::info!(
                            video,
                            url_ms,
                            first_chunk_ms = t0.elapsed().as_millis() as u64 - url_ms,
                            "audio abierto"
                        );
                        // Bajada entera, queda en la caché de disco: si se
                        // vuelve a escuchar (o se retoma al abrir) no espera.
                        if let Some(cache) = provider.audio_cache().cloned() {
                            let video = video.clone();
                            stream.on_complete(move |data| {
                                if !cache.contains(&video) {
                                    match cache.put(&video, data) {
                                        Ok(_) => tracing::debug!(video, "audio escuchado guardado en disco"),
                                        Err(e) => tracing::debug!(video, error = %e, "no se pudo guardar el audio escuchado"),
                                    }
                                }
                            });
                        }
                        return Ok(Opened {
                            source: Box::new(StreamSource(stream)),
                            ext: Some(info.ext),
                            title: info.title,
                            duration: info.duration,
                        });
                    }
                    Err(surco_provider::Error::Expired) if attempt < EXPIRED_RETRIES => {
                        tracing::info!(attempt, "URL rechazada al abrir, re-resolviendo");
                        provider.invalidate(&video);
                        refresh = true;
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                    Err(surco_provider::Error::Expired) => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            // YouTube sigue rechazando la URL: que la baje yt-dlp entera.
            let path = provider.download_with_ytdlp(&video).await.map_err(|e| {
                format!("la URL del stream venció y yt-dlp tampoco pudo bajarla: {e}")
            })?;
            let file =
                std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Opened {
                source: Box::new(file),
                ext: Some("m4a".into()),
                title: None,
                duration: None,
            })
        }
    }
}

/// Adaptador de [`HttpStream`] a `MediaSource` de symphonia.
struct StreamSource(HttpStream);

impl std::io::Read for StreamSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Seek for StreamSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl MediaSource for StreamSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sources() {
        assert_eq!(
            Source::parse(" https://www.youtube.com/watch?v=aqz-KE-bpKQ "),
            Some(Source::YouTube(
                "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into()
            ))
        );
        assert_eq!(
            Source::parse("https://youtu.be/aqz-KE-bpKQ"),
            Some(Source::YouTube("https://youtu.be/aqz-KE-bpKQ".into()))
        );
        assert_eq!(
            Source::parse("aqz-KE-bpKQ"),
            Some(Source::YouTube("aqz-KE-bpKQ".into()))
        );
        assert_eq!(
            Source::parse("http://localhost:8000/a.m4a"),
            Some(Source::Url("http://localhost:8000/a.m4a".into()))
        );
        assert_eq!(
            Source::parse("file:///tmp/a.m4a"),
            Some(Source::File("/tmp/a.m4a".into()))
        );
        assert_eq!(
            Source::parse("song.m4a"),
            Some(Source::File("song.m4a".into()))
        );
        assert_eq!(Source::parse("   "), None);
    }

    #[test]
    fn volume_curve_bounds() {
        assert_eq!(volume_curve(0.0), 0.0);
        assert_eq!(volume_curve(1.0), 1.0);
        assert!(volume_curve(0.5) < 0.5);
    }
}
