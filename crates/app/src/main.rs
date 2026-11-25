//! Binario de Surco: GUI con iced.

// En Windows, release sin ventana de consola.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod bar;
mod config;
mod covers;
mod home;
mod icons;
mod library;
mod media;
mod offline;
mod playback;
mod playlists;
mod pointer;
mod precache;
mod queue_view;
mod radio;
mod search;
mod settings;
mod skeleton;
mod spotify_keys;
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
use surco_provider::{AudioCache, HttpClient, Provider, YtDlp, AUDIO_CACHE_LIMIT};
use surco_resolver::{Match, Resolver};
use surco_spotify::{Credentials, Importer};
use surco_storage::Store;

use playback::{Active, CoverJobs, TrackRes};
use precache::PrecacheState;
use settings::Settings;

const IMPORT_INPUT: &str = "import-input";
const FILTER_INPUT: &str = "filter-input";
const SEEK_STEP: f32 = 5.0;

/// Runtime de tokio para la red y los procesos hijos (yt-dlp).
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn main() -> anyhow::Result<()> {
    // El guard vacía el log en disco al salir.
    let _log_guard = init_logging();
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
        .style(Surco::style)
        .subscription(Surco::subscription)
        .window(window::Settings {
            size: Size::new(1000.0, 640.0),
            min_size: Some(Size::new(800.0, 500.0)),
            icon: window_icon(),
            ..window::Settings::default()
        })
        .run()?;

    Ok(())
}

/// Logs a stderr y a un archivo diario en `<datos>/logs/` (se guardan 7 días).
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("surco=info,warn"));
    let (file_layer, guard) = match log_dir().and_then(|dir| {
        // Si la carpeta no existe, tracing-appender se queja al limpiar logs viejos.
        std::fs::create_dir_all(&dir).ok()?;
        tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("surco")
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir)
            .ok()
    }) {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            (
                Some(fmt::layer().with_writer(writer).with_ansi(false)),
                Some(guard),
            )
        }
        None => (None, None),
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .try_init();
    guard
}

fn log_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Surco").map(|d| d.data_local_dir().join("logs"))
}

const ICON_PNG: &[u8] = include_bytes!("../../../packaging/icons/surco-64.png");

/// Ícono de la ventana (Linux y Windows; en macOS lo pone el `.app`).
fn window_icon() -> Option<window::Icon> {
    let img = ::image::load_from_memory_with_format(ICON_PNG, ::image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    window::icon::from_rgba(img.into_raw(), w, h).ok()
}

/// Logo para la barra lateral.
fn logo() -> Option<image::Handle> {
    let img = ::image::load_from_memory_with_format(ICON_PNG, ::image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Some(image::Handle::from_rgba(w, h, img.into_raw()))
}

/// Carpeta del audio precargado.
fn audio_cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Surco").map(|d| d.cache_dir().join("audio"))
}

/// Provider (yt-dlp + caché de URLs) con la caché de audio en disco.
fn make_provider(ytdlp: YtDlp) -> Arc<Provider> {
    let p = Provider::new(ytdlp);
    Arc::new(match audio_cache_dir() {
        Some(dir) => p.with_audio_cache(AudioCache::new(dir, AUDIO_CACHE_LIMIT)),
        None => p,
    })
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
    ytdlp: YtDlpState,
    /// Valor del slider mientras se arrastra (segundos).
    seeking: Option<f32>,
    fatal: Option<String>,
    spotify_creds: Option<Credentials>,
    spotify_source: spotify_keys::KeySource,
    keys_form: spotify_keys::KeysForm,
    http: HttpClient,
    /// Playlists importadas en esta sesión (se guardan en disco en la Fase 6),
    /// con la estrategia que funcionó.
    playlists: Vec<(Playlist, String)>,
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
    /// Desplazamiento y alto visible de la lista de la playlist (para dibujar
    /// solo las filas que se ven).
    list_view: (f32, f32),
    /// MPRIS / SMTC.
    media: Option<media::Media>,
    /// Biblioteca, matches, historial y ajustes en SQLite. `None` si no se
    /// pudo abrir (la app funciona igual, sin guardar nada).
    store: Option<Store>,
    /// Última fuente anotada en el historial.
    recorded: Option<Source>,
    /// Ajustes de la pantalla de ajustes.
    settings: Settings,
    /// Lo que se escribe en los campos de color del tema personalizado.
    custom_hex: [String; 2],
    /// Qué muestra el panel principal.
    screen: Screen,
    theme: Theme,
    /// yt-dlp + caché de audio (para la precarga y la radio).
    provider: Option<Arc<Provider>>,
    precache: PrecacheState,
    /// Hay que revisar qué precargar (cambió la cola, una resolución o un ajuste).
    precache_dirty: bool,
    /// IDs de las playlists descargadas (ver `offline`).
    offline: std::collections::HashSet<String>,
    radio_loading: bool,
    cover_jobs: CoverJobs,
    /// Pidiendo parecidas para la cola sin fin.
    extending: bool,
    /// Semilla con la que no se consiguieron parecidas (no reintentar en bucle).
    extend_failed: Option<String>,
    /// Arrastre (en la cola o en la barra lateral).
    drag: Option<Drag>,
    /// Playlist de la barra lateral bajo el mouse.
    sidebar_hover: Option<usize>,
    search: search::SearchState,
    home: home::HomeState,
    /// Menú abierto de una canción (agregar a la cola o a una playlist).
    song_menu: Option<playlists::SongMenu>,
    /// Playlist de la barra lateral con el menú de clic derecho abierto.
    playlist_menu: Option<usize>,
    /// Menú de la flechita de una playlist descargada (borrar descargas).
    download_menu: Option<usize>,
    /// Último clic (y tamaño de la ventana), para abrir menús al lado.
    pointer: (iced::Point, iced::Size),
    /// Dónde se abrió el menú que está abierto.
    menu_at: (iced::Point, iced::Size),
    /// Cambia con cada cambio de pantalla (ver `Surco::style`).
    repaint: u8,
    /// Mutear y desplegable del volumen.
    volume_ui: bar::VolumeUi,
    /// Playlist que se está renombrando y el nombre escrito.
    renaming: Option<(usize, String)>,
    /// Recomendadas por playlist (ID).
    recs: HashMap<String, playlists::Recs>,
    /// Descargado por playlist (ID).
    downloads: playlists::Downloads,
    /// Búsqueda de fondo de las canciones de la playlist abierta (portadas).
    background: playback::Background,
    /// Se pidió una radio con el botón y todavía no llegó.
    radio_waiting: bool,
    /// Latido de los esqueletos de carga.
    pulse: u32,
    logo: Option<image::Handle>,
}

/// Qué lista se está arrastrando.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragList {
    Queue,
    Sidebar,
}

/// Arrastre: lista, posición tomada y posición sobre la que está.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Drag {
    list: DragList,
    from: usize,
    to: usize,
}

/// Panel principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    /// Playlist abierta.
    Library,
    Import,
    Settings,
    Queue,
    Search,
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
    /// La lista de la playlist se desplazó: (desplazamiento, alto visible).
    ListScrolled(f32, f32),
    RemovePlaylist(usize),
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
    /// Mostrar u ocultar la pantalla de ajustes.
    ToggleSettings,
    Setting(settings::Change),
    ClearAudioCache,
    /// Radio de canciones parecidas a la que suena.
    StartRadio,
    RadioLoaded(bool, radio::RadioResult),
    Precached(String, Result<Match, String>),
    /// Llegaron parecidas para la cola sin fin (semilla, videos).
    QueueExtended(String, Result<Vec<surco_provider::SearchResult>, String>),
    /// Mostrar u ocultar la cola.
    ToggleQueue,
    QueueRemove(usize),
    DragStart(DragList, usize),
    DragOver(DragList, usize),
    DragEnd,
    SidebarHover(Option<usize>),
    GoHome,
    /// Escribieron en el buscador del inicio.
    HomeSearch(String),
    PlayRecent(usize),
    OpenImport,
    NewPlaylist,
    StartRename(usize),
    RenameInput(String),
    RenameCommit,
    /// Abrir (o cerrar) el menú de una canción.
    SongMenu(Option<Box<playlists::SongMenu>>),
    SongMenuPick(playlists::AddTarget),
    /// Menú de clic derecho de una playlist de la barra lateral.
    PlaylistMenu(Option<usize>),
    /// Menú de la flechita de una playlist descargada.
    DownloadMenu(Option<usize>),
    /// Clic en (posición, tamaño de la ventana).
    Pointer(iced::Point, iced::Size),
    VolumeUi(bar::VolumeMsg),
    /// Abrir una playlist y empezar a renombrarla.
    RenameFromMenu(usize),
    RecsLoaded(String, Result<Vec<surco_provider::SearchResult>, String>),
    RecsRefresh(usize),
    RecAdd(usize, usize),
    RecPlay(usize, usize),
    /// Dejar de descargar una playlist y borrar su audio.
    DeleteDownloads(usize),
    /// Flechita: descargar la playlist.
    StartOffline(usize),
    /// Paso del latido de los esqueletos de carga.
    Tick,
    Search(search::SearchMsg),
    Keys(spotify_keys::KeysMsg),
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
        let provider = ytdlp.clone().map(make_provider);
        let options = surco_player::Options {
            null_output: std::env::var_os("SURCO_NULL_AUDIO").is_some(),
        };
        let (player, fatal) = match RUNTIME.get() {
            Some(rt) => match surco_player::spawn(provider.clone(), rt.handle().clone(), options) {
                Ok(p) => (Some(p), None),
                Err(e) => (None, Some(e.to_string())),
            },
            None => (None, Some("runtime no inicializado".to_owned())),
        };
        let mut state = Self {
            status: player
                .as_ref()
                .map(PlayerHandle::status)
                .unwrap_or_default(),
            player,
            ytdlp: match ytdlp {
                Some(y) => YtDlpState::Found(y.path().to_path_buf()),
                None => YtDlpState::Missing,
            },
            seeking: None,
            fatal,
            spotify_creds: None,
            spotify_source: spotify_keys::KeySource::None,
            keys_form: spotify_keys::KeysForm::default(),
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
            list_view: (0.0, 0.0),
            media: None,
            store: library::open(),
            recorded: None,
            settings: Settings::default(),
            custom_hex: Default::default(),
            screen: Screen::Home,
            theme: theme::iced_theme(),
            provider,
            precache: PrecacheState::default(),
            precache_dirty: false,
            offline: Default::default(),
            radio_loading: false,
            cover_jobs: CoverJobs::default(),
            extending: false,
            extend_failed: None,
            drag: None,
            sidebar_hover: None,
            search: search::SearchState::default(),
            home: home::HomeState::default(),
            song_menu: None,
            playlist_menu: None,
            download_menu: None,
            pointer: Default::default(),
            menu_at: Default::default(),
            repaint: 0,
            volume_ui: bar::VolumeUi::default(),
            renaming: None,
            recs: HashMap::new(),
            downloads: playlists::Downloads::new(),
            background: playback::Background::default(),
            radio_waiting: false,
            pulse: 0,
            logo: logo(),
        };
        state.restore();
        state.load_settings();
        state.load_spotify_keys();
        state.refresh_downloads();
        let covers = state.queue_list_covers();
        let home = state.refresh_home();
        // Los controles del sistema necesitan la ventana (HWND en Windows).
        let handle = window::oldest()
            .then(|id| match id {
                Some(id) => window::run(id, media::hwnd_of),
                None => Task::done(None),
            })
            .map(Message::WindowHandle);
        (state, Task::batch([handle, covers, home]))
    }

    fn send(&self, cmd: Command) {
        if let Some(p) = &self.player {
            p.send(cmd);
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.page();
        let task = self.handle(message);
        if self.page() != before {
            self.repaint = self.repaint.wrapping_add(1);
        }
        let precache = self.pump_precache();
        let endless = self.extend_endless();
        let background = self.pump_background();
        self.sync_media();
        Task::batch([task, precache, endless, background])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
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
                self.persist_volume();
            }
            Message::Status(status) => return self.on_status(status),
            Message::DownloadYtDlp => return self.download_ytdlp(),
            Message::YtDlpDownloaded(Ok(path)) => {
                self.set_resolver(Some(Arc::new(Resolver::new(Arc::new(YtDlp::new(&path))))));
                self.seed_resolver();
                let provider = make_provider(YtDlp::new(&path));
                self.provider = Some(Arc::clone(&provider));
                self.send(Command::SetProvider(provider));
                self.ytdlp = YtDlpState::Found(path);
                return self.resolve_in_background();
            }
            Message::YtDlpDownloaded(Err(e)) => self.ytdlp = YtDlpState::Failed(e),
            Message::ImportInputChanged(s) => self.import_input = s,
            Message::Import => return self.start_import(),
            Message::PickCsv => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title(tr!("Importar CSV de Exportify", "Import Exportify CSV"))
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
            Message::Imported(result) => return self.imported(result),
            Message::SelectPlaylist(None) => return self.go_home(),
            Message::SelectPlaylist(Some(i)) => {
                if self.renaming.as_ref().is_some_and(|(r, _)| *r != i) {
                    self.renaming = None;
                }
                self.open_playlist(i);
                return self.on_playlist_opened();
            }
            Message::RemovePlaylist(i) => {
                self.playlist_menu = None;
                self.forget_offline(i);
                self.remove_playlist(i);
            }
            Message::FocusImport | Message::OpenImport => {
                self.screen = Screen::Import;
                return operation::focus(Id::new(IMPORT_INPUT));
            }
            Message::PlayTrack(p, i) => return self.play_track(p, i),
            Message::Resolved(key, result) => return self.on_resolved(key, result),
            Message::OpenChooser(at) => self.choosing = at,
            Message::ChooseVideo(id) => return self.choose_video(&id),
            Message::Skip(dir) => return self.skip(dir),
            Message::ToggleShuffle => return self.toggle_shuffle(),
            Message::CycleRepeat => return self.cycle_repeat(),
            Message::Cover(id, result) => return self.on_cover(id, result),
            Message::ToggleSettings => {
                if self.screen == Screen::Settings {
                    return self.go_home();
                }
                self.screen = Screen::Settings;
                self.filter = None;
                self.refresh_downloads();
            }
            Message::Setting(change) => return self.change_setting(change),
            Message::ClearAudioCache => self.clear_audio_cache(),
            Message::StartRadio => {
                let task = self.start_radio(false);
                if self.radio_loading {
                    self.radio_waiting = true;
                    self.screen = Screen::Library;
                }
                return task;
            }
            Message::RadioLoaded(autostart, result) => return self.on_radio(autostart, result),
            Message::Precached(key, result) => return self.on_precached(key, result),
            Message::QueueExtended(seed, result) => return self.on_extended(seed, result),
            Message::ToggleQueue => {
                self.drag = None;
                if self.screen == Screen::Queue {
                    return self.back_from_panel();
                }
                self.screen = Screen::Queue;
            }
            Message::QueueRemove(pos) => self.queue_remove(pos),
            Message::DragStart(list, pos) => {
                self.drag = Some(Drag {
                    list,
                    from: pos,
                    to: pos,
                })
            }
            Message::DragOver(list, pos) => {
                if let Some(d) = self.drag.as_mut().filter(|d| d.list == list) {
                    d.to = pos;
                }
            }
            Message::DragEnd => return self.drag_end(),
            Message::SidebarHover(i) => self.sidebar_hover = i,
            Message::GoHome => return self.go_home(),
            Message::HomeSearch(q) => return self.home_search(q),
            Message::PlayRecent(i) => return self.play_recent(i),
            Message::NewPlaylist => return self.new_playlist(None),
            Message::StartRename(i) => return self.start_rename(i),
            Message::RenameInput(name) => {
                if let Some((_, n)) = &mut self.renaming {
                    *n = name;
                }
            }
            Message::RenameCommit => self.commit_rename(),
            Message::Pointer(at, window) => self.pointer = (at, window),
            Message::SongMenu(menu) => {
                self.menu_at = self.pointer;
                self.song_menu = menu.map(|m| *m);
            }
            Message::DownloadMenu(menu) => {
                self.menu_at = self.pointer;
                self.download_menu = menu;
            }
            Message::VolumeUi(msg) => return self.on_volume_ui(msg),
            Message::PlaylistMenu(menu) => {
                self.menu_at = self.pointer;
                self.playlist_menu = menu;
                if menu.is_some() {
                    self.refresh_downloads();
                }
            }
            Message::RenameFromMenu(i) => {
                self.playlist_menu = None;
                self.open_playlist(i);
                let opened = self.on_playlist_opened();
                return Task::batch([opened, self.start_rename(i)]);
            }
            Message::SongMenuPick(target) => return self.song_menu_pick(target),
            Message::RecsLoaded(id, result) => return self.on_recs(id, result),
            Message::RecsRefresh(p) => return self.load_recs(p, true),
            Message::RecAdd(p, i) => self.add_rec(p, i),
            Message::RecPlay(p, i) => return self.play_rec(p, i),
            Message::DeleteDownloads(p) => {
                self.playlist_menu = None;
                self.download_menu = None;
                self.stop_offline(p);
            }
            Message::StartOffline(p) => self.start_offline(p),
            Message::Tick => self.pulse = self.pulse.wrapping_add(1),
            Message::ListScrolled(offset, height) => {
                let before = view::visible_rows(self.list_view, usize::MAX);
                self.list_view = (offset, height);
                // Buscar las portadas de lo que ahora se ve.
                if view::visible_rows(self.list_view, usize::MAX) != before {
                    return self.resolve_in_background();
                }
            }
            Message::Search(msg) => return self.on_search(msg),
            Message::Keys(msg) => self.on_keys(msg),
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
                self.renaming = None;
                self.song_menu = None;
                self.playlist_menu = None;
                self.download_menu = None;
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

    fn imported(&mut self, result: ImportResult) -> Task<Message> {
        self.importing = false;
        match result {
            Ok(imp) => {
                self.notice = (!imp.warnings.is_empty()).then(|| Notice {
                    error: false,
                    text: imp.warnings.join(" "),
                });
                self.import_input.clear();
                self.persist_playlist(&imp.playlist, imp.source);
                // Reimportar la misma playlist la reemplaza.
                let idx = match self
                    .playlists
                    .iter()
                    .position(|(p, _)| p.id == imp.playlist.id)
                {
                    Some(i) => {
                        self.playlists[i] = (imp.playlist, imp.source.to_owned());
                        if let Some(a) = self.active.as_mut().filter(|a| a.origin == Some(i)) {
                            a.origin = None;
                        }
                        i
                    }
                    None => {
                        self.playlists.push((imp.playlist, imp.source.to_owned()));
                        self.playlists.len() - 1
                    }
                };
                self.open_playlist(idx);
                self.recs.remove(&self.playlists[idx].0.id);
                self.on_playlist_opened()
            }
            Err(e) => {
                self.notice = Some(Notice {
                    error: true,
                    text: e,
                });
                Task::none()
            }
        }
    }

    /// Se abrió una playlist: portadas, búsqueda de fondo y recomendadas.
    fn on_playlist_opened(&mut self) -> Task<Message> {
        self.refresh_downloads();
        let covers = self.queue_list_covers();
        let background = self.resolve_in_background();
        let recs = self.maybe_load_recs();
        // Cada playlist se abre desde arriba.
        let top = operation::snap_to(
            Id::new(view::LIST_SCROLL),
            iced::widget::scrollable::RelativeOffset::START,
        );
        Task::batch([covers, background, recs, top])
    }

    /// Al cerrar la cola o los ajustes: a la playlist abierta o al inicio.
    fn back_from_panel(&mut self) -> Task<Message> {
        match self.selected {
            Some(_) => {
                self.screen = Screen::Library;
                Task::none()
            }
            None => self.go_home(),
        }
    }

    /// Hay algo cargando (para el latido de los esqueletos).
    fn is_loading(&self) -> bool {
        self.importing
            || self.radio_waiting
            || self.search.searching.is_some()
            || self.search.opening
            || (self.screen == Screen::Queue && self.extending)
            || (self.screen == Screen::Library && self.background.is_running())
            || self
                .selected
                .and_then(|i| self.playlists.get(i))
                .is_some_and(|(p, _)| {
                    matches!(self.recs.get(&p.id), Some(playlists::Recs::Loading))
                })
                && self.screen == Screen::Library
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
        if let Some(id) = search::youtube_playlist_id(&input) {
            return self.import_youtube(input, id);
        }
        let csv = csv_path(&input);
        if csv.is_none() && surco_spotify::parse_playlist_id(&input).is_none() {
            self.notice = Some(Notice {
                error: true,
                text: tr!(
                    "Eso no parece un link de playlist de Spotify o YouTube, ni un archivo .csv.",
                    "That doesn't look like a Spotify or YouTube playlist link, or a .csv file."
                )
                .into(),
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
        if self.media.is_none() {
            return;
        }
        let now = self.now_track().map(|t| {
            let video = match self.resolutions.get(&surco_resolver::cache_key(t)) {
                Some(TrackRes::Done(m)) => Some(m.youtube_id.clone()),
                _ => None,
            };
            media::NowPlaying {
                title: t.title.clone(),
                artist: t.artists.join(", "),
                album: t.album.clone(),
                duration: Some(Duration::from_millis(t.duration_ms)),
                cover_url: video.and_then(|v| {
                    covers::large_path(&v).map(|p| format!("file://{}", p.display()))
                }),
            }
        });
        let playing = matches!(self.status.state, PlayerState::Playing);
        let position = self.status.position;
        if let Some(media) = &mut self.media {
            media.update(now.as_ref(), playing, position);
        }
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
        if self.is_loading() {
            subs.push(iced::time::every(Duration::from_millis(120)).map(|_| Message::Tick));
        }
        if self.drag.is_some() {
            // Soltar el botón en cualquier lado termina el arrastre.
            subs.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                    iced::mouse::Button::Left,
                )) => Some(Message::DragEnd),
                _ => None,
            }));
        }
        Subscription::batch(subs)
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// Qué pantalla (y playlist, y si hay un diálogo) se ve: al cambiar, se
    /// redibuja todo.
    fn page(&self) -> (Screen, Option<usize>, bool) {
        let dialog = self.song_menu.is_some()
            || self.playlist_menu.is_some()
            || self.download_menu.is_some();
        (self.screen, self.selected, dialog)
    }

    /// Fondo de la ventana. Cambia de forma invisible (1/255 en el azul) cada
    /// vez que cambia la pantalla: el renderer por CPU de iced 0.14 redibuja
    /// solo lo que cree que cambió y a veces deja texto viejo mezclado con el
    /// nuevo (por ejemplo, de Buscar a Importar). Con otro fondo redibuja la
    /// ventana entera.
    fn style(&self, theme: &Theme) -> iced::theme::Style {
        use iced::theme::Base;
        let mut style = theme.base();
        if self.repaint % 2 == 1 {
            let b = style.background_color.b;
            style.background_color.b = if b > 0.5 {
                b - 1.0 / 255.0
            } else {
                b + 1.0 / 255.0
            };
        }
        style
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
