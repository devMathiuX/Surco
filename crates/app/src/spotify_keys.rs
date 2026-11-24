//! Claves de la Web API de Spotify (Client Credentials). Con claves, una
//! playlist se importa entera (la página pública muestra hasta 100).
//!
//! Orden: las que el usuario cargó en Ajustes, las de `config.toml`, y las
//! que trae el build oficial (secretos del repo, inyectados con
//! `SURCO_SPOTIFY_CLIENT_ID` / `SURCO_SPOTIFY_CLIENT_SECRET` al compilar).
//! Si la API falla (403, 404, 429…), la importación sigue con la página
//! pública o el CSV.

use surco_spotify::Credentials;

use crate::{Message, Surco};

const ID: &str = "spotify_client_id";
const SECRET: &str = "spotify_client_secret";

/// De dónde salen las claves en uso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeySource {
    /// Cargadas en Ajustes.
    Own,
    /// `config.toml`.
    Config,
    /// Incluidas en Surco.
    Builtin,
    #[default]
    None,
}

/// Claves incluidas al compilar (vacío en builds locales).
pub fn builtin() -> Option<Credentials> {
    creds(
        option_env!("SURCO_SPOTIFY_CLIENT_ID")?,
        option_env!("SURCO_SPOTIFY_CLIENT_SECRET")?,
    )
}

fn creds(id: &str, secret: &str) -> Option<Credentials> {
    let (id, secret) = (id.trim(), secret.trim());
    (!id.is_empty() && !secret.is_empty()).then(|| Credentials {
        client_id: id.to_owned(),
        client_secret: secret.to_owned(),
    })
}

/// Edición de las claves en Ajustes.
#[derive(Debug, Clone)]
pub enum KeysMsg {
    Id(String),
    Secret(String),
    Save,
    Clear,
}

#[derive(Debug, Default)]
pub struct KeysForm {
    pub id: String,
    pub secret: String,
}

impl Surco {
    /// Elige qué claves usar (al arrancar y al cambiarlas).
    pub(crate) fn load_spotify_keys(&mut self) {
        let get = |k| {
            self.store
                .as_ref()
                .and_then(|s| s.setting(k).ok().flatten())
                .unwrap_or_default()
        };
        let own = creds(&get(ID), &get(SECRET));
        let (c, src) = if let Some(c) = own {
            (Some(c), KeySource::Own)
        } else if let Some(c) = crate::config::Config::load().spotify_credentials() {
            (Some(c), KeySource::Config)
        } else if let Some(c) = builtin() {
            (Some(c), KeySource::Builtin)
        } else {
            (None, KeySource::None)
        };
        self.spotify_creds = c;
        self.spotify_source = src;
    }

    pub(crate) fn on_keys(&mut self, msg: KeysMsg) {
        match msg {
            KeysMsg::Id(s) => self.keys_form.id = s,
            KeysMsg::Secret(s) => self.keys_form.secret = s,
            KeysMsg::Save => {
                if creds(&self.keys_form.id, &self.keys_form.secret).is_some() {
                    let form = std::mem::take(&mut self.keys_form);
                    self.save_keys(form.id.trim(), form.secret.trim());
                }
            }
            KeysMsg::Clear => self.save_keys("", ""),
        }
    }

    fn save_keys(&mut self, id: &str, secret: &str) {
        if let Some(store) = &self.store {
            for (k, v) in [(ID, id), (SECRET, secret)] {
                if let Err(e) = store.set_setting(k, v) {
                    tracing::warn!(error = %e, "guardar claves de Spotify");
                }
            }
        }
        self.load_spotify_keys();
    }
}

/// Para que `Message` lo envuelva.
pub fn msg(m: KeysMsg) -> Message {
    Message::Keys(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_both_parts() {
        assert!(creds(" id ", " secret ").is_some());
        assert!(creds("id", " ").is_none());
        assert!(creds("", "x").is_none());
    }
}
