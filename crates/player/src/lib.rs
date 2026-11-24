//! Motor de audio, cola y estados. Se implementa en las Fases 2 y 5.

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("todavía no implementado")]
    NotImplemented,
}
