//! Pantalla de ajustes: pestañas (apariencia, reproducción, descargas,
//! sistema y acerca de), un buscador que filtra entre todas, y cada sección
//! en su tarjeta. Con la ventana ancha, en dos columnas.

use iced::widget::{
    button, column, container, progress_bar, responsive, row, scrollable, text, text_input,
    toggler, Column, Space,
};
use iced::{Alignment, Element, Length};
use surco_player::eq::{Preset, BANDS, MAX_GAIN};

use crate::icons::{self, Kind};
use crate::settings::Change;
use crate::updater::{ManualCheck, UpdateMsg};
use crate::view::{format_bytes, songs};
use crate::{i18n, theme, Message, Surco};

/// Pestaña de los ajustes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Look,
    Playback,
    Downloads,
    System,
    About,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Look,
        Tab::Playback,
        Tab::Downloads,
        Tab::System,
        Tab::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Look => tr!("Apariencia", "Appearance"),
            Tab::Playback => tr!("Reproducción", "Playback"),
            Tab::Downloads => tr!("Descargas", "Downloads"),
            Tab::System => tr!("Sistema", "System"),
            Tab::About => tr!("Acerca de", "About"),
        }
    }

    fn icon(self) -> Kind {
        match self {
            Tab::Look => Kind::Edit,
            Tab::Playback => Kind::Play,
            Tab::Downloads => Kind::Import,
            Tab::System => Kind::Settings,
            Tab::About => Kind::Queue,
        }
    }
}

/// Ancho a partir del cual las secciones van en dos columnas.
const TWO_COLUMNS: f32 = 900.0;

/// Una sección: pestaña, columna (0 izquierda, 1 derecha), título, palabras
/// para el buscador y contenido.
struct Section<'a> {
    tab: Tab,
    right: bool,
    title: &'static str,
    keywords: &'static str,
    body: Element<'a, Message>,
}

/// Saca acentos y pasa a minúsculas, para buscar sin importar cómo se
/// escriba.
pub(crate) fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// Si una sección coincide con lo buscado.
fn matches(query: &str, title: &str, keywords: &str) -> bool {
    let q = fold(query.trim());
    q.is_empty() || fold(title).contains(&q) || fold(keywords).contains(&q)
}

/// Fila de un ajuste: título y explicación a la izquierda, control a la
/// derecha.
fn setting_row<'a>(
    title: &'static str,
    hint: &'static str,
    control: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(theme::SIZE_M),
            text(hint).size(theme::SIZE_S).style(theme::secondary_text),
        ]
        .spacing(2)
        .width(Length::Fill),
        control.into(),
    ]
    .spacing(16)
    .align_y(Alignment::Center)
    .into()
}

fn switch<'a>(on: bool, f: fn(bool) -> Change) -> Element<'a, Message> {
    toggler(on)
        .on_toggle(move |v| Message::Setting(f(v)))
        .size(22)
        .style(theme::switch)
        .into()
}

fn hint(s: &'static str) -> iced::widget::Text<'static> {
    text(s).size(theme::SIZE_S).style(theme::secondary_text)
}

impl Surco {
    pub(crate) fn view_settings(&self) -> Element<'_, Message> {
        let query = self.settings_query.trim();
        let tabs = row(Tab::ALL.iter().map(|&t| {
            button(
                row![
                    icons::icon(t.icon(), theme::colors().text_secondary, 14.0),
                    text(t.label()).size(theme::SIZE_S),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .on_press(Message::SettingsTab(t))
            .padding([7, 14])
            .style(theme::pill(query.is_empty() && self.settings_tab == t))
            .into()
        }))
        .spacing(8)
        .wrap();
        let search = container(
            row![
                icons::icon(Kind::Search, theme::colors().text_secondary, 16.0),
                text_input(
                    tr!("Buscar en ajustes…", "Search settings…"),
                    &self.settings_query
                )
                .on_input(Message::SettingsQuery)
                .padding(8)
                .size(theme::SIZE_S)
                .style(theme::input),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(280);
        let header = row![
            text(tr!("Ajustes", "Settings")).size(theme::SIZE_L + 6),
            Space::new().width(Length::Fill),
            search,
        ]
        .align_y(Alignment::Center);

        let content = responsive(move |size| {
            let sections: Vec<Section<'_>> = self
                .settings_sections()
                .into_iter()
                .filter(|s| {
                    if query.is_empty() {
                        s.tab == self.settings_tab
                    } else {
                        matches(query, s.title, s.keywords)
                    }
                })
                .collect();
            let empty = sections.is_empty();
            let (mut left, mut right) = (Vec::new(), Vec::new());
            for s in sections {
                let card: Element<'_, Message> = column![
                    text(s.title).size(theme::SIZE_M + 1),
                    container(s.body)
                        .padding(16)
                        .width(Length::Fill)
                        .style(theme::card),
                ]
                .spacing(10)
                .into();
                if s.right {
                    right.push(card);
                } else {
                    left.push(card);
                }
            }
            let body: Element<'_, Message> = if empty {
                hint(tr!(
                    "No hay ajustes con ese nombre.",
                    "No settings match that."
                ))
                .into()
            } else if size.width >= TWO_COLUMNS && !right.is_empty() && !left.is_empty() {
                row![
                    Column::with_children(left)
                        .spacing(24)
                        .width(Length::FillPortion(1)),
                    Column::with_children(right)
                        .spacing(24)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(24)
                .into()
            } else {
                Column::with_children(left.into_iter().chain(right))
                    .spacing(24)
                    .max_width(760)
                    .into()
            };
            crate::smooth::smooth(
                scrollable(container(body).padding(iced::Padding::ZERO.right(16)))
                    .height(Length::Fill),
            )
        });
        column![header, tabs, content].spacing(18).into()
    }

    fn settings_sections<'a>(&'a self) -> Vec<Section<'a>> {
        let mut out: Vec<Section<'a>> = Vec::new();
        let mut push = |tab, right, title, keywords, body: Element<'a, Message>| {
            out.push(Section {
                tab,
                right,
                title,
                keywords,
                body,
            });
        };

        // Apariencia.
        let custom =
            (self.settings.theme == crate::theme::CUSTOM).then(|| self.view_custom_theme());
        push(
            Tab::Look,
            false,
            tr!("Cómo se ve Surco", "How Surco looks"),
            "tema theme colores colors oscuro claro vidrio brutal brillo minimal personalizado custom apariencia appearance",
            column![self.theme_cards(true, true)].push(custom).spacing(14).into(),
        );
        push(
            Tab::Look,
            true,
            tr!("Idioma y tamaño", "Language and size"),
            "idioma language español english tamaño size zoom escala scale interfaz interface",
            column![
                setting_row(
                    tr!("Idioma", "Language"),
                    tr!(
                        "Por defecto, el del sistema.",
                        "Defaults to your system language."
                    ),
                    self.languages_row(),
                ),
                setting_row(
                    tr!("Tamaño de la interfaz", "Interface size"),
                    tr!(
                        "También con Ctrl + y Ctrl −.",
                        "Also with Ctrl + and Ctrl −."
                    ),
                    self.scale_row(),
                ),
            ]
            .spacing(16)
            .into(),
        );
        push(
            Tab::Look,
            true,
            tr!("Portadas en la lista", "Covers in the list"),
            "portadas covers lista list imagen image",
            setting_row(
                tr!("Mostrar portadas en la lista", "Show covers in the list"),
                tr!(
                    "La portada al lado de cada canción encontrada en YouTube.",
                    "The cover next to each song found on YouTube."
                ),
                switch(self.settings.covers_in_list, Change::CoversInList),
            ),
        );

        // Reproducción.
        push(
            Tab::Playback,
            false,
            tr!("Ecualizador", "Equalizer"),
            "ecualizador equalizer eq graves bass agudos treble sonido sound",
            self.view_equalizer(),
        );
        push(
            Tab::Playback,
            true,
            tr!("Canciones parecidas", "Similar songs"),
            "radio parecidas similar autoplay cola queue",
            column![
                setting_row(
                    tr!("Radio automática", "Automatic radio"),
                    tr!(
                        "Al terminar la cola, seguir con canciones parecidas.",
                        "When the queue ends, keep playing similar songs."
                    ),
                    switch(self.settings.autoplay, Change::Autoplay),
                ),
                hint(tr!(
                    "Con el botón de radio de la barra de abajo armás una lista de canciones parecidas a la que suena.",
                    "The radio button in the bottom bar builds a list of songs similar to the one playing."
                )),
            ]
            .spacing(12)
            .into(),
        );

        // Descargas.
        push(
            Tab::Downloads,
            false,
            tr!("Precargar canciones", "Preload songs"),
            "precarga preload cache caché disco disk descargas downloads",
            self.view_precache(),
        );

        // Sistema.
        push(
            Tab::System,
            false,
            tr!("Herramientas", "Tools"),
            "yt-dlp deno ruta path herramientas tools sistema system",
            self.view_tools_status(),
        );

        // Acerca de.
        push(
            Tab::About,
            false,
            tr!("Acerca de", "About"),
            "acerca about version versión licencias licenses actualizaciones updates",
            self.view_about(),
        );
        out
    }

    fn view_precache(&self) -> Element<'_, Message> {
        let usage = self
            .provider
            .as_ref()
            .and_then(|p| p.audio_cache())
            .map(|c| c.usage())
            .unwrap_or_default();
        let limit = surco_provider::AUDIO_CACHE_LIMIT.max(1);
        let used = if i18n::en() {
            format!(
                "On disk: {} ({})",
                songs(usage.files),
                format_bytes(usage.bytes)
            )
        } else {
            format!(
                "En disco: {} ({})",
                songs(usage.files),
                format_bytes(usage.bytes)
            )
        };
        let disk = container(
            row![
                column![
                    hint_owned(used),
                    progress_bar(0.0..=1.0, (usage.bytes as f32 / limit as f32).min(1.0))
                        .girth(6)
                        .style(theme::meter),
                ]
                .spacing(8)
                .width(Length::Fill),
                button(text(tr!("Vaciar caché", "Clear cache")).size(theme::SIZE_S))
                    .on_press_maybe((usage.files > 0).then_some(Message::ClearAudioCache))
                    .padding([6, 12])
                    .style(theme::ghost_button),
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .padding(12)
        .style(theme::inset);
        column![
            hint(tr!(
                "Baja el audio antes de que suene, así no hay que esperar a que cargue. La siguiente siempre se prepara en memoria para que no haya cortes. Se borra al cerrar Surco.",
                "Downloads the audio before it plays, so you don't wait for it to load. The next song is always prepared in memory so there are no gaps. It's cleared when Surco closes."
            )),
            self.precache_row(),
            disk,
        ]
        .spacing(14)
        .into()
    }

    fn view_tools_status(&self) -> Element<'_, Message> {
        let ok = |good: bool| {
            text(if good { "✓" } else { "✕" })
                .size(theme::SIZE_M)
                .style(if good {
                    theme::accent_text
                } else {
                    theme::error_text
                })
        };
        let ytdlp: Element<'_, Message> = match &self.ytdlp {
            crate::YtDlpState::Found(p) => container(
                row![
                    text(p.display().to_string())
                        .size(theme::SIZE_S)
                        .font(iced::Font::MONOSPACE)
                        .width(Length::Fill),
                    ok(true),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .padding([8, 12])
            .style(theme::inset)
            .into(),
            crate::YtDlpState::Downloading => hint(tr!("Descargando…", "Downloading…")).into(),
            _ => row![
                ok(false),
                hint(tr!("No encontrado", "Not found")).width(Length::Fill),
                button(text(tr!("Descargar", "Download")).size(theme::SIZE_S))
                    .on_press(Message::DownloadYtDlp)
                    .padding([6, 12])
                    .style(theme::primary_button),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        };
        let deno: Element<'_, Message> = match &self.deno {
            crate::DenoState::Ok => row![ok(true), hint(tr!("Listo", "Ready"))]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
            crate::DenoState::Downloading => hint(tr!("Descargando…", "Downloading…")).into(),
            _ => row![
                ok(false),
                hint(tr!("Falta", "Missing")).width(Length::Fill),
                button(text(tr!("Instalar", "Install")).size(theme::SIZE_S))
                    .on_press(Message::DownloadDeno)
                    .padding([6, 12])
                    .style(theme::primary_button),
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        };
        column![
            column![
                text("yt-dlp").size(theme::SIZE_M),
                hint(tr!(
                    "Busca en YouTube y consigue el audio. Surco lo mantiene actualizado.",
                    "Searches YouTube and gets the audio. Surco keeps it up to date."
                )),
                ytdlp,
            ]
            .spacing(8),
            column![
                text("deno").size(theme::SIZE_M),
                hint(tr!(
                    "Evita que YouTube rechace algunas canciones.",
                    "Keeps YouTube from rejecting some songs."
                )),
                deno,
            ]
            .spacing(8),
        ]
        .spacing(20)
        .into()
    }

    fn view_about(&self) -> Element<'_, Message> {
        let status: Element<'_, Message> = match self.update_check {
            ManualCheck::Idle => Space::new().into(),
            ManualCheck::Checking => hint(tr!("Buscando…", "Checking…")).into(),
            ManualCheck::UpToDate => hint(tr!(
                "Tenés la última versión.",
                "You have the latest version."
            ))
            .into(),
            ManualCheck::Found => hint(tr!(
                "Hay una versión nueva: mirá el aviso abajo a la izquierda.",
                "There's a new version: see the notice at the bottom left."
            ))
            .into(),
            ManualCheck::Failed => text(tr!(
                "No se pudo consultar. ¿Hay internet?",
                "Couldn't check. Are you online?"
            ))
            .size(theme::SIZE_S)
            .style(theme::error_text)
            .into(),
        };
        row![
            container(icons::logo(72.0)).padding(4),
            column![
                text("Surco").size(theme::SIZE_L),
                hint_owned(trf!(
                    "Surco {} · © 2026 mathiux · Licencia MIT",
                    "Surco {} · © 2026 mathiux · MIT License",
                    env!("CARGO_PKG_VERSION")
                )),
                hint(tr!(
                    "Surco no está afiliado a YouTube, Google ni Spotify. Sus marcas son de sus dueños.",
                    "Surco isn't affiliated with YouTube, Google or Spotify. Their trademarks belong to their owners."
                )),
                row![
                    button(text(tr!("Licencias", "Licenses")).size(theme::SIZE_S))
                        .on_press(Message::OpenLicenses)
                        .padding([6, 12])
                        .style(theme::ghost_button),
                    button(
                        text(tr!("Buscar actualizaciones", "Check for updates"))
                            .size(theme::SIZE_S)
                    )
                    .on_press_maybe(
                        (self.update_check != ManualCheck::Checking)
                            .then_some(Message::Update(UpdateMsg::CheckNow))
                    )
                    .padding([6, 12])
                    .style(theme::ghost_button),
                ]
                .spacing(8),
                status,
            ]
            .spacing(6)
            .width(Length::Fill),
        ]
        .spacing(18)
        .align_y(Alignment::Center)
        .into()
    }

    /// Ecualizador de 10 bandas: prendido/apagado, ajustes de fábrica y un
    /// slider vertical por banda.
    fn view_equalizer(&self) -> Element<'_, Message> {
        let e = self.settings.equalizer;
        let matching = Preset::matching(&e.gains);
        let presets = row(Preset::ALL.iter().map(|&p| {
            button(text(preset_name(p)).size(theme::SIZE_S))
                .on_press(Message::Setting(Change::EqPreset(p)))
                .padding([6, 12])
                .style(theme::pill(matching == Some(p)))
                .into()
        }))
        .push(matching.is_none().then(|| {
            button(text(tr!("Personalizado", "Custom")).size(theme::SIZE_S))
                .padding([6, 12])
                .style(theme::pill(true))
        }))
        .spacing(8)
        .wrap();
        let label = |hz: f32| {
            if hz >= 1000.0 {
                format!("{}k", hz / 1000.0)
            } else {
                format!("{hz}")
            }
        };
        let bands = row(BANDS.iter().zip(e.gains).enumerate().map(|(i, (&hz, g))| {
            column![
                text(if g == 0.0 {
                    "0".to_owned()
                } else {
                    format!("{g:+}")
                })
                .size(11)
                .style(theme::secondary_text),
                iced::widget::vertical_slider(-MAX_GAIN..=MAX_GAIN, g, move |v| {
                    Message::Setting(Change::EqBand(i, v))
                })
                .step(0.5)
                .height(120)
                .on_release(Message::Setting(Change::EqSave))
                .style(theme::seek_slider),
                text(label(hz)).size(11).style(if e.enabled {
                    theme::plain_text
                } else {
                    theme::secondary_text
                }),
            ]
            .spacing(6)
            .width(40)
            .align_x(Alignment::Center)
            .into()
        }))
        .spacing(4);
        column![
            setting_row(
                tr!("Activado", "On"),
                tr!(
                    "Subí o bajá graves, medios y agudos. Se oye al instante.",
                    "Boost or cut bass, mids and treble. You hear it right away."
                ),
                switch(e.enabled, Change::EqEnabled),
            ),
            presets,
            bands,
        ]
        .spacing(14)
        .into()
    }
}

fn hint_owned(s: String) -> iced::widget::Text<'static> {
    text(s).size(theme::SIZE_S).style(theme::secondary_text)
}

/// Nombre de un ajuste de fábrica del ecualizador.
fn preset_name(p: Preset) -> &'static str {
    match p {
        Preset::Flat => tr!("Plano", "Flat"),
        Preset::Bass => tr!("Graves", "Bass"),
        Preset::Treble => tr!("Agudos", "Treble"),
        Preset::Vocal => tr!("Voces", "Vocal"),
        Preset::Rock => tr!("Rock", "Rock"),
        Preset::Pop => tr!("Pop", "Pop"),
        Preset::Electronic => tr!("Electrónica", "Electronic"),
        Preset::Acoustic => tr!("Acústica", "Acoustic"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ignores_case_and_accents() {
        assert!(matches("ECUALIZ", "Ecualizador", ""));
        assert!(matches("musica", "", "música"));
        assert!(matches("", "x", "y"));
        assert!(!matches("deno", "Tema", "colores"));
    }
}
