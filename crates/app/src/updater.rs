//! Aviso de versión nueva al abrir, e instalación desde la app.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use iced::widget::{button, column, container, row, text};
use iced::{Element, Length, Task};
use surco_provider::update::{self, Installed, Release};

use crate::{theme, Message, Surco, RUNTIME};

/// Espera antes de consultar GitHub, para no competir con el arranque.
const CHECK_DELAY: Duration = Duration::from_secs(3);

/// Versión nueva a abrir cuando Surco termine de cerrarse.
pub static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, Default)]
pub enum Update {
    #[default]
    None,
    Available(Release),
    Installing(String),
    /// macOS: se abrió el `.dmg`.
    Opened,
    /// Linux sin pkexec: se abrió una terminal con el comando.
    Terminal(String),
    /// Linux sin pkexec ni terminal: hay que correr el comando a mano.
    Manual(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum UpdateMsg {
    Checked(Result<Option<Release>, String>),
    Install,
    Dismiss,
    Installed(Result<Installed, String>),
    OpenPage,
    /// Copiar el comando de instalación (Linux sin pkexec).
    CopyCommand,
    /// "Buscar actualizaciones" en Acerca de.
    CheckNow,
}

/// Estado de la búsqueda pedida a mano desde Acerca de.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualCheck {
    Idle,
    Checking,
    UpToDate,
    Found,
    Failed,
}

/// Consulta, al rato de abrir, si hay una versión nueva.
pub fn check() -> Task<Message> {
    update::cleanup();
    let Some(rt) = RUNTIME.get() else {
        return Task::none();
    };
    let job = rt.spawn(async {
        tokio::time::sleep(CHECK_DELAY).await;
        update::check(&update::client(), env!("CARGO_PKG_VERSION"))
            .await
            .map_err(|e| e.to_string())
    });
    Task::perform(job, |r| {
        Message::Update(UpdateMsg::Checked(r.unwrap_or_else(|e| Err(e.to_string()))))
    })
}

impl Surco {
    pub(crate) fn on_update(&mut self, msg: UpdateMsg) -> Task<Message> {
        match msg {
            UpdateMsg::CheckNow => {
                self.update_check = ManualCheck::Checking;
                return check();
            }
            UpdateMsg::Checked(Ok(Some(r))) => {
                tracing::info!(version = %r.version, "hay una versión nueva");
                self.update = Update::Available(r);
                if self.update_check == ManualCheck::Checking {
                    self.update_check = ManualCheck::Found;
                }
            }
            UpdateMsg::Checked(Ok(None)) => {
                tracing::debug!("Surco está al día");
                if self.update_check == ManualCheck::Checking {
                    self.update_check = ManualCheck::UpToDate;
                }
            }
            // Sin internet o GitHub no responde: no molestar.
            UpdateMsg::Checked(Err(e)) => {
                tracing::info!(error = %e, "no se pudo buscar actualizaciones");
                if self.update_check == ManualCheck::Checking {
                    self.update_check = ManualCheck::Failed;
                }
            }
            UpdateMsg::Dismiss => self.update = Update::None,
            UpdateMsg::OpenPage => {
                let page = match &self.update {
                    Update::Available(r) => r.page.clone(),
                    _ => update::RELEASES_PAGE.to_owned(),
                };
                open_in_browser(&page);
            }
            UpdateMsg::Install => {
                let Update::Available(r) = &self.update else {
                    return Task::none();
                };
                let Some(kind) = update::detect() else {
                    open_in_browser(&r.page);
                    return Task::none();
                };
                let Some(rt) = RUNTIME.get() else {
                    return Task::none();
                };
                let release = r.clone();
                self.update = Update::Installing(r.version.clone());
                let job = rt.spawn(async move {
                    update::install(&update::client(), &release, &kind)
                        .await
                        .map_err(|e| e.to_string())
                });
                return Task::perform(job, |r| {
                    Message::Update(UpdateMsg::Installed(
                        r.unwrap_or_else(|e| Err(e.to_string())),
                    ))
                });
            }
            UpdateMsg::Installed(Ok(Installed::Restart { relaunch })) => {
                tracing::info!("actualización instalada; reiniciando");
                if let Ok(mut r) = RELAUNCH.lock() {
                    *r = relaunch;
                }
                return iced::exit();
            }
            UpdateMsg::Installed(Ok(Installed::Opened)) => self.update = Update::Opened,
            UpdateMsg::Installed(Ok(Installed::Terminal { command })) => {
                self.update = Update::Terminal(command)
            }
            UpdateMsg::Installed(Ok(Installed::Manual { command })) => {
                self.update = Update::Manual(command)
            }
            UpdateMsg::CopyCommand => {
                if let Update::Terminal(c) | Update::Manual(c) = &self.update {
                    return iced::clipboard::write(c.clone());
                }
            }
            UpdateMsg::Installed(Err(e)) => {
                tracing::warn!(error = %e, "no se pudo actualizar");
                self.update = Update::Failed(e);
            }
        }
        Task::none()
    }

    /// Tarjeta de la barra lateral con la versión nueva.
    pub(crate) fn view_update(&self) -> Option<Element<'_, Message>> {
        let small = |s: String| text(s).size(theme::SIZE_S).style(theme::secondary_text);
        let action = |label: &'static str, msg: UpdateMsg, primary: bool| {
            button(text(label).size(theme::SIZE_S))
                .on_press(Message::Update(msg))
                .padding([4, 10])
                .style(if primary {
                    theme::primary_button
                } else {
                    theme::ghost_button
                })
        };
        let body: Element<'_, Message> = match &self.update {
            Update::None => return None,
            Update::Available(r) => {
                let can_install = update::detect().is_some();
                let install = if can_install {
                    action(tr!("Instalar", "Install"), UpdateMsg::Install, true)
                } else {
                    action(tr!("Descargar", "Download"), UpdateMsg::OpenPage, true)
                };
                column![
                    text(crate::trf!(
                        "Surco {} está disponible",
                        "Surco {} is available",
                        r.version
                    ))
                    .size(theme::SIZE_S),
                    row![
                        install,
                        action(tr!("Ahora no", "Not now"), UpdateMsg::Dismiss, false)
                    ]
                    .spacing(6),
                ]
                .spacing(8)
                .into()
            }
            Update::Installing(v) => small(crate::trf!(
                "Instalando Surco {}… se va a reiniciar solo.",
                "Installing Surco {}… it will restart by itself.",
                v
            ))
            .into(),
            Update::Opened => column![
                small(
                    tr!(
                        "Se abrió el instalador: arrastrá Surco a Aplicaciones.",
                        "The installer opened: drag Surco to Applications."
                    )
                    .into()
                ),
                action(tr!("Listo", "Done"), UpdateMsg::Dismiss, false),
            ]
            .spacing(8)
            .into(),
            Update::Terminal(_) => column![
                small(
                    tr!(
                        "Se abrió una terminal: escribí tu contraseña ahí. Cuando termine, cerrá y abrí Surco.",
                        "A terminal opened: type your password there. When it's done, close and reopen Surco."
                    )
                    .into()
                ),
                row![
                    action(tr!("Copiar comando", "Copy command"), UpdateMsg::CopyCommand, false),
                    action(tr!("Listo", "Done"), UpdateMsg::Dismiss, false),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
            Update::Manual(command) => column![
                small(
                    tr!(
                        "Para instalarla, corré esto en una terminal:",
                        "To install it, run this in a terminal:"
                    )
                    .into()
                ),
                container(
                    text(command.as_str())
                        .size(theme::SIZE_S - 1)
                        .font(iced::Font::MONOSPACE)
                )
                .padding(6)
                .width(Length::Fill)
                .style(theme::cover_placeholder),
                row![
                    action(tr!("Copiar", "Copy"), UpdateMsg::CopyCommand, true),
                    action(tr!("Cerrar", "Close"), UpdateMsg::Dismiss, false),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
            Update::Failed(e) => column![
                text(crate::trf!(
                    "No se pudo actualizar: {}",
                    "Couldn't update: {}",
                    e
                ))
                .size(theme::SIZE_S)
                .style(theme::error_text),
                row![
                    action(tr!("Descargar", "Download"), UpdateMsg::OpenPage, true),
                    action(tr!("Cerrar", "Close"), UpdateMsg::Dismiss, false),
                ]
                .spacing(6),
            ]
            .spacing(8)
            .into(),
        };
        Some(
            container(body)
                .padding(10)
                .width(Length::Fill)
                .style(theme::card)
                .into(),
        )
    }
}

/// Abre `url` en el navegador del sistema.
pub fn open_in_browser(url: &str) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    if let Err(e) = cmd.spawn() {
        tracing::warn!(error = %e, "no se pudo abrir el navegador");
    }
}

/// Abre la versión nueva si se instaló una (después de cerrar esta).
pub fn relaunch_if_updated() {
    if let Some(path) = RELAUNCH.lock().ok().and_then(|mut r| r.take()) {
        update::relaunch(&path);
    }
}
