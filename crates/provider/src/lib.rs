//! Obtención de audio desde YouTube: wrapper de `yt-dlp`, caché de URLs de
//! stream y lector HTTP con soporte de `Range` para reproducir en streaming.

pub use surco_core as core;

mod audio_cache;
mod http_stream;
mod ytdlp;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use audio_cache::{is_video_id, AudioCache, CacheUsage, DEFAULT_LIMIT as AUDIO_CACHE_LIMIT};
pub use http_stream::{download_all, HttpStream, StreamProgress};
pub use reqwest::Client as HttpClient;
pub use ytdlp::{SearchResult, StreamInfo, YtDlp, AUDIO_FORMAT, BINARY_NAME};

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no se encontró yt-dlp")]
    YtDlpMissing,
    #[error("no se pudo ejecutar {path}: {source}")]
    Spawn {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("yt-dlp: {0}")]
    YtDlp(String),
    /// YouTube pidió verificar que no somos un bot (IP marcada, VPN, datacenter).
    #[error("YouTube bloqueó la consulta desde esta red (pide verificar que no sos un bot)")]
    BotCheck(String),
    #[error("tiempo de espera agotado ({0:?})")]
    Timeout(Duration),
    #[error("respuesta inesperada: {0}")]
    Parse(String),
    /// La URL de stream venció o fue rechazada (HTTP 403): hay que volver a resolverla.
    #[error("la URL del stream venció (HTTP 403)")]
    Expired,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("red: {0}")]
    Http(#[from] reqwest::Error),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Las URLs de googlevideo vencen a las ~6 h; nos quedamos con 5 h de margen.
pub const STREAM_URL_TTL: Duration = Duration::from_secs(5 * 60 * 60);

/// Punto de entrada: yt-dlp + caché en memoria de URLs de stream.
#[derive(Debug)]
pub struct Provider {
    ytdlp: YtDlp,
    cache: Mutex<HashMap<String, (StreamInfo, Instant)>>,
    ttl: Duration,
    audio: Option<AudioCache>,
}

impl Provider {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            ytdlp,
            cache: Mutex::new(HashMap::new()),
            ttl: STREAM_URL_TTL,
            audio: None,
        }
    }

    /// Activa la caché de audio en disco (precarga).
    pub fn with_audio_cache(mut self, cache: AudioCache) -> Self {
        self.audio = Some(cache);
        self
    }

    pub fn audio_cache(&self) -> Option<&AudioCache> {
        self.audio.as_ref()
    }

    /// Audio de `video` ya bajado a disco, si está.
    pub fn cached_audio(&self, video: &str) -> Option<PathBuf> {
        self.audio.as_ref()?.get(video)
    }

    /// Baja el audio de `video` a la caché de disco (si no estaba).
    pub async fn precache(&self, client: &reqwest::Client, video: &str) -> Result<PathBuf> {
        let cache = self
            .audio
            .as_ref()
            .ok_or_else(|| Error::Parse("no hay caché de audio".into()))?;
        if let Some(p) = cache.get(video) {
            return Ok(p);
        }
        let mut refresh = false;
        for attempt in 0..2 {
            let info = self.stream_info(video, refresh).await?;
            if !info.is_supported() {
                return Err(Error::Parse(format!("formato {} no soportado", info.ext)));
            }
            match http_stream::download_all(client, &info.url).await {
                Ok(data) => {
                    let path = cache.put(video, &data)?;
                    tracing::info!(video, bytes = data.len(), "audio precargado");
                    return Ok(path);
                }
                Err(Error::Expired) if attempt == 0 => {
                    self.invalidate(video);
                    refresh = true;
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::Expired)
    }

    pub fn ytdlp(&self) -> &YtDlp {
        &self.ytdlp
    }

    /// URL de audio para `video` (ID o URL de YouTube). Usa la caché salvo
    /// que `refresh` sea `true` o la entrada haya vencido.
    pub async fn stream_info(&self, video: &str, refresh: bool) -> Result<StreamInfo> {
        if !refresh {
            if let Some(info) = self.cached(video) {
                tracing::debug!(video, "URL de stream desde caché");
                return Ok(info);
            }
        }
        let info = self.ytdlp.stream_info(video).await?;
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(video.to_owned(), (info.clone(), Instant::now()));
        }
        Ok(info)
    }

    /// Olvida la URL cacheada de `video` (por ejemplo, después de un 403).
    pub fn invalidate(&self, video: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(video);
        }
    }

    fn cached(&self, video: &str) -> Option<StreamInfo> {
        let mut cache = self.cache.lock().ok()?;
        match cache.get(video) {
            Some((info, at)) if at.elapsed() < self.ttl => Some(info.clone()),
            Some(_) => {
                cache.remove(video);
                None
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_with(ttl: Duration) -> Provider {
        let mut p = Provider::new(YtDlp::new("/nonexistent/yt-dlp"));
        p.ttl = ttl;
        p
    }

    fn info() -> StreamInfo {
        StreamInfo {
            url: "https://example.com/a".into(),
            ext: "m4a".into(),
            title: None,
            duration: None,
        }
    }

    #[test]
    fn cache_hit_and_expiry() {
        let p = provider_with(Duration::from_secs(60));
        p.cache
            .lock()
            .unwrap()
            .insert("v".into(), (info(), Instant::now()));
        assert_eq!(p.cached("v"), Some(info()));
        p.invalidate("v");
        assert_eq!(p.cached("v"), None);

        let p = provider_with(Duration::ZERO);
        p.cache
            .lock()
            .unwrap()
            .insert("v".into(), (info(), Instant::now()));
        assert_eq!(p.cached("v"), None);
    }

    #[tokio::test]
    async fn missing_binary_is_an_error() {
        let p = provider_with(Duration::from_secs(60));
        let err = p.stream_info("abc", false).await.unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }), "{err}");
    }
}
