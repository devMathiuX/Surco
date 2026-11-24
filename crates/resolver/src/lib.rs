//! Resolución Spotify -> YouTube (búsqueda + scoring). Se implementa en la Fase 4.

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("todavía no implementado")]
    NotImplemented,
}
