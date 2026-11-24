//! Motor de audio de Surco.
//!
//! El motor corre en un hilo propio (la salida de audio no es `Send` en todas
//! las plataformas). La GUI le manda [`Command`]s y lee el [`Status`] por un
//! canal `watch`, así nunca bloquea.

pub use surco_core as core;

mod decode;

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use surco_core::PlayerState;
use surco_provider::{HttpClient, HttpStream, Provider};
use symphonia::core::io::MediaSource;
use tokio::runtime::Handle;
use tokio::sync::watch;

use decode::{AudioSpec, DecodeEvent, DecodeFailure, RingSource, Shared};

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
    Play,
    Pause,
    TogglePause,
    Seek(Duration),
    /// Volumen 0.0-1.0 (lineal para la UI; el motor aplica una curva).
    SetVolume(f32),
    Stop,
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
}

impl Default for Status {
    fn default() -> Self {
        Self {
            state: PlayerState::Idle,
            title: None,
            position: Duration::ZERO,
            duration: None,
            volume: 0.8,
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
    current: Option<Current>,
    /// Generación de carga: descarta resultados de cargas viejas.
    gen: u64,
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
        let output = open_output(options.null_output);
        let status = Status::default();
        if let Ok(out) = &output {
            out.player.set_volume(volume_curve(status.volume));
        }
        if let Err(e) = &output {
            tracing::error!(error = %e, "sin salida de audio");
        }
        Self {
            provider,
            client: HttpClient::new(),
            rt,
            tx,
            status_tx,
            status,
            output,
            current: None,
            gen: 0,
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
            stale => tracing::debug!(?stale, "mensaje de una carga anterior, se ignora"),
        }
    }

    fn command(&mut self, cmd: Command) {
        match cmd {
            Command::Load(source) => {
                self.unload();
                self.status.position = Duration::ZERO;
                self.status.duration = None;
                self.status.title = None;
                self.paused = false;
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
            Command::Stop => {
                self.unload();
                self.current = None;
                self.status = Status {
                    volume: self.status.volume,
                    ..Status::default()
                };
            }
        }
    }

    /// Resuelve y abre la fuente en segundo plano. Responde con `Msg::Opened`.
    fn load(&mut self, source: Source, retries: u32, refresh: bool) {
        self.gen += 1;
        let gen = self.gen;
        self.status.state = PlayerState::Loading;
        self.current = Some(Current {
            source: source.clone(),
            shared: None,
            spec: None,
            retries,
        });
        if let Err(e) = &self.output {
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
        if cur.retries >= 1 {
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
        if shared.drained() && empty {
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

    /// Detiene el decodificador y vacía la salida (sin olvidar `current`).
    fn unload(&mut self) {
        if let Some(shared) = self.current.as_mut().and_then(|c| c.shared.take()) {
            shared.stop();
        }
        if let Some(cur) = &mut self.current {
            cur.spec = None;
        }
        if let Ok(out) = &self.output {
            out.player.clear();
        }
        // `clear()` deja la salida en pausa: la reactivamos para la próxima.
        if let Ok(out) = &self.output {
            out.player.play();
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

fn open_output(null_output: bool) -> Result<Output, String> {
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
    let mut device = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?;
    device.log_on_drop(false);
    let player = rodio::Player::connect_new(device.mixer());
    Ok(Output {
        player,
        _device: Some(device),
    })
}

/// Consume la cola a velocidad real y descarta las muestras.
fn null_sink(mut queue: rodio::queue::SourcesQueueOutput) {
    use rodio::Source;
    let tick = Duration::from_millis(10);
    loop {
        let per_tick = (queue.sample_rate().get() as usize * queue.channels().get() as usize) / 100;
        for _ in 0..per_tick {
            if queue.next().is_none() {
                break;
            }
        }
        std::thread::sleep(tick);
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
            let mut refresh = refresh;
            for attempt in 0..2 {
                let info = provider
                    .stream_info(&video, refresh)
                    .await
                    .map_err(|e| e.to_string())?;
                if !info.is_supported() {
                    return Err(format!(
                        "YouTube solo ofrece este audio en formato {}, que todavía no está soportado",
                        info.ext
                    ));
                }
                match HttpStream::open(client.clone(), info.url.clone()).await {
                    Ok(stream) => {
                        return Ok(Opened {
                            source: Box::new(StreamSource(stream)),
                            ext: Some(info.ext),
                            title: info.title,
                            duration: info.duration,
                        })
                    }
                    Err(surco_provider::Error::Expired) if attempt == 0 => {
                        provider.invalidate(&video);
                        refresh = true;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err("la URL del stream venció".into())
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
