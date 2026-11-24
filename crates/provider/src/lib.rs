//! Wrapper de yt-dlp y obtención de URLs de stream. Se implementa en la Fase 2.

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("todavía no implementado")]
    NotImplemented,
}
