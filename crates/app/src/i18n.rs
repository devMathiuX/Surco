//! Idioma de la interfaz: español o inglés.
//!
//! El idioma activo vive en un global (como el tema): los textos se eligen con
//! [`tr!`] al armar la vista. Por defecto sale del idioma del sistema:
//! español si es `es-*`, inglés si no.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Es, Lang::En];

    /// Clave que se guarda en los ajustes.
    pub fn key(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::En => "en",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        match s {
            "es" => Some(Lang::Es),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// Nombre en su propio idioma.
    pub fn name(self) -> &'static str {
        match self {
            Lang::Es => "Español",
            Lang::En => "English",
        }
    }

    /// Idioma del sistema (`es-AR`, `es_ES.UTF-8`… → español; otro → inglés).
    pub fn system() -> Self {
        Self::from_locale(sys_locale::get_locale().as_deref().unwrap_or(""))
    }

    fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("es") {
            Lang::Es
        } else {
            Lang::En
        }
    }
}

pub fn set(lang: Lang) {
    ENGLISH.store(lang == Lang::En, Ordering::Relaxed);
}

/// Si la interfaz está en inglés.
pub fn en() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Elige el texto según el idioma: `tr!("Hola", "Hello")`.
#[macro_export]
macro_rules! tr {
    ($es:expr, $en:expr $(,)?) => {
        if $crate::i18n::en() {
            $en
        } else {
            $es
        }
    };
}

/// Como [`tr!`] pero con formato: `trf!("Hola {name}", "Hello {name}")`.
#[macro_export]
macro_rules! trf {
    ($es:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::en() {
            format!($en $(, $arg)*)
        } else {
            format!($es $(, $arg)*)
        }
    };
}

/// "1 canción", "3 canciones" / "1 song", "3 songs".
pub fn songs(n: usize) -> String {
    match (en(), n) {
        (false, 1) => "1 canción".to_owned(),
        (false, n) => format!("{n} canciones"),
        (true, 1) => "1 song".to_owned(),
        (true, n) => format!("{n} songs"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_picks_spanish_only_for_es() {
        assert_eq!(Lang::from_locale("es-AR"), Lang::Es);
        assert_eq!(Lang::from_locale("es_ES.UTF-8"), Lang::Es);
        assert_eq!(Lang::from_locale("ES"), Lang::Es);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale("pt-BR"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
        for l in Lang::ALL {
            assert_eq!(Lang::from_key(l.key()), Some(l));
        }
    }
}
