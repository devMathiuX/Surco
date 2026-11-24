//! Persistencia en SQLite: caché de matches, biblioteca, historial. Se implementa en la Fase 6.

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("todavía no implementado")]
    NotImplemented,
}
