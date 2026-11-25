//! Herramientas externas (yt-dlp y deno): al abrir, si falta alguna, un
//! diálogo ofrece bajarlas; si ya están (y las bajó Surco), se actualizan
//! solas en segundo plano.

use iced::widget::{button, column, container, row, stack, text, Space};
use iced::{Alignment, Element, Length, Task};

use crate::{theme, DenoState, Message, Surco, YtDlpState, RUNTIME};

/// Estado de una herramienta en el diálogo.
enum Row {
    Ok,
    Missing,
    Downloading,
    Failed(String),
}

impl Surco {
    /// Falta yt-dlp o un runtime de JavaScript.
    pub(crate) fn tools_missing(&self) -> bool {
        matches!(self.ytdlp, YtDlpState::Missing | YtDlpState::Failed(_)) || self.deno_needed()
    }

    fn deno_needed(&self) -> bool {
        match &self.deno {
            DenoState::Missing | DenoState::Failed(_) => true,
            DenoState::Downloading => false,
            // Sin yt-dlp todavía no se revisó: se mira directo.
            DenoState::Ok => surco_provider::js_runtime().is_none(),
        }
    }

    /// Al abrir: el diálogo aparece si falta algo.
    pub(crate) fn maybe_ask_tools(&mut self) {
        self.tools_popup = self.tools_missing();
    }

    /// "Descargar" en el diálogo: baja lo que falte, las dos a la vez.
    pub(crate) fn download_tools(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        if matches!(self.ytdlp, YtDlpState::Missing | YtDlpState::Failed(_)) {
            tasks.push(self.download_ytdlp());
        }
        if self.deno_needed() {
            tasks.push(self.download_deno());
        }
        Task::batch(tasks)
    }

    /// Terminó una descarga: si ya no falta nada, el diálogo se cierra solo.
    pub(crate) fn tools_changed(&mut self) {
        let busy = matches!(self.ytdlp, YtDlpState::Downloading)
            || matches!(self.deno, DenoState::Downloading);
        if self.tools_popup && !busy && !self.tools_missing() {
            self.tools_popup = false;
        }
    }

    /// Al abrir, en segundo plano: actualiza deno si lo bajó Surco y hay
    /// uno nuevo (como mucho una vez por día).
    pub(crate) fn refresh_deno(&self) -> Task<Message> {
        let (Some(dir), Some(rt)) = (crate::data_dir(), RUNTIME.get()) else {
            return Task::none();
        };
        let client = self.http.clone();
        let job = rt.spawn(async move { surco_provider::refresh_deno(&client, &dir).await });
        Task::perform(job, |r| {
            match r {
                Ok(Ok(Some(path))) => {
                    tracing::info!(path = %path.display(), "deno actualizado")
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "no se pudo actualizar deno"),
                Err(e) => tracing::warn!(error = %e, "no se pudo actualizar deno"),
            }
            Message::Noop
        })
    }

    pub(crate) fn view_tools_popup(&self) -> Option<Element<'_, Message>> {
        if !self.tools_popup {
            return None;
        }
        let ytdlp = match &self.ytdlp {
            YtDlpState::Found(_) => Row::Ok,
            YtDlpState::Missing => Row::Missing,
            YtDlpState::Downloading => Row::Downloading,
            YtDlpState::Failed(e) => Row::Failed(e.clone()),
        };
        let deno = match &self.deno {
            DenoState::Downloading => Row::Downloading,
            DenoState::Failed(e) => Row::Failed(e.clone()),
            _ if self.deno_needed() => Row::Missing,
            _ => Row::Ok,
        };
        let busy = matches!(ytdlp, Row::Downloading) || matches!(deno, Row::Downloading);
        let failed = matches!(ytdlp, Row::Failed(_)) || matches!(deno, Row::Failed(_));
        let line = |name: &'static str, what: &'static str, state: Row| {
            let status: Element<'_, Message> = match state {
                Row::Ok => text(tr!("Listo", "Ready"))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into(),
                Row::Missing => text(tr!("Falta", "Missing"))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into(),
                Row::Downloading => text(tr!("Descargando…", "Downloading…"))
                    .size(theme::SIZE_S)
                    .into(),
                Row::Failed(e) => text(trf!("Falló: {e}", "Failed: {e}"))
                    .size(theme::SIZE_S)
                    .style(theme::error_text)
                    .into(),
            };
            row![
                column![
                    text(name).size(theme::SIZE_M),
                    text(what).size(theme::SIZE_S).style(theme::secondary_text),
                ]
                .spacing(2)
                .width(Length::Fill),
                status,
            ]
            .spacing(12)
            .align_y(Alignment::Center)
        };
        let action = if busy {
            button(text(tr!("Descargando…", "Downloading…")).size(theme::SIZE_S))
                .padding([6, 14])
                .style(theme::primary_button)
        } else {
            button(
                text(if failed {
                    tr!("Reintentar", "Retry")
                } else {
                    tr!("Descargar", "Download")
                })
                .size(theme::SIZE_S),
            )
            .on_press(Message::DownloadTools)
            .padding([6, 14])
            .style(theme::primary_button)
        };
        let card = container(
            column![
                text(tr!(
                    "Faltan herramientas para escuchar YouTube",
                    "Tools needed to play YouTube"
                ))
                .size(theme::SIZE_M),
                text(tr!(
                    "Surco las baja de sus páginas oficiales a su carpeta (unos 80 MB en total) y las mantiene actualizadas solo.",
                    "Surco downloads them from their official pages into its own folder (about 80 MB in total) and keeps them up to date."
                ))
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
                line(
                    "yt-dlp",
                    tr!(
                        "Busca en YouTube y consigue el audio.",
                        "Searches YouTube and gets the audio."
                    ),
                    ytdlp
                ),
                line(
                    "deno",
                    tr!(
                        "Evita que YouTube rechace algunas canciones.",
                        "Keeps YouTube from rejecting some songs."
                    ),
                    deno
                ),
                row![
                    Space::new().width(Length::Fill),
                    button(text(tr!("Ahora no", "Not now")).size(theme::SIZE_S))
                        .on_press(Message::ToolsPopup(false))
                        .padding([6, 14])
                        .style(theme::ghost_button),
                    action,
                ]
                .spacing(8),
            ]
            .spacing(14),
        )
        .padding(20)
        .width(440)
        .style(theme::dialog);
        let backdrop = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::scrim);
        Some(iced::widget::opaque(stack![
            backdrop,
            container(iced::widget::opaque(card))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        ]))
    }
}
