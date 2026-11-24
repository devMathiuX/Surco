//! Barra inferior fija: portada, título, controles, progreso y volumen.

use iced::widget::{button, column, container, image, row, slider, text, Space};
use iced::{Alignment, Element, Length};
use surco_core::{format_duration, PlayerState, RepeatMode};

use crate::icons::{self, Kind};
use crate::playback::TrackRes;
use crate::{theme, Message, Surco};

const COVER: f32 = 56.0;

impl Surco {
    pub(crate) fn view_player_bar(&self) -> Element<'_, Message> {
        let s = &self.status;
        let at = self.now();
        let now = at.and_then(|n| self.track(n));
        let resolving =
            now.is_some_and(|t| matches!(self.resolution(t), Some(TrackRes::Resolving)));

        // Portada (o un cuadrado vacío).
        let cover: Element<'_, Message> = match at
            .and_then(|n| self.video_of(n))
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
            (Some(t), _) if resolving => (t.title.clone(), "buscando en YouTube…".to_owned()),
            (Some(t), PlayerState::Loading) => (t.title.clone(), "cargando…".to_owned()),
            (Some(t), _) => (t.title.clone(), t.artists.join(", ")),
            (None, PlayerState::Loading) => ("Cargando…".to_owned(), String::new()),
            (None, PlayerState::Idle) => ("Nada sonando".to_owned(), String::new()),
            (None, PlayerState::Error(_)) => ("No se pudo reproducir".to_owned(), String::new()),
            (None, _) => (
                s.title.clone().unwrap_or_else(|| "Sin título".to_owned()),
                String::new(),
            ),
        };
        let info = column![
            text(title)
                .size(theme::SIZE_M)
                .wrapping(text::Wrapping::None),
            text(subtitle)
                .size(theme::SIZE_S)
                .style(theme::secondary_text)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill)
        .clip(true);

        // Controles.
        let playing = matches!(s.state, PlayerState::Playing);
        let play = button(
            container(icons::icon(
                if playing { Kind::Pause } else { Kind::Play },
                theme::BACKGROUND,
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
                .width(44)
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

        // Derecha: cambiar el video (si el match no es el correcto) y volumen.
        let change: Element<'_, Message> = match (at, now.and_then(|t| self.resolution(t))) {
            (Some(at), Some(TrackRes::Done(_))) => {
                button(text("Cambiar video").size(theme::SIZE_S))
                    .on_press(Message::OpenChooser(Some(at)))
                    .padding([4, 10])
                    .style(theme::ghost_button)
                    .into()
            }
            _ => Space::new().into(),
        };
        let volume = slider(0.0..=1.0, s.volume, Message::Volume)
            .step(0.01_f32)
            .width(100)
            .style(theme::seek_slider);
        let right = row![
            Space::new().width(Length::Fill),
            change,
            icons::icon(Kind::Volume, theme::TEXT_SECONDARY, 18.0),
            volume,
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill);

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
        .style(theme::surface)
        .into()
    }
}
