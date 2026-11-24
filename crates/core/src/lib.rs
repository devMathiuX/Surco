//! Modelos y tipos compartidos por todos los crates de Surco.
//!
//! Este crate no hace I/O: solo define datos y contratos.

use serde::{Deserialize, Serialize};

/// Una canción tal como la describe la fuente de la playlist (Spotify, CSV…).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Track {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub isrc: Option<String>,
    /// ID de Spotify, si la canción viene de Spotify.
    pub spotify_id: Option<String>,
}

impl Track {
    /// Artista principal (el primero), o cadena vacía si no hay.
    pub fn main_artist(&self) -> &str {
        self.artists.first().map(String::as_str).unwrap_or_default()
    }
}

/// Una playlist importada.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    /// Identificador estable (ID de Spotify o hash del CSV).
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

/// Estado de la resolución Spotify -> YouTube de un elemento de la cola.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Resolution {
    #[default]
    Pending,
    Resolved {
        youtube_id: String,
        /// Puntaje 0-100 del match.
        score: u8,
    },
    Failed(String),
}

impl Resolution {
    /// Umbral por debajo del cual un match se considera dudoso.
    pub const DOUBTFUL_THRESHOLD: u8 = 60;

    pub fn is_doubtful(&self) -> bool {
        matches!(self, Resolution::Resolved { score, .. } if *score < Self::DOUBTFUL_THRESHOLD)
    }
}

/// Un elemento de la cola de reproducción.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueItem {
    pub track: Track,
    pub resolution: Resolution,
}

impl From<Track> for QueueItem {
    fn from(track: Track) -> Self {
        Self {
            track,
            resolution: Resolution::Pending,
        }
    }
}

/// Estados del reproductor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PlayerState {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Error(String),
}

/// Modo de repetición.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

/// Formatea una duración en milisegundos como `m:ss` (o `h:mm:ss`).
pub fn format_duration(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artists: &[&str]) -> Track {
        Track {
            title: "Song".into(),
            artists: artists.iter().map(|a| a.to_string()).collect(),
            album: None,
            duration_ms: 0,
            isrc: None,
            spotify_id: None,
        }
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(65_000), "1:05");
        assert_eq!(format_duration(3_725_999), "1:02:05");
    }

    #[test]
    fn main_artist() {
        assert_eq!(track(&["A", "B"]).main_artist(), "A");
        assert_eq!(track(&[]).main_artist(), "");
    }

    #[test]
    fn doubtful_threshold() {
        let r = |score| Resolution::Resolved {
            youtube_id: "x".into(),
            score,
        };
        assert!(r(59).is_doubtful());
        assert!(!r(60).is_doubtful());
        assert!(!Resolution::Pending.is_doubtful());
    }
}
