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
        let offline = get(crate::offline::OFFLINE);
        self.load_offline(offline);
        self.settings = s;
        self.custom_hex = [theme::to_hex(s.custom.0), theme::to_hex(s.custom.1)];
        i18n::set(s.language);
        self.theme = theme::set(s.theme, s.custom);
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precache_keys_round_trip() {
        for p in Precache::ALL {
            assert_eq!(Precache::from_key(p.key()), p);
        }
        assert_eq!(Precache::from_key("raro"), Precache::Off);
    }
}
