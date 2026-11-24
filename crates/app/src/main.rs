//! Binario de Surco: GUI con iced.

// En Windows, release sin ventana de consola.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod bar;
mod config;
mod covers;
mod icons;
mod media;
mod playback;
mod theme;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use iced::keyboard::{self, key, Key};
use iced::widget::{image, operation, Id};
use iced::{window, Size, Subscription, Task, Theme};
use surco_core::{PlayerState, Playlist, RepeatMode};
use surco_player::{Command, PlayerHandle, Source, Status};
use surco_provider::{HttpClient, Provider, YtDlp};
use surco_resolver::{Match, Resolver};
use surco_spotify::{Credentials, Importer};

use playback::{Active, TrackRes};

/// Video de prueba de la Fase 2: "Big Buck Bunny" (Blender Foundation, CC BY 3.0).
const TEST_URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const IMPORT_INPUT: &str = "import-input";
const FILTER_INPUT: &str = "filter-input";
const SEEK_STEP: f32 = 5.0;

/// Runtime de tokio para la red y los procesos hijos (yt-dlp).
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn main() -> anyhow::Result<()> {
    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "iniciando Surco");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("surco-rt")
        .enable_all()
        .build()?;
    let _ = RUNTIME.set(rt);

    iced::application(Surco::new, Surco::update, Surco::view)
        .title("Surco")
        .theme(Surco::theme)
        .subscription(Surco::subscription)
        .window(window::Settings {
            size: Size::new(1000.0, 640.0),
            min_size: Some(Size::new(800.0, 500.0)),
            ..window::Settings::default()
        })
        .run()?;

    Ok(())
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    // Por ahora a stderr. El archivo rotativo llega con la persistencia (Fase 6).
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("surco=info,warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Surco").map(|d| d.data_dir().to_path_buf())
}

#[derive(Debug, Clone)]
enum YtDlpState {
    Found(PathBuf),
    Missing,
    Downloading,
    Failed(String),
}

struct Surco {
    player: Option<PlayerHandle>,
    status: Status,
    input: String,
    ytdlp: YtDlpState,
    /// Valor del slider mientras se arrastra (segundos).
    seeking: Option<f32>,
    fatal: Option<String>,
    spotify_creds: Option<Credentials>,
    http: HttpClient,
    /// Playlists importadas en esta sesión (se guardan en disco en la Fase 6),
    /// con la estrategia que funcionó.
    playlists: Vec<(Playlist, &'static str)>,
    selected: Option<usize>,
    import_input: String,
    importing: bool,
    notice: Option<Notice>,
    /// `None` si no hay yt-dlp.
    resolver: Option<Arc<Resolver>>,
    /// Resoluciones por clave de canción (`surco_resolver::cache_key`).
    resolutions: HashMap<String, TrackRes>,
    /// Cola de reproducción activa.
    active: Option<Active>,
    shuffle: bool,
    repeat: RepeatMode,
    /// Lo último que se le pidió pre-cargar al motor.
    preloaded: Option<Source>,
    /// Canción cuyo selector de video está abierto.
    choosing: Option<(usize, usize)>,
    failures_in_a_row: u32,
    /// Portadas chicas por ID de video.
    covers: HashMap<String, image::Handle>,
    /// Búsqueda dentro de la playlist (Ctrl+F); `None` = cerrada.
    filter: Option<String>,
    /// MPRIS / SMTC.
    media: Option<media::Media>,
}

/// Aviso discreto en la barra lateral.
#[derive(Debug, Clone)]
struct Notice {
    error: bool,
    text: String,
}

type ImportResult = Result<surco_spotify::Imported, String>;

#[derive(Debug, Clone)]
enum Message {
    InputChanged(String),
    PlayInput,
    Play,
    Pause,
    TogglePause,
    SeekPreview(f32),
    SeekCommit,
    /// Segundos hacia adelante (o atrás, si es negativo).
    SeekRelative(f32),
    SeekTo(Duration),
    Volume(f32),
    Status(Status),
    DownloadYtDlp,
    YtDlpDownloaded(Result<PathBuf, String>),
    ImportInputChanged(String),
    Import,
    PickCsv,
    CsvPicked(Option<PathBuf>),
    Imported(ImportResult),
    SelectPlaylist(Option<usize>),
    FocusImport,
    PlayTrack(usize, usize),
    Resolved(String, Result<Match, String>),
    /// Abrir (o cerrar, con `None`) el selector de video de una canción.
    OpenChooser(Option<(usize, usize)>),
    ChooseVideo(String),
    /// Siguiente (`1`) o anterior (`-1`).
    Skip(isize),
    ToggleShuffle,
    CycleRepeat,
    Cover(String, Result<image::Handle, String>),
    OpenFilter,
    FilterChanged(String),
    CloseFilter,
    /// HWND de la ventana (en Windows) para los controles del sistema.
    WindowHandle(Option<isize>),
}

impl Surco {
    fn new() -> (Self, Task<Message>) {
        let ytdlp = YtDlp::locate(data_dir().as_deref());
        let resolver = ytdlp.clone().map(|y| Arc::new(Resolver::new(Arc::new(y))));
        let provider = ytdlp.clone().map(|y| Arc::new(Provider::new(y)));
        let options = surco_player::Options {
            null_output: std::env::var_os("SURCO_NULL_AUDIO").is_some(),
        };
        let (player, fatal) = match RUNTIME.get() {
            Some(rt) => match surco_player::spawn(provider, rt.handle().clone(), options) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(e.to_string())),
            },
            None => (None, Some("runtime no inicializado".to_owned())),
        };
        let state = Self {
            status: player
                .as_ref()
                .map(PlayerHandle::status)
                .unwrap_or_default(),
            player,
            input: TEST_URL.to_owned(),
            ytdlp: match ytdlp {
                Some(y) => YtDlpState::Found(y.path().to_path_buf()),
                None => YtDlpState::Missing,
            },
            seeking: None,
            fatal,
            spotify_creds: config::Config::load().spotify_credentials(),
            http: HttpClient::new(),
            playlists: Vec::new(),
            selected: None,
            import_input: String::new(),
            importing: false,
            notice: None,
            resolver,
            resolutions: HashMap::new(),
            active: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            preloaded: None,
            choosing: None,
            failures_in_a_row: 0,
            covers: HashMap::new(),
            filter: None,
            media: None,
        };
        // Los controles del sistema necesitan la ventana (HWND en Windows).
        let handle = window::oldest()
            .then(|id| match id {
                Some(id) => window::run(id, media::hwnd_of),
                None => Task::done(None),
            })
            .map(Message::WindowHandle);
        (state, handle)
    }

    fn send(&self, cmd: Command) {
        if let Some(p) = &self.player {
            p.send(cmd);
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.sync_media();
        task
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(s) => self.input = s,
            Message::PlayInput => {
                if let Some(source) = Source::parse(&self.input) {
                    self.active = None;
                    self.send(Command::Load(source));
                }
            }
            Message::Play => {
                if matches!(self.status.state, PlayerState::Paused) {
                    self.send(Command::Play);
                } else if !matches!(self.status.state, PlayerState::Playing) {
                    return self.toggle_pause();
                }
            }
            Message::Pause => self.send(Command::Pause),
            Message::TogglePause => return self.toggle_pause(),
            Message::SeekPreview(v) => self.seeking = Some(v),
            Message::SeekCommit => {
                if let Some(v) = self.seeking.take() {
                    self.send(Command::Seek(Duration::from_secs_f32(v.max(0.0))));
                }
            }
            Message::SeekRelative(secs) => {
                let pos = (self.status.position.as_secs_f32() + secs).max(0.0);
                self.send(Command::Seek(Duration::from_secs_f32(pos)));
            }
            Message::SeekTo(d) => self.send(Command::Seek(d)),
            Message::Volume(v) => {
                self.status.volume = v;
                self.send(Command::SetVolume(v));
            }
            Message::Status(status) => return self.on_status(status),
            Message::DownloadYtDlp => return self.download_ytdlp(),
            Message::YtDlpDownloaded(Ok(path)) => {
                self.set_resolver(Some(Arc::new(Resolver::new(Arc::new(YtDlp::new(&path))))));
                self.send(Command::SetProvider(Arc::new(Provider::new(YtDlp::new(
                    &path,
                )))));
                self.ytdlp = YtDlpState::Found(path);
            }
            Message::YtDlpDownloaded(Err(e)) => self.ytdlp = YtDlpState::Failed(e),
            Message::ImportInputChanged(s) => self.import_input = s,
            Message::Import => return self.start_import(),
            Message::PickCsv => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Importar CSV de Exportify")
                            .add_filter("CSV", &["csv"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::CsvPicked,
                );
            }
            Message::CsvPicked(Some(path)) => {
                self.import_input = path.display().to_string();
                return self.start_import();
            }
            Message::CsvPicked(None) => {}
            Message::Imported(result) => self.imported(result),
            Message::SelectPlaylist(i) => {
                self.selected = i;
                self.filter = None;
            }
            Message::FocusImport => return operation::focus(Id::new(IMPORT_INPUT)),
            Message::PlayTrack(p, i) => return self.play_track(p, i),
            Message::Resolved(key, result) => return self.on_resolved(key, result),
            Message::OpenChooser(at) => self.choosing = at,
            Message::ChooseVideo(id) => return self.choose_video(&id),
            Message::Skip(dir) => return self.skip(dir),
            Message::ToggleShuffle => return self.toggle_shuffle(),
            Message::CycleRepeat => return self.cycle_repeat(),
            Message::Cover(id, Ok(handle)) => {
                self.covers.insert(id, handle);
            }
            Message::Cover(id, Err(e)) => tracing::debug!(%id, error = %e, "sin portada"),
            Message::OpenFilter => {
                if self.selected.is_some() {
                    self.filter.get_or_insert_with(String::new);
                    return operation::focus(Id::new(FILTER_INPUT));
                }
            }
            Message::FilterChanged(s) => self.filter = Some(s),
            Message::CloseFilter => {
                self.filter = None;
                self.choosing = None;
            }
            Message::WindowHandle(hwnd) => {
                if self.media.is_none() && std::env::var_os("SURCO_NO_MEDIA_KEYS").is_none() {
                    self.media = media::Media::new(hwnd);
                }
            }
        }
        Task::none()
    }

    fn download_ytdlp(&mut self) -> Task<Message> {
        let Some(dir) = data_dir() else {
            self.ytdlp = YtDlpState::Failed("no hay carpeta de datos".into());
            return Task::none();
        };
        let Some(rt) = RUNTIME.get() else {
            return Task::none();
        };
        self.ytdlp = YtDlpState::Downloading;
        let client = self.http.clone();
        let job = rt.spawn(async move {
            YtDlp::download(&client, &dir)
                .await
                .map(|y| y.path().to_path_buf())
                .map_err(|e| e.to_string())
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            Message::YtDlpDownloaded,
        )
    }

    fn imported(&mut self, result: ImportResult) {
        self.importing = false;
        match result {
            Ok(imp) => {
                self.notice = (!imp.warnings.is_empty()).then(|| Notice {
                    error: false,
                    text: imp.warnings.join(" "),
                });
                self.import_input.clear();
                // Reimportar la misma playlist la reemplaza.
                let idx = match self
                    .playlists
                    .iter()
                    .position(|(p, _)| p.id == imp.playlist.id)
                {
                    Some(i) => {
                        self.playlists[i] = (imp.playlist, imp.source);
                        if self.active.as_ref().is_some_and(|a| a.playlist == i) {
                            self.active = None;
                        }
                        i
                    }
                    None => {
                        self.playlists.push((imp.playlist, imp.source));
                        self.playlists.len() - 1
                    }
                };
                self.selected = Some(idx);
                self.filter = None;
            }
            Err(e) => {
                self.notice = Some(Notice {
                    error: true,
                    text: e,
                })
            }
        }
    }

    /// Importa desde un link de Spotify o, si es una ruta a un `.csv`, desde el archivo.
    fn start_import(&mut self) -> Task<Message> {
        let input = self.import_input.trim().to_owned();
        if input.is_empty() || self.importing {
            return Task::none();
        }
        let Some(rt) = RUNTIME.get() else {
            return Task::none();
        };
        let csv = csv_path(&input);
        if csv.is_none() && surco_spotify::parse_playlist_id(&input).is_none() {
            self.notice = Some(Notice {
                error: true,
                text: "Eso no parece un link de playlist de Spotify ni un archivo .csv.".into(),
            });
            return Task::none();
        }
        self.importing = true;
        self.notice = None;
        let creds = self.spotify_creds.clone();
        let client = self.http.clone();
        let job = rt.spawn(async move {
            let result = match csv {
                Some(path) => {
                    tokio::task::spawn_blocking(move || surco_spotify::import_csv_file(&path))
                        .await
                        .map_err(|e| e.to_string())?
                }
                None => Importer::standard(client, creds).import_link(&input).await,
            };
            result.map_err(|e| e.to_string())
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            Message::Imported,
        )
    }

    /// Le cuenta al sistema (MPRIS/SMTC) qué suena.
    fn sync_media(&mut self) {
        let Some(media) = &mut self.media else {
            return;
        };
        let now = self.active.as_ref().and_then(|a| {
            let at = (a.playlist, a.queue.current()?);
            let t = self.playlists.get(at.0)?.0.tracks.get(at.1)?;
            let video = match self.resolutions.get(&surco_resolver::cache_key(t)) {
                Some(TrackRes::Done(m)) => Some(m.youtube_id.clone()),
                _ => None,
            };
            Some(media::NowPlaying {
                title: t.title.clone(),
                artist: t.artists.join(", "),
                album: t.album.clone(),
                duration: Some(Duration::from_millis(t.duration_ms)),
                cover_url: video.and_then(|v| {
                    covers::large_path(&v).map(|p| format!("file://{}", p.display()))
                }),
            })
        });
        let playing = matches!(self.status.state, PlayerState::Playing);
        media.update(now.as_ref(), playing, self.status.position);
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![keyboard::listen().filter_map(on_key)];
        if let Some(p) = &self.player {
            subs.push(Subscription::run_with(
                StatusFeed(p.subscribe()),
                status_stream,
            ));
        }
        if self.media.is_some() {
            subs.push(Subscription::run(media::events));
        }
        Subscription::batch(subs)
    }

    fn theme(&self) -> Theme {
        theme::dark()
    }
}

/// Envoltorio del canal de estado del motor para usarlo como suscripción.
struct StatusFeed(tokio::sync::watch::Receiver<Status>);

impl std::hash::Hash for StatusFeed {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hay un solo motor: una identidad fija alcanza.
        "surco-status-feed".hash(state);
    }
}

/// Emite un mensaje cada vez que el motor publica un estado nuevo.
fn status_stream(feed: &StatusFeed) -> impl iced::futures::Stream<Item = Message> + use<> {
    iced::futures::stream::unfold(feed.0.clone(), |mut rx| async move {
        rx.changed().await.ok()?;
        let status = rx.borrow_and_update().clone();
        Some((Message::Status(status), rx))
    })
}

/// Atajos globales (solo si ningún widget usó la tecla, p. ej. el campo de texto).
/// "Ctrl" es Cmd en macOS (`Modifiers::command`).
fn on_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if modifiers.command() && !modifiers.alt() {
        return match key.as_ref() {
            Key::Character("l") | Key::Character("L") => Some(Message::FocusImport),
            Key::Character("f") | Key::Character("F") => Some(Message::OpenFilter),
            Key::Named(key::Named::ArrowLeft) => Some(Message::Skip(-1)),
            Key::Named(key::Named::ArrowRight) => Some(Message::Skip(1)),
            _ => None,
        };
    }
    if modifiers.command() || modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    match key {
        Key::Named(key::Named::Space) => Some(Message::TogglePause),
        Key::Named(key::Named::ArrowLeft) => Some(Message::SeekRelative(-SEEK_STEP)),
        Key::Named(key::Named::ArrowRight) => Some(Message::SeekRelative(SEEK_STEP)),
        Key::Named(key::Named::Escape) => Some(Message::CloseFilter),
        Key::Named(key::Named::MediaPlayPause) => Some(Message::TogglePause),
        Key::Named(key::Named::MediaTrackNext) => Some(Message::Skip(1)),
        Key::Named(key::Named::MediaTrackPrevious) => Some(Message::Skip(-1)),
        _ => None,
    }
}

/// Si el texto es la ruta a un `.csv` existente, la devuelve.
fn csv_path(input: &str) -> Option<PathBuf> {
    let trimmed = input.trim_matches(|c| c == '"' || c == '\'');
    let path = PathBuf::from(trimmed.strip_prefix("file://").unwrap_or(trimmed));
    let is_csv = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("csv"));
    (is_csv && path.is_file()).then_some(path)
}
