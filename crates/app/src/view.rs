//! Vistas: barra lateral, estado vacío, lista de la playlist (con búsqueda),
//! selector de video y panel de prueba.

use iced::widget::{
    button, column, container, image, row, scrollable, text, text_input, toggler, tooltip, Column,
    Id, Space,
};
use iced::{Alignment, Element, Length};
use surco_core::{format_duration, PlayerState, Playlist, Track};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::settings::{Change, Precache};
use crate::{theme, Message, Surco, YtDlpState, FILTER_INPUT, IMPORT_INPUT};

impl Surco {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let main: Element<'_, Message> = match self.selected.and_then(|i| self.playlists.get(i)) {
            _ if self.show_settings => self.view_settings(),
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

    pub(crate) fn view_sidebar(&self) -> Element<'_, Message> {
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
        let pick_csv = button(text("Elegir CSV…").size(theme::SIZE_S))
            .on_press_maybe((!self.importing).then_some(Message::PickCsv))
            .width(Length::Fill)
            .padding([6, 10])
            .style(theme::ghost_button);

        let gear = tooltip(
            button(icons::icon(
                Kind::Settings,
                if self.show_settings {
                    theme::colors().accent
                } else {
                    theme::colors().text_secondary
                },
                18.0,
            ))
            .on_press(Message::ToggleSettings)
            .padding(4)
            .style(theme::icon_button),
            container(text("Ajustes").size(theme::SIZE_S))
                .padding(6)
                .style(theme::card),
            tooltip::Position::Bottom,
        );
        let mut col = column![
            row![
                button(text("Surco").size(theme::SIZE_L))
                    .on_press(Message::SelectPlaylist(None))
                    .padding(0)
                    .style(theme::list_button(false)),
                Space::new().width(Length::Fill),
                gear,
            ]
            .align_y(Alignment::Center),
            scrollable(list).height(Length::Fill),
            input,
            column![import, pick_csv].spacing(4),
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

    pub(crate) fn view_empty(&self) -> Element<'_, Message> {
        column![
            text("Pegá un link de playlist de Spotify para empezar").size(theme::SIZE_L),
            text("Usá el campo de la izquierda (Ctrl+L) o elegí un CSV exportado con Exportify. Clic en una canción para escucharla.")
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            text("Atajos: Espacio pausa · ←/→ 5 s · Ctrl+←/→ anterior/siguiente · Ctrl+F buscar · Esc cierra")
                .size(theme::SIZE_S)
                .style(theme::secondary_text),
            Space::new().height(24),
            self.view_test_panel(),
        ]
        .spacing(8)
        .into()
    }

    pub(crate) fn view_playlist<'a>(
        &'a self,
        index: usize,
        p: &'a Playlist,
        source: &str,
    ) -> Element<'a, Message> {
        let total_ms: u64 = p.tracks.iter().map(|t| t.duration_ms).sum();
        let origin = if source == "radio" {
            "canciones parecidas, del Mix de YouTube".to_owned()
        } else {
            format!("importada desde {source}")
        };
        let mut info = format!(
            "{} canciones · {} · {origin}",
            p.tracks.len(),
            format_duration(total_ms)
        );
        if self.active.as_ref().is_some_and(|a| a.playlist == index) && self.precache.target > 0 {
            info.push_str(&format!(
                " · precargadas {} de {}{}",
                self.precache.ready,
                self.precache.target,
                if self.precache.running.is_some() {
                    " (bajando…)"
                } else {
                    ""
                }
            ));
        }
        let header = row![
            column![
                text(&p.name).size(theme::SIZE_L),
                text(info).size(theme::SIZE_S).style(theme::secondary_text),
            ]
            .spacing(4)
            .width(Length::Fill),
            button(text("Quitar").size(theme::SIZE_S))
                .on_press(Message::RemovePlaylist(index))
                .padding([4, 10])
                .style(theme::ghost_button),
        ]
        .align_y(Alignment::Center);

        let query = self.filter.as_deref().unwrap_or("");
        let shown = p
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| matches_filter(t, query));
        let rows = Column::with_children(shown.map(|(i, t)| {
            let playing = self.now() == Some((index, i));
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
            let number = container(if playing {
                icons::icon(Kind::Volume, theme::colors().accent, 16.0)
            } else {
                text(i + 1)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .into()
            })
            .width(40);
            let cover: Element<'_, Message> = if self.settings.covers_in_list {
                match self.video_of((index, i)).and_then(|v| self.covers.get(v)) {
                    Some(h) => image(h.clone())
                        .width(LIST_COVER)
                        .height(LIST_COVER)
                        .border_radius(4.0)
                        .into(),
                    None => container(Space::new())
                        .width(LIST_COVER)
                        .height(LIST_COVER)
                        .style(theme::cover_placeholder)
                        .into(),
                }
            } else {
                Space::new().width(0).into()
            };
            button(
                row![
                    number,
                    cover,
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
        if let Some(q) = &self.filter {
            col = col.push(
                row![
                    icons::icon(Kind::Search, theme::colors().text_secondary, 18.0),
                    text_input("Buscar en la playlist", q)
                        .id(Id::new(FILTER_INPUT))
                        .on_input(Message::FilterChanged)
                        .padding(8)
                        .size(theme::SIZE_S)
                        .style(theme::input),
                    button(text("Cerrar").size(theme::SIZE_S))
                        .on_press(Message::CloseFilter)
                        .padding([4, 10])
                        .style(theme::ghost_button),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        if !query.trim().is_empty() && !p.tracks.iter().any(|t| matches_filter(t, query)) {
            col = col.push(
                text(format!("Nada coincide con «{}».", query.trim()))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            );
        }
        if let Some(chooser) = self.view_chooser(index) {
            col = col.push(chooser);
        }
        col.push(scrollable(rows).height(Length::Fill)).into()
    }

    /// Pantalla de ajustes: tema, portadas, precarga y radio automática.
    pub(crate) fn view_settings(&self) -> Element<'_, Message> {
        let section = |title: &'static str, hint: &'static str| {
            column![
                text(title).size(theme::SIZE_M),
                text(hint).size(theme::SIZE_S).style(theme::secondary_text),
            ]
            .spacing(2)
        };

        // Temas: botón con dos muestras de color (fondo y acento).
        let themes = row(theme::THEMES.iter().enumerate().map(|(i, t)| {
            let sw = |c| {
                container(Space::new())
                    .width(14)
                    .height(14)
                    .style(theme::swatch(c))
            };
            button(
                row![
                    sw(t.background),
                    sw(t.accent),
                    text(t.name).size(theme::SIZE_S)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Setting(Change::Theme(i)))
            .padding([6, 10])
            .style(theme::choice_button(self.settings.theme == i))
            .into()
        }))
        .spacing(8)
        .wrap();

        let precache = row(Precache::ALL.iter().map(|&p| {
            button(text(p.label()).size(theme::SIZE_S))
                .on_press(Message::Setting(Change::Precache(p)))
                .padding([6, 10])
                .style(theme::choice_button(self.settings.precache == p))
                .into()
        }))
        .spacing(8);
        let usage = self
            .provider
            .as_ref()
            .and_then(|p| p.audio_cache())
            .map(|c| c.usage())
            .unwrap_or_default();
        let cache_row = row![
            text(format!(
                "En disco: {} canciones, {}",
                usage.files,
                format_bytes(usage.bytes)
            ))
            .size(theme::SIZE_S)
            .style(theme::secondary_text),
            button(text("Vaciar").size(theme::SIZE_S))
                .on_press_maybe((usage.files > 0).then_some(Message::ClearAudioCache))
                .padding([4, 10])
                .style(theme::ghost_button),
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let switch = |label: &'static str, on: bool, f: fn(bool) -> Change| {
            toggler(on)
                .label(label)
                .on_toggle(move |v| Message::Setting(f(v)))
                .size(20)
                .text_size(theme::SIZE_S)
                .spacing(10)
                .style(theme::switch)
        };

        let body = column![
            text("Ajustes").size(theme::SIZE_L),
            column![
                section("Tema", "Cómo se ve Surco."),
                themes,
            ]
            .spacing(10),
            column![
                section(
                    "Portadas en la lista",
                    "Muestra la portada al lado de cada canción ya encontrada en YouTube."
                ),
                switch(
                    "Mostrar portadas",
                    self.settings.covers_in_list,
                    Change::CoversInList
                ),
            ]
            .spacing(10),
            column![
                section(
                    "Precargar canciones",
                    "Baja el audio antes de que suene, así no hay que esperar a que cargue. La siguiente siempre se prepara en memoria para que no haya cortes."
                ),
                precache,
                cache_row,
            ]
            .spacing(10),
            column![
                section(
                    "Canciones parecidas",
                    "Con el botón de radio de la barra de abajo armás una lista de canciones parecidas a la que suena."
                ),
                switch(
                    "Al terminar, seguir con canciones parecidas",
                    self.settings.autoplay,
                    Change::Autoplay
                ),
            ]
            .spacing(10),
        ]
        .spacing(28)
        .max_width(760);
        scrollable(body).height(Length::Fill).into()
    }

    /// Selector de video para una canción (los candidatos con su puntaje).
    pub(crate) fn view_chooser(&self, playlist: usize) -> Option<Element<'_, Message>> {
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
    pub(crate) fn view_test_panel(&self) -> Element<'_, Message> {
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
}

/// Tamaño de las portadas en la lista.
const LIST_COVER: f32 = 36.0;

/// `1536000` → `1,5 MB`.
fn format_bytes(b: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = b as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0).replace('.', ",")
    } else {
        format!("{mb:.0} MB")
    }
}

/// ¿La canción coincide con la búsqueda? (sin distinguir mayúsculas ni tildes)
pub fn matches_filter(track: &Track, query: &str) -> bool {
    let q = fold(query);
    if q.is_empty() {
        return true;
    }
    let hay = fold(&format!(
        "{} {} {}",
        track.title,
        track.artists.join(" "),
        track.album.as_deref().unwrap_or("")
    ));
    q.split_whitespace().all(|w| hay.contains(w))
}

fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_ignores_case_and_accents() {
        let t = Track {
            title: "Canción Animal".into(),
            artists: vec!["Soda Stereo".into()],
            album: None,
            duration_ms: 0,
            isrc: None,
            spotify_id: None,
        };
        assert!(matches_filter(&t, "cancion"));
        assert!(matches_filter(&t, "SODA animal"));
        assert!(!matches_filter(&t, "cerati"));
        assert!(matches_filter(&t, "  "));
    }
}
