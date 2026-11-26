//! Importación de playlists de Spotify.
//!
//! Estrategias, probadas en cascada por [`Importer`]:
//! 1. [`WebPlayer`]: como open.spotify.com sin sesión (token anónimo y la API
//!    interna del reproductor web). Playlist entera, sin claves ni Premium.
//! 2. [`Embed`]: página embed pública, sin credenciales (hasta 100 canciones).
//! 3. CSV de Exportify ([`import_csv_file`]), que no depende de la red.
//!
//! Todo se normaliza a [`surco_core::Track`].

pub use surco_core as core;

mod csv;
mod embed;
mod link;
mod webplayer;

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use surco_core::Playlist;

pub use csv::{import_csv, import_csv_file};
pub use embed::{parse_embed_html, Embed, EMBED_LIMIT};
pub use link::parse_playlist_id;
pub use webplayer::{parse_token, WebPlayer};

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no es un link de playlist de Spotify")]
    InvalidLink,
    #[error("la playlist no existe o es privada")]
    NotFound,
    #[error("Spotify no devolvió las canciones de esta playlist")]
    NoTracks,
    #[error("Spotify limitó las consultas (reintentar en {0:?})")]
    RateLimited(Duration),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("formato inesperado: {0}")]
    Parse(String),
    #[error("red: {0}")]
    Http(#[from] reqwest::Error),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("{}", format_all(.0))]
    AllFailed(Vec<(&'static str, Error)>),
}

fn format_all(errors: &[(&'static str, Error)]) -> String {
    let parts: Vec<String> = errors.iter().map(|(s, e)| format!("{s}: {e}")).collect();
    format!("no se pudo importar ({})", parts.join("; "))
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Resultado de una importación.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub playlist: Playlist,
    /// Qué estrategia funcionó (para mostrar y loguear).
    pub source: &'static str,
    /// Avisos para el usuario (ej. playlist recortada a 100 canciones).
    pub warnings: Vec<String>,
}

/// Una forma de obtener una playlist de Spotify a partir de su ID.
///
/// Devuelve un future en caja para poder guardar fuentes distintas en un
/// `Vec<Box<dyn PlaylistSource>>`.
pub trait PlaylistSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn fetch<'a>(
        &'a self,
        id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Imported>> + Send + 'a>>;
}

/// Prueba cada fuente en orden hasta que una funcione.
pub struct Importer {
    sources: Vec<Box<dyn PlaylistSource>>,
}

impl std::fmt::Debug for Importer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<_> = self.sources.iter().map(|s| s.name()).collect();
        f.debug_struct("Importer").field("sources", &names).finish()
    }
}

impl Importer {
    pub fn new(sources: Vec<Box<dyn PlaylistSource>>) -> Self {
        Self { sources }
    }

    /// Cascada estándar: reproductor web sin sesión y después embed.
    pub fn standard(client: reqwest::Client) -> Self {
        Self::new(vec![
            Box::new(WebPlayer::new(client.clone())),
            Box::new(Embed::new(client)),
        ])
    }

    /// Importa desde un link, URI o ID de playlist.
    pub async fn import_link(&self, input: &str) -> Result<Imported> {
        let id = parse_playlist_id(input).ok_or(Error::InvalidLink)?;
        self.import_id(&id).await
    }

    pub async fn import_id(&self, id: &str) -> Result<Imported> {
        let mut errors = Vec::new();
        for source in &self.sources {
            match source.fetch(id).await {
                Ok(imported) => {
                    tracing::info!(
                        source = source.name(),
                        tracks = imported.playlist.tracks.len(),
                        "playlist importada"
                    );
                    return Ok(imported);
                }
                // Si la playlist no existe, no tiene sentido seguir probando.
                Err(Error::NotFound) => return Err(Error::NotFound),
                Err(e) => {
                    tracing::warn!(source = source.name(), error = %e, "falló una estrategia");
                    errors.push((source.name(), e));
                }
            }
        }
        Err(Error::AllFailed(errors))
    }
}
