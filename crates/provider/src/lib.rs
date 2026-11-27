//! Obtención de audio desde YouTube: wrapper de `yt-dlp`, caché de URLs de
//! stream y lector HTTP con soporte de `Range` para reproducir en streaming.

pub use surco_core as core;

mod audio_cache;
mod deno;
mod http_stream;
pub mod innertube;
pub mod update;
mod ytdlp;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use audio_cache::{is_video_id, AudioCache, CacheUsage, DEFAULT_LIMIT as AUDIO_CACHE_LIMIT};
/// Cliente HTTP para bajar el audio de YouTube.
///
/// Va siempre por IPv4, igual que yt-dlp (`--force-ipv4`): las URLs de
/// googlevideo quedan atadas a la IP que las pidió (`ip=` en la URL). Con
/// IPv6, Windows usa direcciones temporales que cambian, y si yt-dlp y la
/// descarga salen por direcciones distintas YouTube contesta 403.
pub fn stream_client() -> HttpClient {
    HttpClient::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .build()
        .unwrap_or_default()
}

pub use deno::{download_deno, refresh_deno, set_tools_dir, DENO_BINARY};
pub use http_stream::{download_all, download_all_with, HttpStream, StreamProgress};
pub use reqwest::Client as HttpClient;
pub use ytdlp::{js_runtime, Listing, SearchResult, StreamInfo, YtDlp, AUDIO_FORMAT, BINARY_NAME};

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

/// Cliente de YouTube que no necesita resolver el desafío de JavaScript
/// (el mismo que usa yt-dlp cuando no hay deno). Pedir la URL con él tarda
/// bastante menos; si falla o YouTube rechaza esa URL, se usa el normal.
pub const FAST_CLIENT: &str = "visionos";

/// Tiempo máximo del intento rápido antes de pasar al normal.
const FAST_TIMEOUT: Duration = Duration::from_secs(15);

/// Fallos del cliente rápido tras los cuales se deja de usar en la sesión.
const FAST_MAX_FAILURES: u32 = 3;

/// De dónde salió una URL de stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Direct,
    Fast,
    Normal,
}

/// Cliente para pedir la URL directo: por IPv4, como la descarga.
fn direct_client() -> &'static HttpClient {
    static CLIENT: std::sync::OnceLock<HttpClient> = std::sync::OnceLock::new();
    CLIENT.get_or_init(stream_client)
}

/// Punto de entrada: yt-dlp + caché en memoria de URLs de stream.
#[derive(Debug)]
pub struct Provider {
    ytdlp: YtDlp,
    cache: Mutex<HashMap<String, (StreamInfo, Instant)>>,
    ttl: Duration,
    audio: Option<AudioCache>,
    /// Videos cuya URL en caché salió del cliente rápido.
    fast_urls: Mutex<HashSet<String>>,
    /// Fallos del cliente rápido en esta sesión (ver [`FAST_MAX_FAILURES`]).
    fast_failures: AtomicU32,
    /// Pedir la URL directo a YouTube, sin yt-dlp (ver [`innertube::player`]).
    direct: bool,
    /// Videos cuya URL en caché salió del pedido directo.
    direct_urls: Mutex<HashSet<String>>,
    /// Fallos del pedido directo en esta sesión.
    direct_failures: AtomicU32,
    /// De dónde salió la última URL rechazada de cada video (ver
    /// [`invalidate`](Self::invalidate)).
    rejected: Mutex<HashMap<String, Source>>,
}

impl Provider {
    pub fn new(ytdlp: YtDlp) -> Self {
        Self {
            ytdlp,
            cache: Mutex::new(HashMap::new()),
            ttl: STREAM_URL_TTL,
            audio: None,
            fast_urls: Mutex::new(HashSet::new()),
            fast_failures: AtomicU32::new(0),
            direct: false,
            direct_urls: Mutex::new(HashSet::new()),
            direct_failures: AtomicU32::new(0),
            rejected: Mutex::new(HashMap::new()),
        }
    }

    /// Pide primero la URL del audio directo a YouTube (sin arrancar
    /// yt-dlp); si falla, sigue con yt-dlp como siempre.
    pub fn with_direct_player(mut self) -> Self {
        self.direct = true;
        self
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
            match http_stream::download_all_with(client, &info.url, info.user_agent.as_deref())
                .await
            {
                Ok(data) => {
                    let path = cache.put(video, &data)?;
                    tracing::info!(video, bytes = data.len(), "audio precargado");
                    return Ok(path);
                }
                Err(Error::Expired) if attempt == 0 => {
                    self.invalidate(video);
                    refresh = true;
                }
                Err(Error::Expired) => break,
                Err(e) => return Err(e),
            }
        }
        self.download_with_ytdlp(video).await
    }

    /// Plan B ante un 403: que yt-dlp baje el audio y guardarlo en la caché.
    pub async fn download_with_ytdlp(&self, video: &str) -> Result<PathBuf> {
        let cache = self
            .audio
            .as_ref()
            .ok_or_else(|| Error::Parse("no hay caché de audio".into()))?;
        tracing::info!(video, "YouTube rechazó la URL; la baja yt-dlp");
        let tmp = self.ytdlp.download_audio(video, cache.dir()).await?;
        let data = tokio::fs::read(&tmp).await;
        let _ = tokio::fs::remove_file(&tmp).await;
        let path = cache.put(video, &data?)?;
        tracing::info!(video, "audio bajado por yt-dlp");
        Ok(path)
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
        // Después de un rechazo (`refresh`) no se repite lo que falló: si
        // YouTube rechazó la URL directa se prueba el cliente rápido de
        // yt-dlp, y si rechazó ese, el normal.
        let rejected = if refresh {
            self.rejected
                .lock()
                .ok()
                .and_then(|r| r.get(video).copied())
                .or(Some(Source::Normal))
        } else {
            None
        };
        let try_fast = matches!(rejected, None | Some(Source::Direct));
        if rejected.is_none() && self.direct_enabled() {
            match innertube::player(direct_client(), video).await {
                Ok(info) => {
                    tracing::info!(video, "URL del audio pedida directo a YouTube");
                    self.remember(video, &info, Source::Direct);
                    return Ok(info);
                }
                Err(e) => {
                    tracing::warn!(video, error = %e, "falló el pedido directo del audio; se usa yt-dlp");
                    self.direct_failed();
                }
            }
        }
        if try_fast && self.fast_enabled() {
            match self
                .ytdlp
                .stream_info_with(video, Some(FAST_CLIENT), FAST_TIMEOUT)
                .await
            {
                Ok(info) if info.is_supported() => {
                    tracing::info!(video, "URL del audio con yt-dlp (cliente rápido)");
                    self.remember(video, &info, Source::Fast);
                    return Ok(info);
                }
                // Sin yt-dlp no tiene sentido reintentar.
                Err(e @ Error::Spawn { .. }) => return Err(e),
                Ok(info) => {
                    tracing::warn!(video, ext = %info.ext, "el cliente rápido dio un formato no soportado");
                    self.fast_failed();
                }
                Err(e) => {
                    tracing::warn!(video, error = %e, "falló el cliente rápido; se usa el normal");
                    self.fast_failed();
                }
            }
        }
        let info = self.ytdlp.stream_info(video).await?;
        tracing::info!(video, "URL del audio con yt-dlp (cliente normal)");
        self.remember(video, &info, Source::Normal);
        Ok(info)
    }

    fn remember(&self, video: &str, info: &StreamInfo, source: Source) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(video.to_owned(), (info.clone(), Instant::now()));
        }
        for (set, mine) in [
            (&self.fast_urls, source == Source::Fast),
            (&self.direct_urls, source == Source::Direct),
        ] {
            if let Ok(mut set) = set.lock() {
                if mine {
                    set.insert(video.to_owned());
                } else {
                    set.remove(video);
                }
            }
        }
    }

    fn direct_enabled(&self) -> bool {
        self.direct && self.direct_failures.load(Ordering::Relaxed) < FAST_MAX_FAILURES
    }

    fn direct_failed(&self) {
        let n = self.direct_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if n == FAST_MAX_FAILURES {
            tracing::warn!("el pedido directo del audio falló varias veces; se deja de usar");
        }
    }

    fn fast_enabled(&self) -> bool {
        self.fast_failures.load(Ordering::Relaxed) < FAST_MAX_FAILURES
    }

    fn fast_failed(&self) {
        let n = self.fast_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if n == FAST_MAX_FAILURES {
            tracing::warn!("el cliente rápido de YouTube falló varias veces; se deja de usar");
        }
    }

    /// Hay una URL vigente de `video` en la caché.
    pub fn has_url(&self, video: &str) -> bool {
        self.cached(video).is_some()
    }

    /// Olvida todas las URLs cacheadas (por ejemplo, al instalar deno).
    pub fn forget_urls(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
        for set in [&self.fast_urls, &self.direct_urls] {
            if let Ok(mut set) = set.lock() {
                set.clear();
            }
        }
    }

    /// Olvida la URL cacheada de `video` (por ejemplo, después de un 403).
    pub fn invalidate(&self, video: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(video);
        }
        // YouTube rechazó una URL del cliente rápido o directa: cuenta como fallo.
        let mut source = Source::Normal;
        if self.fast_urls.lock().is_ok_and(|mut s| s.remove(video)) {
            self.fast_failed();
            source = Source::Fast;
        }
        if self.direct_urls.lock().is_ok_and(|mut s| s.remove(video)) {
            self.direct_failed();
            source = Source::Direct;
        }
        if let Ok(mut r) = self.rejected.lock() {
            r.insert(video.to_owned(), source);
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
            user_agent: None,
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

    /// yt-dlp falso que falla si le piden el cliente rápido y anota cada llamada.
    #[cfg(unix)]
    fn fake_ytdlp(name: &str, fast_works: bool) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("surco-fast-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls");
        let _ = std::fs::remove_file(&log);
        let script = dir.join("yt-dlp");
        let fast = if fast_works { ":" } else { "exit 1" };
        let body = format!(
            "#!/bin/sh\ncase \"$*\" in *player_client=*) echo fast >> '{l}'; {fast};; *) echo normal >> '{l}';; esac\nprintf 'm4a\\n1\\nt\\nhttps://example.com/%s\\n' \"$1\"\n",
            l = log.display()
        );
        // El archivo lo escribe otro proceso: si lo abriera este, otro test
        // que lanza un proceso justo en ese momento heredaría el archivo
        // abierto, y ejecutarlo fallaría con "Text file busy" (ETXTBSY).
        let ok = std::process::Command::new("sh")
            .args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"", "sh"])
            .arg(&body)
            .arg(&script)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        (script, log)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejected_direct_url_retries_with_the_fast_client() {
        let (script, log) = fake_ytdlp("direct", true);
        let p = Provider::new(YtDlp::new(&script));
        // Una URL directa que YouTube rechazó (403).
        p.remember("aqz-KE-bpKQ", &info(), Source::Direct);
        p.invalidate("aqz-KE-bpKQ");
        p.stream_info("aqz-KE-bpKQ", true).await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["fast"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fast_client_first_then_normal_after_rejection() {
        let (script, log) = fake_ytdlp("ok", true);
        let p = Provider::new(YtDlp::new(&script));
        p.stream_info("aqz-KE-bpKQ", false).await.unwrap();
        // Una URL rápida rechazada: el refresco va por el cliente normal.
        p.invalidate("aqz-KE-bpKQ");
        p.stream_info("aqz-KE-bpKQ", true).await.unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["fast", "normal"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fast_client_failures_fall_back_and_turn_it_off() {
        let (script, log) = fake_ytdlp("fail", false);
        let p = Provider::new(YtDlp::new(&script));
        for i in 0..5 {
            p.stream_info(&format!("video{i}"), false).await.unwrap();
        }
        let calls = std::fs::read_to_string(&log).unwrap();
        let fast = calls.lines().filter(|l| *l == "fast").count();
        let normal = calls.lines().filter(|l| *l == "normal").count();
        assert_eq!((fast, normal), (FAST_MAX_FAILURES as usize, 5));
    }

    #[tokio::test]
    async fn missing_binary_is_an_error() {
        let p = provider_with(Duration::from_secs(60));
        let err = p.stream_info("abc", false).await.unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }), "{err}");
    }
}
