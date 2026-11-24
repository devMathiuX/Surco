//! Binario de Surco: GUI con iced.

// En Windows, release sin ventana de consola.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod config;
mod playback;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use iced::keyboard::{self, key, Key};
use iced::widget::{
    button, column, container, operation, row, scrollable, slider, text, text_input, tooltip,
    Column, Id, Space,
};
use iced::{window, Alignment, Element, Length, Size, Subscription, Task, Theme};
use surco_core::{format_duration, PlayerState, Playlist};
use surco_player::{Command, PlayerHandle, Source, Status};
use surco_provider::{HttpClient, Provider, YtDlp};
use surco_resolver::{Match, Resolver};
use surco_spotify::{Credentials, Importer};

use playback::TrackRes;

/// Video de prueba de la Fase 2: "Big Buck Bunny" (Blender Foundation, CC BY 3.0).
const TEST_URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
const IMPORT_INPUT: &str = "import-input";
const SEEK_STEP: Duration = Duration::from_secs(5);

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
    /// Canción de playlist sonando: (playlist, índice).
    now: Option<(usize, usize)>,
    /// Canción cuyo selector de video está abierto.
    choosing: Option<(usize, usize)>,
    failures_in_a_row: u32,
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
    TogglePause,
    SeekPreview(f32),
    SeekCommit,
    SeekBy(i64),
    Volume(f32),
    Status(Status),
    DownloadYtDlp,
    YtDlpDownloaded(Result<PathBuf, String>),
    ImportInputChanged(String),
    Import,
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
            playlists: Vec::new(),
            selected: None,
            import_input: String::new(),
            importing: false,
            notice: None,
            resolver,
            resolutions: HashMap::new(),
            now: None,
            choosing: None,
            failures_in_a_row: 0,
        };
        (state, Task::none())
    }

    fn send(&self, cmd: Command) {
        if let Some(p) = &self.player {
            p.send(cmd);
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::InputChanged(s) => self.input = s,
            Message::PlayInput => {
                if let Some(source) = Source::parse(&self.input) {
                    self.now = None;
                    self.send(Command::Load(source));
                }
            }
            Message::TogglePause => self.send(Command::TogglePause),
            Message::SeekPreview(v) => self.seeking = Some(v),
            Message::SeekCommit => {
                if let Some(v) = self.seeking.take() {
                    self.send(Command::Seek(Duration::from_secs_f32(v.max(0.0))));
                }
            }
            Message::SeekBy(secs) => {
                let step = SEEK_STEP.as_secs() as i64 * secs.signum();
                let pos = self.status.position.as_secs() as i64 + step;
                self.send(Command::Seek(Duration::from_secs(pos.max(0) as u64)));
            }
            Message::Volume(v) => {
                self.status.volume = v;
                self.send(Command::SetVolume(v));
            }
            Message::Status(status) => return self.on_status(status),
            Message::DownloadYtDlp => {
                let Some(dir) = data_dir() else {
                    self.ytdlp = YtDlpState::Failed("no hay carpeta de datos".into());
                    return Task::none();
                };
                self.ytdlp = YtDlpState::Downloading;
                let Some(rt) = RUNTIME.get() else {
                    return Task::none();
                };
                let job = rt.spawn(async move {
                    YtDlp::download(&HttpClient::new(), &dir)
                        .await
                        .map(|y| y.path().to_path_buf())
                        .map_err(|e| e.to_string())
                });
                return Task::perform(
                    async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
                    Message::YtDlpDownloaded,
                );
            }
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
            Message::Imported(result) => {
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
                                i
                            }
                            None => {
                                self.playlists.push((imp.playlist, imp.source));
                                self.playlists.len() - 1
                            }
                        };
                        self.selected = Some(idx);
                    }
                    Err(e) => {
                        self.notice = Some(Notice {
                            error: true,
                            text: e,
                        })
                    }
                }
            }
            Message::SelectPlaylist(i) => self.selected = i,
            Message::FocusImport => return operation::focus(Id::new(IMPORT_INPUT)),
            Message::PlayTrack(p, i) => {
                self.failures_in_a_row = 0;
                return self.play_track(p, i);
            }
            Message::Resolved(key, result) => return self.on_resolved(key, result),
            Message::OpenChooser(at) => self.choosing = at,
            Message::ChooseVideo(id) => return self.choose_video(&id),
            Message::Skip(dir) => return self.advance(dir),
        }
        Task::none()
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
        let job = rt.spawn(async move {
            let result = match csv {
                Some(path) => {
                    tokio::task::spawn_blocking(move || surco_spotify::import_csv_file(&path))
                        .await
                        .map_err(|e| e.to_string())?
                }
                None => {
                    Importer::standard(HttpClient::new(), creds)
                        .import_link(&input)
                        .await
                }
            };
            result.map_err(|e| e.to_string())
        });
        Task::perform(
            async move { job.await.unwrap_or_else(|e| Err(e.to_string())) },
            Message::Imported,
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::listen().filter_map(on_key);
        match &self.player {
            Some(p) => Subscription::batch([
                keys,
                Subscription::run_with(StatusFeed(p.subscribe()), status_stream),
            ]),
            None => keys,
        }
    }

    fn theme(&self) -> Theme {
        theme::dark()
    }

    fn view(&self) -> Element<'_, Message> {
        let main: Element<'_, Message> = match self.selected.and_then(|i| self.playlists.get(i)) {
            Some((playlist, source)) => {
                self.view_playlist(self.selected.unwrap_or(0), playlist, source)
            }
            None => self.view_empty(),
        };
        let main = container(main)
            .padding([24, 32])
            .width(Length::Fill)
            .height(Length::Fill);

        column![
            row![self.view_sidebar(), main].height(Length::Fill),
            self.view_player_bar()
        ]
        .into()
    }

    fn view_sidebar(&self) -> Element<'_, Message> {
        let mut list = Column::new().spacing(2);
        for (i, (p, _)) in self.playlists.iter().enumerate() {
            list = list.push(
                button(
                    text(&p.name)
                        .size(theme::SIZE_S)
                        .wrapping(text::Wrapping::None),
                )
                .on_press(Message::SelectPlaylist(Some(i)))
                .width(Length::Fill)
                .padding([6, 8])
                .style(theme::list_button(self.selected == Some(i))),
            );
        }

        let input = text_input("Link de Spotify o archivo .csv", &self.import_input)
            .id(Id::new(IMPORT_INPUT))
            .on_input(Message::ImportInputChanged)
            .on_submit(Message::Import)
            .padding(8)
            .size(theme::SIZE_S)
            .style(theme::input);
        let import = button(
            text(if self.importing {
                "Importando…"
            } else {
                "+ Importar"
            })
            .size(theme::SIZE_S),
        )
        .on_press_maybe((!self.importing).then_some(Message::Import))
        .width(Length::Fill)
        .padding([6, 10])
        .style(theme::ghost_button);

        let mut col = column![
            button(text("Surco").size(theme::SIZE_L))
                .on_press(Message::SelectPlaylist(None))
                .padding(0)
                .style(theme::list_button(false)),
            scrollable(list).height(Length::Fill),
            input,
            import,
        ]
        .spacing(12);
        if let Some(n) = &self.notice {
            col = col.push(text(&n.text).size(theme::SIZE_S).style(if n.error {
                theme::error_text
            } else {
                theme::secondary_text
            }));
        }
        container(col)
            .padding(16)
            .width(220)
            .height(Length::Fill)
            .style(theme::surface)
            .into()
    }

    fn view_empty(&self) -> Element<'_, Message> {
        column![
            text("Pegá un link de playlist de Spotify para empezar").size(theme::SIZE_L),
            text("Usá el campo de la izquierda (Ctrl+L). También acepta un CSV exportado con Exportify. Clic en una canción para escucharla; Ctrl+←/→ pasa a la anterior/siguiente.")
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            Space::new().height(24),
            self.view_test_panel(),
        ]
        .spacing(8)
        .into()
    }

    fn view_playlist<'a>(
        &'a self,
        index: usize,
        p: &'a Playlist,
        source: &str,
    ) -> Element<'a, Message> {
        let total_ms: u64 = p.tracks.iter().map(|t| t.duration_ms).sum();
        let header = column![
            text(&p.name).size(theme::SIZE_L),
            text(format!(
                "{} canciones · {} · importada desde {source}",
                p.tracks.len(),
                format_duration(total_ms)
            ))
            .size(theme::SIZE_S)
            .style(theme::secondary_text),
        ]
        .spacing(4);

        let rows = Column::with_children(p.tracks.iter().enumerate().map(|(i, t)| {
            let playing = self.now == Some((index, i));
            let badge: Element<'_, Message> = match self.resolution(t) {
                Some(TrackRes::Resolving) => text("buscando…")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into(),
                Some(TrackRes::Done(m)) if m.is_doubtful() => {
                    button(text("dudosa").size(theme::SIZE_S))
                        .on_press(Message::OpenChooser(Some((index, i))))
                        .padding([2, 8])
                        .style(theme::ghost_button)
                        .into()
                }
                Some(TrackRes::Failed(e)) => tooltip(
                    text("sin match")
                        .size(theme::SIZE_S)
                        .style(theme::error_text),
                    container(text(e).size(theme::SIZE_S))
                        .padding(8)
                        .max_width(360)
                        .style(theme::surface),
                    tooltip::Position::Left,
                )
                .into(),
                _ => Space::new().into(),
            };
            button(
                row![
                    text(i + 1)
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text)
                        .width(40),
                    column![
                        text(&t.title)
                            .size(theme::SIZE_M)
                            .wrapping(text::Wrapping::None),
                        text(t.artists.join(", "))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(2)
                    .width(Length::Fill),
                    badge,
                    text(format_duration(t.duration_ms))
                        .size(theme::SIZE_S)
                        .style(theme::secondary_text),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .on_press(Message::PlayTrack(index, i))
            .padding([6, 8])
            .width(Length::Fill)
            .style(theme::list_button(playing))
            .into()
        }))
        .padding(iced::Padding::ZERO.right(16));

        let mut col = column![header].spacing(20);
        if let Some(chooser) = self.view_chooser(index) {
            col = col.push(chooser);
        }
        col.push(scrollable(rows).height(Length::Fill)).into()
    }

    /// Selector de video para una canción (los candidatos con su puntaje).
    fn view_chooser(&self, playlist: usize) -> Option<Element<'_, Message>> {
        let at = self.choosing.filter(|(p, _)| *p == playlist)?;
        let track = self.track(at)?;
        let Some(TrackRes::Done(m)) = self.resolution(track) else {
            return None;
        };
        let mut list = Column::new().spacing(2);
        for c in &m.candidates {
            let chosen = c.candidate.id == m.youtube_id;
            let dur = c
                .candidate
                .duration_secs
                .map(|d| format_duration((d * 1000.0) as u64))
                .unwrap_or_else(|| "--:--".into());
            list = list.push(
                button(
                    row![
                        column![
                            text(&c.candidate.title)
                                .size(theme::SIZE_S)
                                .wrapping(text::Wrapping::None),
                            text(
                                c.candidate
                                    .channel
                                    .as_deref()
                                    .unwrap_or("canal desconocido")
                            )
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text),
                        ]
                        .width(Length::Fill),
                        text(dur).size(theme::SIZE_S).style(theme::secondary_text),
                        text(format!("{:>3}", c.score))
                            .size(theme::SIZE_S)
                            .style(theme::secondary_text)
                            .width(32),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .on_press(Message::ChooseVideo(c.candidate.id.clone()))
                .padding([4, 8])
                .width(Length::Fill)
                .style(theme::list_button(chosen)),
            );
        }
        let header = row![
            text(format!(
                "Elegí el video para «{}» ({} canciones de YouTube; el número es el puntaje)",
                track.title,
                m.candidates.len()
            ))
            .size(theme::SIZE_S)
            .width(Length::Fill),
            button(text("Cerrar").size(theme::SIZE_S))
                .on_press(Message::OpenChooser(None))
                .padding([2, 8])
                .style(theme::ghost_button),
        ]
        .align_y(Alignment::Center);
        Some(
            container(column![header, scrollable(list).height(Length::Shrink)].spacing(8))
                .padding(12)
                .max_height(260)
                .style(theme::surface)
                .into(),
        )
    }

    /// Panel de prueba de la Fase 2: pegar una URL y reproducir.
    fn view_test_panel(&self) -> Element<'_, Message> {
        let input = text_input(
            "URL de YouTube, ID de video, URL directa o archivo local",
            &self.input,
        )
        .on_input(Message::InputChanged)
        .on_submit(Message::PlayInput)
        .padding(10)
        .size(theme::SIZE_M)
        .style(theme::input);

        let play = button(text("Reproducir").size(theme::SIZE_M))
            .on_press(Message::PlayInput)
            .padding([10, 16])
            .style(theme::primary_button);

        let ytdlp: Element<'_, Message> = match &self.ytdlp {
            YtDlpState::Found(p) => text(format!("yt-dlp: {}", p.display()))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into(),
            YtDlpState::Missing => row![
                text("No encontré yt-dlp. Hace falta para reproducir desde YouTube.")
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
                button(text("Descargar yt-dlp").size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .style(theme::ghost_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            YtDlpState::Downloading => text("Descargando yt-dlp…")
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .into(),
            YtDlpState::Failed(e) => row![
                text(format!("No se pudo descargar yt-dlp: {e}"))
                    .size(theme::SIZE_S)
                    .style(theme::error_text),
                button(text("Reintentar").size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .style(theme::ghost_button),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        };

        let mut col = column![
            text("Prueba de audio").size(theme::SIZE_M),
            text("Pegá un link de YouTube y dale a Reproducir. Espacio pausa, ←/→ mueven 5 s.")
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            row![input, play].spacing(8).align_y(Alignment::Center),
            ytdlp,
        ]
        .spacing(16)
        .max_width(720);

        if let Some(e) = &self.fatal {
            col = col.push(text(e).size(theme::SIZE_S).style(theme::error_text));
        }
        if let PlayerState::Error(e) = &self.status.state {
            col = col.push(text(e).size(theme::SIZE_S).style(theme::error_text));
        }
        col.into()
    }

    fn view_player_bar(&self) -> Element<'_, Message> {
        let s = &self.status;
        let playing = matches!(s.state, PlayerState::Playing);
        let toggle =
            button(text(if playing { "Pausar" } else { "Reproducir" }).size(theme::SIZE_S))
                .on_press(Message::TogglePause)
                .padding([8, 14])
                .width(110)
                .style(theme::primary_button);

        let now = self.now.and_then(|n| self.track(n));
        let resolving =
            now.is_some_and(|t| matches!(self.resolution(t), Some(TrackRes::Resolving)));
        let title = match (now, &s.state, &s.title) {
            (Some(t), _, _) if resolving => format!("{} · buscando en YouTube…", t.title),
            (Some(t), PlayerState::Loading, _) => format!("{} · cargando…", t.title),
            (Some(t), _, _) => format!("{} — {}", t.title, t.artists.join(", ")),
            (None, PlayerState::Loading, _) => "Cargando…".to_owned(),
            (None, PlayerState::Idle, _) => "Nada sonando".to_owned(),
            (None, PlayerState::Error(_), _) => "No se pudo reproducir".to_owned(),
            (None, _, Some(t)) => t.clone(),
            (None, _, None) => "Sin título".to_owned(),
        };
        // Cambiar el video de la canción actual (por si el match no es el correcto).
        let change: Element<'_, Message> = match (self.now, now.and_then(|t| self.resolution(t))) {
            (Some(at), Some(TrackRes::Done(_))) => {
                button(text("Cambiar video").size(theme::SIZE_S))
                    .on_press(Message::OpenChooser(Some(at)))
                    .padding([4, 10])
                    .style(theme::ghost_button)
                    .into()
            }
            _ => Space::new().into(),
        };

        let shown = self.seeking.unwrap_or(s.position.as_secs_f32());
        // Sin duración conocida (p. ej. una URL directa a un MP4 fragmentado)
        // no hay barra de progreso; ←/→ siguen funcionando.
        let seek: Element<'_, Message> = match s.duration {
            Some(d) => slider(0.0..=d.as_secs_f32().max(1.0), shown, Message::SeekPreview)
                .on_release(Message::SeekCommit)
                .step(0.5_f32)
                .style(theme::seek_slider)
                .into(),
            None => Space::new().width(Length::Fill).into(),
        };
        let time = text(format!(
            "{} / {}",
            format_duration((shown * 1000.0) as u64),
            s.duration
                .map(|d| format_duration(d.as_millis() as u64))
                .unwrap_or_else(|| "--:--".into())
        ))
        .size(theme::SIZE_S)
        .style(theme::secondary_text);

        let volume = slider(0.0..=1.0, s.volume, Message::Volume)
            .step(0.01_f32)
            .width(110)
            .style(theme::seek_slider);

        container(
            row![
                toggle,
                column![
                    text(title).size(theme::SIZE_S),
                    row![seek, time].spacing(12).align_y(Alignment::Center),
                ]
                .spacing(6)
                .width(Length::Fill),
                change,
                text("Vol").size(theme::SIZE_S).style(theme::secondary_text),
                volume,
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding([12, 16])
        .width(Length::Fill)
        .style(theme::surface)
        .into()
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
fn on_key(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if modifiers.control() && !modifiers.alt() {
        return match key.as_ref() {
            Key::Character("l") | Key::Character("L") => Some(Message::FocusImport),
            Key::Named(key::Named::ArrowLeft) => Some(Message::Skip(-1)),
            Key::Named(key::Named::ArrowRight) => Some(Message::Skip(1)),
            _ => None,
        };
    }
    if modifiers.control() || modifiers.alt() {
        return None;
    }
    match key {
        Key::Named(key::Named::Space) => Some(Message::TogglePause),
        Key::Named(key::Named::ArrowLeft) => Some(Message::SeekBy(-1)),
        Key::Named(key::Named::ArrowRight) => Some(Message::SeekBy(1)),
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
