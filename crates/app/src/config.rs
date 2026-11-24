//! Configuración en `config.toml`, en la carpeta de config del SO
//! (`~/.config/surco/` en Linux, `%APPDATA%\Surco\config\` en Windows).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use surco_spotify::Credentials;

const TEMPLATE: &str = r#"# Configuración de Surco.

[spotify]
# Opcional. Credenciales de una app de https://developer.spotify.com/dashboard
# (Client Credentials). Sin ellas, Surco lee la página pública de la playlist,
# que muestra hasta 100 canciones; para playlists más largas, importá un CSV
# de Exportify (https://exportify.net).
# Nota: desde 2026 Spotify puede no devolver las canciones de playlists ajenas
# a apps en "Development Mode"; en ese caso Surco sigue con la página pública.
client_id = ""
client_secret = ""
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub spotify: SpotifyConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SpotifyConfig {
    pub client_id: String,
    pub client_secret: String,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "Surco").map(|d| d.config_dir().join("config.toml"))
    }

    /// Lee la config. Si no existe, escribe una plantilla comentada.
    /// Nunca falla: ante un error, loguea y usa los valores por defecto.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::parse(&s).unwrap_or_else(|e| {
                tracing::warn!(path = %path.display(), error = %e, "config inválida, uso valores por defecto");
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                write_template(&path);
                Self::default()
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "no pude leer la config");
                Self::default()
            }
        }
    }

    fn parse(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Credenciales de Spotify, si están las dos.
    pub fn spotify_credentials(&self) -> Option<Credentials> {
        let (id, secret) = (
            self.spotify.client_id.trim(),
            self.spotify.client_secret.trim(),
        );
        (!id.is_empty() && !secret.is_empty()).then(|| Credentials {
            client_id: id.to_owned(),
            client_secret: secret.to_owned(),
        })
    }
}

fn write_template(path: &Path) {
    let res = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, TEMPLATE));
    if let Err(e) = res {
        tracing::warn!(path = %path.display(), error = %e, "no pude crear la config");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_without_credentials() {
        let c = Config::parse(TEMPLATE).unwrap();
        assert!(c.spotify_credentials().is_none());
    }

    #[test]
    fn reads_credentials() {
        let c = Config::parse("[spotify]\nclient_id = \"a\"\nclient_secret = \" b \"\n").unwrap();
        assert_eq!(c.spotify_credentials().unwrap().client_secret, "b");
        assert!(Config::parse("").unwrap().spotify_credentials().is_none());
    }
}
