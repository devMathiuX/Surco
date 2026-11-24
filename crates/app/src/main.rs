//! Binario de Surco: GUI con iced.

// En Windows, release sin ventana de consola.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod theme;

use iced::widget::{column, container, row, text, Space};
use iced::{window, Element, Length, Size, Task, Theme};

fn main() -> anyhow::Result<()> {
    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "iniciando Surco");

    iced::application(Surco::new, Surco::update, Surco::view)
        .title("Surco")
        .theme(Surco::theme)
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
    // Fase 1: logs a stderr. El archivo rotativo llega con la persistencia (Fase 6).
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("surco=info,warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

#[derive(Debug, Default)]
struct Surco;

#[derive(Debug, Clone)]
enum Message {}

impl Surco {
    fn new() -> (Self, Task<Message>) {
        (Self, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {}
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

        let empty_state = container(
            text("Pegá un link de playlist de Spotify para empezar")
                .size(theme::SIZE_M)
                .style(theme::secondary_text),
        )
        .center(Length::Fill);

        let player_bar = container(
            text("Nada sonando")
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
        )
        .padding(16)
        .width(Length::Fill)
        .height(72)
        .center_y(72)
        .style(theme::surface);

        column![row![sidebar, empty_state].height(Length::Fill), player_bar].into()
    }
}
