//! Binario de Surco: GUI con iced.

// En Windows, release sin ventana de consola.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod theme;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use iced::keyboard::{self, key, Key};
use iced::widget::{button, column, container, row, slider, text, text_input, Space};
use iced::{window, Alignment, Element, Length, Size, Subscription, Task, Theme};
use surco_core::{format_duration, PlayerState};
use surco_player::{Command, PlayerHandle, Source, Status};
use surco_provider::{HttpClient, Provider, YtDlp};

/// Video de prueba de la Fase 2: "Big Buck Bunny" (Blender Foundation, CC BY 3.0).
const TEST_URL: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
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
}

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
}

impl Surco {
    fn new() -> (Self, Task<Message>) {
        let ytdlp = YtDlp::locate(data_dir().as_deref());
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
            Message::Status(status) => self.status = status,
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
                self.send(Command::SetProvider(Arc::new(Provider::new(YtDlp::new(
                    &path,
                )))));
                self.ytdlp = YtDlpState::Found(path);
            }
            Message::YtDlpDownloaded(Err(e)) => self.ytdlp = YtDlpState::Failed(e),
        }
        Task::none()
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
        let sidebar = container(
            column![
                text("Surco").size(theme::SIZE_L),
                Space::new().height(Length::Fill),
                container(
                    text("+ Importar")
                        .size(theme::SIZE_S)
                        .style(theme::accent_text)
                )
                .padding([6, 10])
                .style(theme::outlined),
            ]
            .spacing(12),
        )
        .padding(16)
        .width(200)
        .height(Length::Fill)
        .style(theme::surface);

        let main = container(self.view_test_panel())
            .padding(32)
            .width(Length::Fill)
            .height(Length::Fill);

        column![
            row![sidebar, main].height(Length::Fill),
            self.view_player_bar()
        ]
        .into()
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
            text("Prueba de audio").size(theme::SIZE_L),
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

        let title = match (&s.state, &s.title) {
            (PlayerState::Loading, _) => "Cargando…".to_owned(),
            (PlayerState::Idle, _) => "Nada sonando".to_owned(),
            (_, Some(t)) => t.clone(),
            (_, None) => "Sin título".to_owned(),
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
