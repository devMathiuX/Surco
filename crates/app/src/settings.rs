//! Ajustes del usuario (pantalla de ajustes): tema, portadas en la lista,
//! precarga y radio automática. Se guardan en la tabla de ajustes de la
//! biblioteca.

use iced::{Color, Task};

use crate::i18n::{self, Lang};
use crate::{theme, Message, Surco};

const THEME: &str = "theme";
const THEME_BACKGROUND: &str = "theme_background";
const THEME_ACCENT: &str = "theme_accent";
const LANGUAGE: &str = "language";
const COVERS_IN_LIST: &str = "covers_in_list";
const PRECACHE: &str = "precache";
const AUTOPLAY: &str = "autoplay";
const UI_SCALE: &str = "ui_scale";

/// Tamaño de la interfaz, en %: de a 10, entre estos límites.
pub const SCALE_MIN: u32 = 70;
pub const SCALE_MAX: u32 = 160;
pub const SCALE_STEP: u32 = 10;

/// Cuánto se baja de antemano a disco.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Precache {
    /// Nada: se escucha en streaming (igual se pre-carga la siguiente en memoria).
    #[default]
    Off,
    /// Las próximas canciones de la cola.
    Next,
    /// Toda la playlist que está sonando.
    Playlist,
}

impl Precache {
    pub const ALL: [Precache; 3] = [Precache::Off, Precache::Next, Precache::Playlist];

    pub fn label(self) -> &'static str {
        match self {
            Precache::Off => tr!("No", "Off"),
            Precache::Next => tr!("Las próximas 5", "Next 5"),
            Precache::Playlist => tr!("Toda la playlist", "Whole playlist"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Precache::Off => "off",
            Precache::Next => "next",
            Precache::Playlist => "all",
        }
    }

    fn from_key(s: &str) -> Self {
        match s {
            "next" => Precache::Next,
            "all" => Precache::Playlist,
            _ => Precache::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// Índice en [`theme::THEMES`], o [`theme::CUSTOM`].
    pub theme: usize,
    /// Fondo y acento del tema personalizado.
    pub custom: (Color, Color),
    pub language: Lang,
    pub covers_in_list: bool,
    pub precache: Precache,
    /// Al terminar la cola, seguir con canciones parecidas.
    pub autoplay: bool,
    /// Tamaño de la interfaz, en % (100 = normal).
    pub ui_scale: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: 0,
            custom: theme::CUSTOM_DEFAULT,
            language: Lang::Es,
            covers_in_list: true,
            precache: Precache::Off,
            autoplay: true,
            ui_scale: 100,
        }
    }
}

/// Cambios desde la pantalla de ajustes.
#[derive(Debug, Clone)]
pub enum Change {
    Theme(usize),
    /// Texto de los campos de color del tema personalizado (0 fondo, 1 acento).
    CustomColor(usize, String),
    Language(Lang),
    CoversInList(bool),
    Precache(Precache),
    Autoplay(bool),
    /// Agrandar (+1) o achicar (-1) la interfaz un paso; 0 la vuelve al 100 %.
    Scale(i32),
}

impl Surco {
    pub(crate) fn load_settings(&mut self) {
        let get = |k| {
            self.store
                .as_ref()
                .and_then(|s| s.setting(k).ok().flatten())
        };
        let mut s = Settings::default();
        if let Some(i) = get(THEME).as_deref().and_then(theme::index_of) {
            s.theme = i;
        }
        if let Some(c) = get(THEME_BACKGROUND).as_deref().and_then(theme::parse_hex) {
            s.custom.0 = c;
        }
        if let Some(c) = get(THEME_ACCENT).as_deref().and_then(theme::parse_hex) {
            s.custom.1 = c;
        }
        s.language = get(LANGUAGE)
            .as_deref()
            .and_then(Lang::from_key)
            .unwrap_or_else(Lang::system);
        if let Some(v) = get(COVERS_IN_LIST) {
            s.covers_in_list = v == "1";
        }
        if let Some(v) = get(PRECACHE) {
            s.precache = Precache::from_key(&v);
        }
        if let Some(v) = get(AUTOPLAY) {
            s.autoplay = v == "1";
        }
        if let Some(v) = get(UI_SCALE).and_then(|v| v.parse::<u32>().ok()) {
            s.ui_scale = v.clamp(SCALE_MIN, SCALE_MAX);
        }
        self.settings = s;
        self.custom_hex = [theme::to_hex(s.custom.0), theme::to_hex(s.custom.1)];
        i18n::set(s.language);
        self.theme = theme::set(s.theme, s.custom);
    }

    /// Borra las claves de la Web API de Spotify que guardaban las versiones
    /// anteriores (desde la 1.0 no se usan).
    pub(crate) fn forget_spotify_keys(&self) {
        let Some(store) = &self.store else {
            return;
        };
        for key in ["spotify_client_id", "spotify_client_secret"] {
            if store
                .setting(key)
                .ok()
                .flatten()
                .is_some_and(|v| !v.is_empty())
            {
                self.save_setting(key, "");
            }
        }
    }

    pub(crate) fn save_setting(&self, key: &str, value: &str) {
        if let Some(store) = &self.store {
            if let Err(e) = store.set_setting(key, value) {
                tracing::warn!(error = %e, "guardar ajuste");
            }
        }
    }

    pub(crate) fn change_setting(&mut self, change: Change) -> Task<Message> {
        let flag = |b: bool| if b { "1" } else { "0" };
        match change {
            Change::Theme(i) => {
                self.settings.theme = i.min(theme::CUSTOM);
                self.theme = theme::set(self.settings.theme, self.settings.custom);
                self.save_setting(THEME, theme::id_of(self.settings.theme));
                Task::none()
            }
            Change::CustomColor(which, value) => {
                let which = which.min(1);
                let parsed = theme::parse_hex(&value);
                self.custom_hex[which] = value;
                if let Some(c) = parsed {
                    let (key, slot) = if which == 0 {
                        (THEME_BACKGROUND, &mut self.settings.custom.0)
                    } else {
                        (THEME_ACCENT, &mut self.settings.custom.1)
                    };
                    *slot = c;
                    self.save_setting(key, &theme::to_hex(c));
                    self.settings.theme = theme::CUSTOM;
                    self.save_setting(THEME, theme::id_of(theme::CUSTOM));
                    self.theme = theme::set(theme::CUSTOM, self.settings.custom);
                }
                Task::none()
            }
            Change::Language(lang) => {
                self.settings.language = lang;
                i18n::set(lang);
                self.save_setting(LANGUAGE, lang.key());
                Task::none()
            }
            Change::CoversInList(on) => {
                self.settings.covers_in_list = on;
                self.save_setting(COVERS_IN_LIST, flag(on));
                self.queue_list_covers()
            }
            Change::Precache(p) => {
                self.settings.precache = p;
                self.save_setting(PRECACHE, p.key());
                self.precache_dirty = true;
                Task::none()
            }
            Change::Autoplay(on) => {
                self.settings.autoplay = on;
                self.save_setting(AUTOPLAY, flag(on));
                Task::none()
            }
            Change::Scale(dir) => {
                self.settings.ui_scale = scaled(self.settings.ui_scale, dir);
                self.save_setting(UI_SCALE, &self.settings.ui_scale.to_string());
                Task::none()
            }
        }
    }
}

/// Un paso más grande (`dir` > 0), más chico (< 0) o el normal (0).
pub fn scaled(current: u32, dir: i32) -> u32 {
    match dir.signum() {
        0 => 100,
        1 => (current + SCALE_STEP).min(SCALE_MAX),
        _ => current.saturating_sub(SCALE_STEP).max(SCALE_MIN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_steps_stay_in_range() {
        assert_eq!(scaled(100, 1), 110);
        assert_eq!(scaled(100, -1), 90);
        assert_eq!(scaled(SCALE_MAX, 1), SCALE_MAX);
        assert_eq!(scaled(SCALE_MIN, -1), SCALE_MIN);
        assert_eq!(scaled(130, 0), 100);
    }

    #[test]
    fn precache_keys_round_trip() {
        for p in Precache::ALL {
            assert_eq!(Precache::from_key(p.key()), p);
        }
        assert_eq!(Precache::from_key("raro"), Precache::Off);
    }
}
