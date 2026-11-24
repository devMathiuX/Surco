//! Ajustes del usuario (pantalla de ajustes): tema, portadas en la lista,
//! precarga y radio automática. Se guardan en la tabla de ajustes de la
//! biblioteca.

use iced::Task;

use crate::{theme, Message, Surco};

const THEME: &str = "theme";
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
            Precache::Off => "No",
            Precache::Next => "Las próximas 5",
            Precache::Playlist => "Toda la playlist",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Índice en [`theme::THEMES`].
    pub theme: usize,
    pub covers_in_list: bool,
    pub precache: Precache,
    /// Al terminar la cola, seguir con canciones parecidas.
    pub autoplay: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: 0,
            covers_in_list: true,
            precache: Precache::Off,
            autoplay: true,
        }
    }
}

/// Cambios desde la pantalla de ajustes.
#[derive(Debug, Clone, Copy)]
pub enum Change {
    Theme(usize),
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
        if let Some(v) = get(COVERS_IN_LIST) {
            s.covers_in_list = v == "1";
        }
        if let Some(v) = get(PRECACHE) {
            s.precache = Precache::from_key(&v);
        }
        if let Some(v) = get(AUTOPLAY) {
            s.autoplay = v == "1";
        }
        self.settings = s;
        self.theme = theme::set(s.theme);
    }

    fn save_setting(&self, key: &str, value: &str) {
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
                self.settings.theme = i.min(theme::THEMES.len() - 1);
                self.theme = theme::set(self.settings.theme);
                self.save_setting(THEME, theme::THEMES[self.settings.theme].id);
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
