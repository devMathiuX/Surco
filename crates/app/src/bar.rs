//! Barra inferior fija: portada, título, controles, progreso y volumen.

use iced::widget::{
    button, column, container, image, mouse_area, responsive, row, slider, text, tooltip,
    vertical_slider, Space,
};
use iced::{Alignment, Element, Length, Task};
use surco_core::{format_duration, PlayerState, RepeatMode};
use surco_player::Source;

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::{theme, Message, Surco};

const COVER: f32 = 56.0;
/// Ancho de la barra de volumen horizontal.
const VOLUME_WIDTH: f32 = 100.0;
/// Botones de ícono de la derecha (radio, cola, volumen) con su separación.
const ICON_SLOT: f32 = 40.0;
/// Lo que ocupa el botón de cambiar el video con su separación.
const CHANGE_SLOT: f32 = ICON_SLOT;
/// Alto del desplegable de volumen.
const POPUP_HEIGHT: f32 = 130.0;
/// Espera antes de cerrar el desplegable al salir con el mouse (para poder
/// ir del ícono al desplegable sin que se cierre).
const POPUP_GRACE: std::time::Duration = std::time::Duration::from_millis(350);
/// Volumen al desmutear si no había uno anterior.
const DEFAULT_VOLUME: f32 = 0.7;

/// Mouse sobre el control de volumen compacto.
#[derive(Debug, Clone, Copy)]
pub enum VolumeMsg {
    /// Clic en el ícono: mutear o volver al volumen de antes.
    ToggleMute,
    /// Entró o salió del ícono (modo compacto).
    IconHover(bool),
    /// Entró o salió del desplegable.
    PopupHover(bool),
    /// Pasó la espera: cerrar si el mouse no está en ninguno.
    Check,
}

/// Estado del control de volumen.
#[derive(Debug, Default)]
pub struct VolumeUi {
    /// Volumen antes de mutear.
    pub before_mute: Option<f32>,
    pub open: bool,
    icon_hover: bool,
    popup_hover: bool,
}

impl Surco {
    pub(crate) fn view_player_bar(&self) -> Element<'_, Message> {
        let s = &self.status;
        let at = self.now_in_playlist();
        let now = self.now_track();
        let resolving =
            now.is_some_and(|t| matches!(self.resolution(t), Some(TrackRes::Resolving)));

        // Portada (o un cuadrado vacío).
        let cover: Element<'_, Message> = match now
            .and_then(|t| self.video_of_track(t))
            .and_then(|v| self.covers.get(v))
        {
            Some(h) => image(h.clone())
                .width(COVER)
                .height(COVER)
                .border_radius(theme::RADIUS)
                .into(),
            None => container(Space::new())
                .width(COVER)
                .height(COVER)
                .style(theme::cover_placeholder)
                .into(),
        };

        // Título y artista.
        let (title, subtitle) = match (now, &s.state) {
            (Some(t), _) if resolving => (
                t.title.clone(),
                tr!("buscando en YouTube…", "searching YouTube…").to_owned(),
            ),
            (Some(t), PlayerState::Loading) => {
                (t.title.clone(), tr!("cargando…", "loading…").to_owned())
            }
            (Some(t), _) => (t.title.clone(), t.artists.join(", ")),
            (None, PlayerState::Loading) => {
                (tr!("Cargando…", "Loading…").to_owned(), String::new())
            }
            (None, PlayerState::Idle) => (
                tr!("Nada sonando", "Nothing playing").to_owned(),
                String::new(),
            ),
            (None, PlayerState::Error(_)) => (
                tr!("No se pudo reproducir", "Couldn't play").to_owned(),
                String::new(),
            ),
            (None, _) => (
                s.title
                    .clone()
                    .unwrap_or_else(|| tr!("Sin título", "Untitled").to_owned()),
                String::new(),
            ),
        };
        // El título que no entra se desliza mientras suena (en Minimal, solo
        // con el mouse encima).
        let slide = matches!(s.state, PlayerState::Playing) && !theme::minimal();
        let info = column![
            crate::marquee::marquee(
                text(title)
                    .size(theme::SIZE_M)
                    .wrapping(text::Wrapping::None),
                slide,
            ),
            crate::marquee::marquee(
                text(subtitle)
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text)
                    .wrapping(text::Wrapping::None),
                slide,
            ),
        ]
        .spacing(2)
        .width(Length::Fill)
        .clip(true);

        // Controles.
        let playing = matches!(s.state, PlayerState::Playing);
        let play = button(
            container(icons::icon(
                if playing { Kind::Pause } else { Kind::Play },
                theme::play_icon(),
                18.0,
            ))
            .center(18),
        )
        .on_press(Message::TogglePause)
        .padding(9)
        .style(theme::play_button);
        let repeat = match self.repeat {
            RepeatMode::One => Kind::RepeatOne,
            _ => Kind::Repeat,
        };
        let controls = row![
            icons::toggle_button(Kind::Shuffle, 18.0, self.shuffle, Message::ToggleShuffle),
            icons::icon_button(Kind::Previous, 18.0, Message::Skip(-1)),
            play,
            icons::icon_button(Kind::Next, 18.0, Message::Skip(1)),
            icons::toggle_button(
                repeat,
                18.0,
                self.repeat != RepeatMode::Off,
                Message::CycleRepeat
            ),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        // Progreso. Mientras el motor no sabe la duración, se usa la de Spotify.
        let shown = self.seeking.unwrap_or(s.position.as_secs_f32());
        let total = s
            .duration
            .or_else(|| now.map(|t| std::time::Duration::from_millis(t.duration_ms)))
            .filter(|d| !d.is_zero());
        let max = total.map_or(1.0, |d| d.as_secs_f32().max(1.0));
        let seek = slider(0.0..=max, shown.min(max), Message::SeekPreview)
            .on_release(Message::SeekCommit)
            .step(0.5_f32)
            .style(theme::seek_slider);
        let time = |ms: Option<u64>| {
            text(ms.map(format_duration).unwrap_or_else(|| "--:--".into()))
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .width(48)
        };
        let progress = row![
            time(Some((shown * 1000.0) as u64)).align_x(Alignment::End),
            seek,
            time(total.map(|d| d.as_millis() as u64)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let center = column![controls, progress]
            .spacing(4)
            .align_x(Alignment::Center)
            .width(Length::FillPortion(2));

        let has_change = matches!(
            (at, now.and_then(|t| self.resolution(t))),
            (Some(_), Some(TrackRes::Done(m))) if !m.candidates.is_empty()
        );
        // Si la barra de volumen no entra, queda solo el ícono y la barra se
        // abre hacia arriba al pasar el mouse.
        let right = responsive(move |size| {
            let needed =
                3.0 * ICON_SLOT + VOLUME_WIDTH + if has_change { CHANGE_SLOT } else { 0.0 };
            let compact = size.width < needed;
            let mut r = row![Space::new().width(Length::Fill)]
                .spacing(10)
                .align_y(Alignment::Center);
            for e in self.right_buttons() {
                r = r.push(e);
            }
            r = r.push(self.volume_icon(compact));
            if !compact {
                r = r.push(
                    slider(0.0..=1.0, self.status.volume, Message::Volume)
                        .step(0.01_f32)
                        .width(VOLUME_WIDTH)
                        .style(theme::seek_slider),
                );
            }
            container(r).center_y(Length::Fill).into()
        })
        .width(Length::Fill)
        .height(COVER);

        let left = row![cover, info]
            .spacing(12)
            .align_y(Alignment::Center)
            .width(Length::Fill);

        container(
            row![left, center, right]
                .spacing(24)
                .align_y(Alignment::Center),
        )
        .padding([10, 16])
        .width(Length::Fill)
        .style(theme::player_bar)
        .into()
    }
}

impl Surco {
    /// Botones de la derecha antes del volumen: cambiar el video, radio y cola.
    fn right_buttons(&self) -> [Element<'_, Message>; 3] {
        let s = &self.status;
        let at = self.now_in_playlist();
        let now = self.now_track();
        // Derecha: cambiar el video (si el match no es el correcto) y volumen.
        let change: Element<'_, Message> = match (at, now.and_then(|t| self.resolution(t))) {
            // Las de la radio no tienen otros candidatos.
            (Some(at), Some(TrackRes::Done(m))) if !m.candidates.is_empty() => tooltip(
                button(icons::icon(Kind::SwapVideo, theme::colors().text, 20.0))
                    .on_press(Message::OpenChooserPopup(at))
                    .padding(5)
                    .style(theme::icon_button),
                container(
                    text(tr!(
                        "Cambiar el video (si no es la canción correcta)",
                        "Change the video (if it's not the right song)"
                    ))
                    .size(theme::SIZE_S),
                )
                .padding(6)
                .style(theme::card),
                tooltip::Position::Top,
            )
            .into(),
            _ => Space::new().into(),
        };
        let can_radio = matches!(s.source, Some(Source::YouTube(_)));
        let radio_icon = icons::icon(
            Kind::Radio,
            if self.radio_loading {
                theme::colors().accent
            } else {
                theme::colors().text
            },
            18.0,
        );
        let radio = tooltip(
            button(radio_icon)
                .on_press_maybe((can_radio && !self.radio_loading).then_some(Message::StartRadio))
                .padding(6)
                .style(theme::side_button),
            container(
                text(if self.radio_loading {
                    tr!("Armando la radio…", "Building the radio…")
                } else {
                    tr!(
                        "Radio: canciones parecidas a esta",
                        "Radio: songs similar to this one"
                    )
                })
                .size(theme::SIZE_S),
            )
            .padding(6)
            .style(theme::card),
            tooltip::Position::Top,
        );
        let queue_on = self.screen == crate::Screen::Queue;
        let queue = button(icons::icon(
            Kind::Queue,
            if queue_on {
                theme::colors().accent
            } else {
                theme::colors().text
            },
            18.0,
        ))
        .on_press(Message::ToggleQueue)
        .padding(6)
        .style(theme::side_button)
        .into();
        [change, radio.into(), queue]
    }

    /// Ícono de volumen: clic mutea. En modo compacto, pasar el mouse abre
    /// la barra hacia arriba.
    fn volume_icon(&self, compact: bool) -> Element<'_, Message> {
        let muted = self.status.volume <= 0.001;
        let icon = icons::icon(
            if muted { Kind::Muted } else { Kind::Volume },
            theme::colors().text,
            18.0,
        );
        let button = button(icon)
            .on_press(Message::VolumeUi(VolumeMsg::ToggleMute))
            .padding(6)
            .style(theme::side_button);
        if compact {
            mouse_area(button)
                .on_enter(Message::VolumeUi(VolumeMsg::IconHover(true)))
                .on_exit(Message::VolumeUi(VolumeMsg::IconHover(false)))
                .into()
        } else {
            button.into()
        }
    }

    /// Desplegable con la barra de volumen vertical, sobre el ícono.
    pub(crate) fn view_volume_popup(&self) -> Option<Element<'_, Message>> {
        if !self.volume_ui.open {
            return None;
        }
        let slider = vertical_slider(0.0..=1.0, self.status.volume, Message::Volume)
            .step(0.01_f32)
            .height(POPUP_HEIGHT - 44.0)
            .style(theme::seek_slider);
        let card = container(
            column![
                slider,
                text(format!("{:.0}", self.status.volume * 100.0))
                    .size(theme::SIZE_S)
                    .style(theme::secondary_text),
            ]
            .spacing(8)
            .align_x(Alignment::Center),
        )
        .padding([12, 8])
        .width(44)
        .height(POPUP_HEIGHT)
        .style(theme::dialog);
        let card = mouse_area(card)
            .on_enter(Message::VolumeUi(VolumeMsg::PopupHover(true)))
            .on_exit(Message::VolumeUi(VolumeMsg::PopupHover(false)));
        // Abajo a la derecha, justo arriba del ícono (el último de la barra).
        Some(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_right(Length::Fill)
                .align_bottom(Length::Fill)
                .padding(iced::Padding::ZERO.right(9).bottom(70))
                .into(),
        )
    }

    pub(crate) fn on_volume_ui(&mut self, msg: VolumeMsg) -> Task<Message> {
        let ui = &mut self.volume_ui;
        match msg {
            VolumeMsg::ToggleMute => {
                let v = if self.status.volume > 0.001 {
                    ui.before_mute = Some(self.status.volume);
                    0.0
                } else {
                    ui.before_mute
                        .take()
                        .filter(|v| *v > 0.001)
                        .unwrap_or(DEFAULT_VOLUME)
                };
                return Task::done(Message::Volume(v));
            }
            VolumeMsg::IconHover(true) => {
                ui.icon_hover = true;
                ui.open = true;
            }
            VolumeMsg::PopupHover(true) => ui.popup_hover = true,
            VolumeMsg::IconHover(false) | VolumeMsg::PopupHover(false) => {
                if matches!(msg, VolumeMsg::IconHover(_)) {
                    ui.icon_hover = false;
                } else {
                    ui.popup_hover = false;
                }
                return Task::perform(tokio::time::sleep(POPUP_GRACE), |_| {
                    Message::VolumeUi(VolumeMsg::Check)
                });
            }
            VolumeMsg::Check => {
                if !ui.icon_hover && !ui.popup_hover {
                    ui.open = false;
                }
            }
        }
        Task::none()
    }
}
