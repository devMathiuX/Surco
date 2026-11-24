//! Importación de playlists de Spotify (Web API, página embed, CSV). Se implementa en la Fase 3.

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("todavía no implementado")]
    NotImplemented,
}
