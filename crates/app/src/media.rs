//! Integración con el sistema: MPRIS en Linux y SMTC en Windows (vía
//! `souvlaki`). Teclas multimedia, widget del sistema y "qué está sonando".

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use iced::futures::Stream;
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};
use tokio::sync::mpsc;

use crate::Message;

/// Eventos del sistema, hasta que la suscripción los toma.
static EVENTS: OnceLock<Mutex<Option<mpsc::UnboundedReceiver<MediaControlEvent>>>> =
    OnceLock::new();

/// Lo que se le muestra al sistema.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub cover_url: Option<String>,
}

pub struct Media {
    controls: MediaControls,
    last_meta: Option<NowPlaying>,
    last_playback: Option<(bool, bool)>,
}

impl std::fmt::Debug for Media {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Media").finish_non_exhaustive()
    }
}

impl Media {
    /// `hwnd` es obligatorio en Windows e ignorado en el resto.
    pub fn new(hwnd: Option<isize>) -> Option<Self> {
        if !session_bus_available() {
            tracing::warn!("no hay bus de sesión de D-Bus: sin controles multimedia del sistema");
            return None;
        }
        let config = PlatformConfig {
            display_name: "Surco",
            dbus_name: "surco",
            hwnd: hwnd.map(|h| h as *mut std::ffi::c_void),
        };
        let mut controls = match MediaControls::new(config) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = ?e, "sin controles multimedia del sistema");
                return None;
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        if let Err(e) = controls.attach(move |event| {
            let _ = tx.send(event);
        }) {
            tracing::warn!(error = ?e, "no se pudieron conectar los controles multimedia");
            return None;
        }
        let _ = EVENTS.set(Mutex::new(Some(rx)));
        tracing::info!("controles multimedia del sistema activos");
        Some(Self {
            controls,
            last_meta: None,
            last_playback: None,
        })
    }

    /// Actualiza metadatos y estado (solo si cambiaron).
    pub fn update(&mut self, now: Option<&NowPlaying>, playing: bool, position: Duration) {
        if let Some(meta) = now {
            if self.last_meta.as_ref() != Some(meta) {
                let res = self.controls.set_metadata(MediaMetadata {
                    title: Some(&meta.title),
                    artist: Some(&meta.artist),
                    album: meta.album.as_deref(),
                    cover_url: meta.cover_url.as_deref(),
                    duration: meta.duration,
                });
                if let Err(e) = res {
                    tracing::debug!(error = ?e, "set_metadata");
                }
                self.last_meta = Some(meta.clone());
                self.last_playback = None;
            }
        }
        let key = (now.is_some(), playing);
        if self.last_playback == Some(key) {
            return;
        }
        self.last_playback = Some(key);
        let progress = Some(MediaPosition(position));
        let playback = match (now.is_some(), playing) {
            (false, _) => MediaPlayback::Stopped,
            (true, true) => MediaPlayback::Playing { progress },
            (true, false) => MediaPlayback::Paused { progress },
        };
        if let Err(e) = self.controls.set_playback(playback) {
            tracing::debug!(error = ?e, "set_playback");
        }
    }

    /// Después de un seek, para que el sistema muestre la posición nueva.
    pub fn force_refresh(&mut self) {
        self.last_playback = None;
    }
}

/// Suscripción con los eventos del sistema traducidos a mensajes.
pub fn events() -> impl Stream<Item = Message> {
    let rx = EVENTS.get().and_then(|m| m.lock().ok()?.take());
    iced::futures::stream::unfold(rx, |rx| async move {
        let mut rx = rx?;
        loop {
            let event = rx.recv().await?;
            if let Some(msg) = to_message(event) {
                return Some((msg, Some(rx)));
            }
        }
    })
}

fn to_message(event: MediaControlEvent) -> Option<Message> {
    const STEP: Duration = Duration::from_secs(5);
    Some(match event {
        MediaControlEvent::Play => Message::Play,
        MediaControlEvent::Pause | MediaControlEvent::Stop => Message::Pause,
        MediaControlEvent::Toggle => Message::TogglePause,
        MediaControlEvent::Next => Message::Skip(1),
        MediaControlEvent::Previous => Message::Skip(-1),
        MediaControlEvent::Seek(dir) => Message::SeekRelative(signed(dir, STEP)),
        MediaControlEvent::SeekBy(dir, d) => Message::SeekRelative(signed(dir, d)),
        MediaControlEvent::SetPosition(MediaPosition(d)) => Message::SeekTo(d),
        MediaControlEvent::SetVolume(v) => Message::Volume((v as f32).clamp(0.0, 1.0)),
        _ => return None,
    })
}

fn signed(dir: SeekDirection, d: Duration) -> f32 {
    match dir {
        SeekDirection::Forward => d.as_secs_f32(),
        SeekDirection::Backward => -d.as_secs_f32(),
    }
}

/// ¿Hay un bus de sesión de D-Bus al que conectarse? (solo importa en Linux)
///
/// `souvlaki` conecta el bus en un hilo propio y hace `unwrap()` si falla:
/// sin bus (sesión sin D-Bus, `ssh`, algunos gestores de ventanas) eso
/// tumbaba la app entera. Así que lo comprobamos antes de crear los controles.
#[cfg(all(unix, not(target_os = "macos")))]
fn session_bus_available() -> bool {
    session_bus_path(
        std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from),
    )
    // Conectar de verdad: un socket que quedó de una sesión vieja existe pero rechaza.
    .is_none_or(|p| std::os::unix::net::UnixStream::connect(p).is_ok())
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn session_bus_available() -> bool {
    true
}

/// Socket del bus de sesión que se va a usar, si es un archivo que se puede
/// comprobar. `None` = no se puede saber (p. ej. socket abstracto o TCP):
/// se intenta igual.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn session_bus_path(
    address: Option<&str>,
    runtime_dir: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    match address {
        Some(addr) => addr.split(';').find_map(|a| {
            let rest = a.strip_prefix("unix:")?;
            rest.split(',')
                .find_map(|kv| kv.strip_prefix("path="))
                .map(std::path::PathBuf::from)
        }),
        // Sin variable, zbus usa $XDG_RUNTIME_DIR/bus (o /run/user/<uid>/bus).
        None => Some(
            runtime_dir
                .unwrap_or_else(|| std::path::PathBuf::from("/nonexistent"))
                .join("bus"),
        ),
    }
}

/// HWND de la ventana (solo Windows), para SMTC.
pub fn hwnd_of(window: &dyn iced::window::Window) -> Option<isize> {
    #[cfg(windows)]
    {
        use iced::window::raw_window_handle::RawWindowHandle;
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                return Some(w.hwnd.get());
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn bus_path_from_address_or_runtime_dir() {
        assert_eq!(
            session_bus_path(Some("unix:path=/run/user/1000/bus"), None),
            Some(PathBuf::from("/run/user/1000/bus"))
        );
        assert_eq!(
            session_bus_path(Some("unix:path=/tmp/x,guid=abc;tcp:host=h"), None),
            Some(PathBuf::from("/tmp/x"))
        );
        // Socket abstracto: no se puede comprobar, se intenta.
        assert_eq!(
            session_bus_path(Some("unix:abstract=/tmp/dbus-1"), None),
            None
        );
        assert_eq!(
            session_bus_path(None, Some(PathBuf::from("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000/bus"))
        );
    }
}
