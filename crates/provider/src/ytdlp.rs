//! Wrapper del binario externo `yt-dlp`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::{Error, Result};

/// Nombre del binario según el SO.
#[cfg(windows)]
pub const BINARY_NAME: &str = "yt-dlp.exe";
#[cfg(not(windows))]
pub const BINARY_NAME: &str = "yt-dlp";

/// Asset de los releases oficiales que bajamos en cada SO.
/// En Linux y macOS usamos los binarios autocontenidos (no necesitan Python);
/// el de macOS es universal (Intel y Apple Silicon).
#[cfg(windows)]
const RELEASE_ASSET: &str = "yt-dlp.exe";
#[cfg(target_os = "macos")]
const RELEASE_ASSET: &str = "yt-dlp_macos";
#[cfg(not(any(windows, target_os = "macos")))]
const RELEASE_ASSET: &str = "yt-dlp_linux";

const RELEASES_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";

/// Versión "en carpeta" (onedir): el mismo yt-dlp, pero ya descomprimido.
/// El binario de un solo archivo se descomprime entero a una carpeta
/// temporal cada vez que arranca (~0,5 s más por búsqueda o canción).
#[cfg(windows)]
const ONEDIR_ASSET: &str = "yt-dlp_win.zip";
#[cfg(target_os = "macos")]
const ONEDIR_ASSET: &str = "yt-dlp_macos.zip";
#[cfg(not(any(windows, target_os = "macos")))]
const ONEDIR_ASSET: &str = "yt-dlp_linux.zip";

/// Ejecutable dentro del zip en carpeta.
#[cfg(windows)]
const ONEDIR_BINARY: &str = "yt-dlp.exe";
#[cfg(target_os = "macos")]
const ONEDIR_BINARY: &str = "yt-dlp_macos";
#[cfg(not(any(windows, target_os = "macos")))]
const ONEDIR_BINARY: &str = "yt-dlp_linux";

/// Carpeta (dentro de la de datos) con las versiones en carpeta. Cada
/// descarga va a una subcarpeta nueva: en Windows no se puede borrar un
/// yt-dlp que se está ejecutando, así que las viejas se borran al arrancar.
pub const ONEDIR_DIR: &str = "yt-dlp-onedir";

/// La versión en carpeta sale de las compilaciones diarias (nightly): traen
/// los arreglos para YouTube días antes que las versiones estables. Es lo
/// que recomienda yt-dlp para YouTube (y lo que usa Nuclear).
const NIGHTLY_RELEASES: &str = "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases";

/// Archivo con la versión (tag) dentro de cada carpeta de yt-dlp.
const TAG_FILE: &str = ".tag";

/// Si no se puede saber cuál es la última versión (sin red, GitHub caído),
/// se baja de nuevo la versión en carpeta cuando tiene más que esto.
pub const ONEDIR_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Formato pedido: m4a/AAC primero (symphonia lo decodifica nativo).
pub const AUDIO_FORMAT: &str = "bestaudio[ext=m4a]/bestaudio";

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(45);
/// Bajar una canción entera con yt-dlp (plan B ante un 403).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
/// Una playlist larga tarda: yt-dlp pide las páginas de a 100.
const PLAYLIST_TIMEOUT: Duration = Duration::from_secs(120);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Resultado de pedirle a yt-dlp la URL del audio de un video.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub url: String,
    /// Extensión del contenedor (`m4a`, `webm`…).
    pub ext: String,
    pub title: Option<String>,
    pub duration: Option<Duration>,
    /// `User-Agent` con el que yt-dlp pidió la URL: hay que mandar el mismo
    /// al bajarla, o YouTube puede contestar 403.
    pub user_agent: Option<String>,
}

impl StreamInfo {
    /// `true` si el formato lo podemos decodificar sin dependencias extra.
    pub fn is_supported(&self) -> bool {
        matches!(self.ext.as_str(), "m4a" | "mp4")
    }
}

/// Un binario de yt-dlp ubicado en disco.
#[derive(Debug, Clone)]
pub struct YtDlp {
    path: PathBuf,
}

impl YtDlp {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Busca yt-dlp primero en la carpeta de datos de la app y después en el `PATH`
    /// (más las carpetas de [`search_path`]).
    pub fn locate(data_dir: Option<&Path>) -> Option<Self> {
        if let Some(dir) = data_dir {
            if let Some((path, _)) = newest_onedir(dir) {
                return Some(Self::new(path));
            }
            let candidate = dir.join(BINARY_NAME);
            if candidate.is_file() {
                return Some(Self::new(candidate));
            }
        }
        find_in_path(BINARY_NAME, search_path().as_deref()).map(Self::new)
    }

    /// Descarga la última versión oficial a `data_dir`: primero la versión en
    /// carpeta, y si falla, la de un solo archivo.
    pub async fn download(client: &reqwest::Client, data_dir: &Path) -> Result<Self> {
        match Self::download_onedir(client, data_dir).await {
            Ok(y) => Ok(y),
            Err(e) => {
                tracing::warn!(error = %e, "no se pudo bajar yt-dlp en carpeta; se baja el de un archivo");
                Self::download_onefile(client, data_dir).await
            }
        }
    }

    /// `true` si este yt-dlp es el de la carpeta de datos de Surco (y no
    /// uno del sistema, que se actualiza por su cuenta).
    pub fn is_managed(&self, data_dir: &Path) -> bool {
        self.path.starts_with(data_dir)
    }

    /// Si Surco maneja yt-dlp y la versión en carpeta falta o tiene más de
    /// [`ONEDIR_MAX_AGE`], baja una nueva. Devuelve el yt-dlp nuevo, o
    /// `None` si no hacía falta.
    pub async fn refresh_onedir(
        &self,
        client: &reqwest::Client,
        data_dir: &Path,
    ) -> Result<Option<Self>> {
        if !self.is_managed(data_dir) {
            return Ok(None);
        }
        let latest = latest_tag(NIGHTLY_RELEASES, ONEDIR_ASSET).await;
        if let Err(e) = &latest {
            tracing::debug!(error = %e, "no se pudo ver la última versión de yt-dlp");
        }
        let latest = latest.ok();
        if let Some((bin, age)) = newest_onedir(data_dir) {
            let installed = bin
                .parent()
                .and_then(|d| std::fs::read_to_string(d.join(TAG_FILE)).ok());
            let up_to_date = match (&latest, installed) {
                (Some(latest), Some(installed)) => installed.trim() == latest,
                // Sin saber la última: se renueva por antigüedad.
                (None, _) => age < ONEDIR_MAX_AGE,
                // Bajada por una versión vieja de Surco (sin tag).
                (Some(_), None) => false,
            };
            if up_to_date {
                return Ok(None);
            }
        }
        Self::download_onedir_tag(client, data_dir, latest.as_deref())
            .await
            .map(Some)
    }

    /// Borra las versiones en carpeta viejas (todas menos la más nueva).
    /// Las que están en uso (Windows) quedan para la próxima.
    pub fn remove_old_onedirs(data_dir: &Path) {
        let Some((newest, _)) = newest_onedir(data_dir) else {
            return;
        };
        let Some(keep) = newest.parent() else {
            return;
        };
        let Ok(dirs) = std::fs::read_dir(data_dir.join(ONEDIR_DIR)) else {
            return;
        };
        for d in dirs.flatten() {
            let path = d.path();
            if path != keep && path.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&path) {
                    tracing::debug!(path = %path.display(), error = %e, "no se pudo borrar un yt-dlp viejo");
                }
            }
        }
    }

    async fn download_onedir(client: &reqwest::Client, data_dir: &Path) -> Result<Self> {
        let tag = latest_tag(NIGHTLY_RELEASES, ONEDIR_ASSET).await.ok();
        Self::download_onedir_tag(client, data_dir, tag.as_deref()).await
    }

    async fn download_onedir_tag(
        client: &reqwest::Client,
        data_dir: &Path,
        tag: Option<&str>,
    ) -> Result<Self> {
        let url = match tag {
            Some(t) => format!("{NIGHTLY_RELEASES}/download/{t}/{ONEDIR_ASSET}"),
            None => format!("{NIGHTLY_RELEASES}/latest/download/{ONEDIR_ASSET}"),
        };
        tracing::info!(%url, "descargando yt-dlp en carpeta");
        let zip = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let root = data_dir.join(ONEDIR_DIR);
        let tmp = root.join(format!("{stamp}.part"));
        let dest = root.join(stamp.to_string());
        let (tmp2, zip_len) = (tmp.clone(), zip.len());
        tokio::task::spawn_blocking(move || -> Result<()> {
            let _ = std::fs::remove_dir_all(&tmp2);
            std::fs::create_dir_all(&tmp2)?;
            crate::deno::extract_all(&zip, &tmp2)?;
            Ok(())
        })
        .await
        .map_err(|e| Error::Parse(e.to_string()))??;
        let binary = tmp.join(ONEDIR_BINARY);
        if !binary.is_file() {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err(Error::Parse(format!("el zip no trae {ONEDIR_BINARY}")));
        }
        make_executable(&binary).await?;
        if let Some(t) = tag {
            tokio::fs::write(tmp.join(TAG_FILE), t).await?;
        }
        tokio::fs::rename(&tmp, &dest).await?;
        let path = dest.join(ONEDIR_BINARY);
        tracing::info!(path = %path.display(), size = zip_len, "yt-dlp en carpeta descargado");
        Ok(Self::new(path))
    }

    /// Descarga el binario de un solo archivo a `data_dir` y lo deja ejecutable.
    async fn download_onefile(client: &reqwest::Client, data_dir: &Path) -> Result<Self> {
        let url = format!("{RELEASES_URL}{RELEASE_ASSET}");
        tracing::info!(%url, "descargando yt-dlp");
        let bytes = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;

        tokio::fs::create_dir_all(data_dir).await?;
        let dest = data_dir.join(BINARY_NAME);
        // Escribimos a un temporal y renombramos, para no dejar un binario a medias.
        let tmp = data_dir.join(format!("{BINARY_NAME}.part"));
        tokio::fs::write(&tmp, &bytes).await?;
        make_executable(&tmp).await?;
        tokio::fs::rename(&tmp, &dest).await?;
        tracing::info!(path = %dest.display(), size = bytes.len(), "yt-dlp descargado");
        Ok(Self::new(dest))
    }

    /// Versión instalada (`yt-dlp --version`).
    pub async fn version(&self) -> Result<String> {
        let out = self.run(["--version"], RESOLVE_TIMEOUT).await?;
        Ok(out.trim().to_owned())
    }

    /// Actualiza el binario (`yt-dlp -U`).
    pub async fn update(&self) -> Result<String> {
        self.run(["-U"], UPDATE_TIMEOUT).await
    }

    /// Obtiene la URL directa del audio de un video (ID o URL de YouTube).
    pub async fn stream_info(&self, video: &str) -> Result<StreamInfo> {
        self.stream_info_with(video, None, RESOLVE_TIMEOUT).await
    }

    /// Como [`stream_info`](Self::stream_info), pero pidiéndole a YouTube con
    /// otro cliente (`player_client` de yt-dlp) y con un tiempo máximo propio.
    pub async fn stream_info_with(
        &self,
        video: &str,
        client: Option<&str>,
        timeout: Duration,
    ) -> Result<StreamInfo> {
        let mut args = js_runtime_args(search_path().as_deref());
        if let Some(c) = client {
            args.push("--extractor-args".into());
            args.push(format!("youtube:player_client={c}"));
        }
        let out = self
            .run(
                args.iter().map(String::as_str).chain([
                    "--no-playlist",
                    "--no-warnings",
                    // La URL queda atada a la IP: la descarga también va por
                    // IPv4 (ver `stream_client`).
                    "--force-ipv4",
                    "-f",
                    AUDIO_FORMAT,
                    "--print",
                    "%(ext)s",
                    "--print",
                    "%(duration)s",
                    "--print",
                    "%(title)s",
                    "--print",
                    "%(http_headers)j",
                    "--print",
                    "%(url)s",
                    "--",
                    video,
                ]),
                timeout,
            )
            .await?;
        parse_stream_info(&out)
    }

    /// Baja el audio de `video` con el propio yt-dlp a `dir` y devuelve el
    /// archivo. Es el plan B cuando YouTube rechaza (403) la URL al bajarla
    /// nosotros: yt-dlp sabe pedirla con todo lo que el cliente necesita.
    pub async fn download_audio(&self, video: &str, dir: &Path) -> Result<PathBuf> {
        if !crate::is_video_id(video) {
            return Err(Error::Parse(format!("ID de video inválido: {video}")));
        }
        tokio::fs::create_dir_all(dir).await?;
        let template = dir.join(format!("{video}.ytdlp.%(ext)s"));
        let js = js_runtime_args(search_path().as_deref());
        let out = self
            .run(
                js.iter().map(String::as_str).chain([
                    "--no-playlist",
                    "--no-warnings",
                    "--force-ipv4",
                    "--no-part",
                    "--force-overwrites",
                    "-f",
                    "bestaudio[ext=m4a]",
                    "-o",
                    &template.to_string_lossy(),
                    "--print",
                    "after_move:filepath",
                    "--",
                    video,
                ]),
                DOWNLOAD_TIMEOUT,
            )
            .await?;
        let path = out
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| Error::Parse("yt-dlp no dijo dónde dejó el audio".into()))?;
        if !path.is_file() {
            return Err(Error::Parse(format!(
                "yt-dlp no dejó el audio en {}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// Busca en YouTube (`ytsearch{n}:{query}`) sin resolver cada video
    /// (`--flat-playlist`): rápido y sin tocar los servidores de video.
    ///
    /// Primero prueba la búsqueda directa ([`crate::innertube`]), que no
    /// arranca un proceso; yt-dlp queda de respaldo.
    pub async fn search(&self, query: &str, n: usize) -> Result<Vec<SearchResult>> {
        if crate::innertube::search_enabled() {
            if let Ok(found) = crate::innertube::search(query, n.clamp(1, 20)).await {
                return Ok(found);
            }
        }
        let target = format!("ytsearch{}:{}", n.clamp(1, 20), query);
        let out = self
            .run(
                [
                    "--flat-playlist",
                    "--dump-json",
                    "--no-warnings",
                    "--",
                    &target,
                ],
                RESOLVE_TIMEOUT,
            )
            .await?;
        Ok(parse_search(&out))
    }

    /// Playlists de YouTube que coinciden con `query`.
    pub async fn search_playlists(&self, query: &str, n: usize) -> Result<Vec<Listing>> {
        if crate::innertube::search_enabled() {
            if let Ok(found) = crate::innertube::search_playlists(query, n.clamp(1, 20)).await {
                return Ok(found);
            }
        }
        self.search_filtered(query, "EgIQAw%3D%3D", n).await
    }

    /// Canales (artistas) que coinciden con `query`.
    pub async fn search_channels(&self, query: &str, n: usize) -> Result<Vec<Listing>> {
        if crate::innertube::search_enabled() {
            if let Ok(found) = crate::innertube::search_channels(query, n.clamp(1, 20)).await {
                return Ok(found);
            }
        }
        self.search_filtered(query, "EgIQAg%3D%3D", n).await
    }

    /// Búsqueda de YouTube con filtro de tipo (`sp`).
    async fn search_filtered(&self, query: &str, sp: &str, n: usize) -> Result<Vec<Listing>> {
        let url = format!(
            "https://www.youtube.com/results?search_query={}&sp={sp}",
            encode_query(query)
        );
        let end = n.clamp(1, 20).to_string();
        let out = self
            .run(
                [
                    "--flat-playlist",
                    "--dump-json",
                    "--no-warnings",
                    "--playlist-end",
                    &end,
                    "--",
                    &url,
                ],
                RESOLVE_TIMEOUT,
            )
            .await?;
        Ok(parse_listings(&out))
    }

    /// Videos de una playlist (link de YouTube o YouTube Music): título y canciones.
    pub async fn playlist(&self, url: &str, max: usize) -> Result<(String, Vec<SearchResult>)> {
        let end = max.clamp(1, 5000).to_string();
        let out = self
            .run(
                [
                    "--flat-playlist",
                    "--yes-playlist",
                    "-J",
                    "--no-warnings",
                    "--playlist-end",
                    &end,
                    "--",
                    url,
                ],
                PLAYLIST_TIMEOUT,
            )
            .await?;
        parse_playlist(&out)
    }

    /// Videos parecidos a `video`: el "Mix" de YouTube (`list=RD<id>`), sin
    /// resolver cada uno. El primero suele ser el mismo video.
    pub async fn mix(&self, video: &str, n: usize) -> Result<Vec<SearchResult>> {
        if !crate::is_video_id(video) {
            return Err(Error::Parse(format!("ID de video inválido: {video}")));
        }
        let url = format!("https://www.youtube.com/watch?v={video}&list=RD{video}");
        let end = n.clamp(1, 50).to_string();
        let out = self
            .run(
                [
                    "--flat-playlist",
                    "--dump-json",
                    "--no-warnings",
                    "--playlist-end",
                    &end,
                    "--",
                    &url,
                ],
                RESOLVE_TIMEOUT,
            )
            .await?;
        Ok(parse_search(&out))
    }

    async fn run<I, S>(&self, args: I, timeout: Duration) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new(&self.path);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        hide_console(&mut cmd);
        // Para que yt-dlp encuentre deno aunque la app no herede el PATH de la terminal.
        if let Some(path) = search_path() {
            cmd.env("PATH", path);
        }

        let child = cmd.spawn().map_err(|e| Error::Spawn {
            path: self.path.clone(),
            source: e,
        })?;
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| Error::Timeout(timeout))??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("sin detalle")
                .trim()
                .to_owned();
            if is_bot_check(&stderr) {
                return Err(Error::BotCheck(msg));
            }
            return Err(Error::YtDlp(msg));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// Última versión publicada en `releases` (una página de releases de
/// GitHub), sin usar la API (que limita los pedidos):
/// `releases/latest/download/<asset>` redirige a
/// `releases/download/<versión>/<asset>`. (No se baja nada: solo se mira
/// adónde redirige.)
pub(crate) async fn latest_tag(releases: &str, asset: &str) -> Result<String> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default()
    });
    let resp = client
        .head(format!("{releases}/latest/download/{asset}"))
        .send()
        .await?;
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .ok_or_else(|| Error::Status(resp.status().as_u16()))?;
    tag_from_location(location)
        .map(str::to_owned)
        .ok_or_else(|| Error::Parse(format!("redirección inesperada: {location}")))
}

fn tag_from_location(location: &str) -> Option<&str> {
    let (_, rest) = location.rsplit_once("/releases/download/")?;
    let tag = rest.split('/').next()?;
    (!tag.is_empty() && tag != "latest").then_some(tag)
}

/// El yt-dlp en carpeta más nuevo de `data_dir` y su antigüedad.
fn newest_onedir(data_dir: &Path) -> Option<(PathBuf, Duration)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    std::fs::read_dir(data_dir.join(ONEDIR_DIR))
        .ok()?
        .flatten()
        .filter_map(|d| {
            let stamp: u64 = d.file_name().to_str()?.parse().ok()?;
            let bin = d.path().join(ONEDIR_BINARY);
            bin.is_file().then_some((stamp, bin))
        })
        .max_by_key(|(stamp, _)| *stamp)
        .map(|(stamp, bin)| (bin, Duration::from_secs(now.saturating_sub(stamp))))
}

/// Un resultado de búsqueda.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub channel: Option<String>,
    pub duration: Option<Duration>,
    pub views: Option<u64>,
}

/// Un canal o una playlist en resultados de búsqueda.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub id: String,
    pub title: String,
    pub url: String,
    /// Dueño de la playlist (para canales, vacío).
    pub channel: Option<String>,
    pub followers: Option<u64>,
}

fn parse_listings(out: &str) -> Vec<Listing> {
    out.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
            Some(Listing {
                id: text("id")?,
                title: text("title")?,
                url: text("url")?,
                channel: text("channel").or_else(|| text("uploader")),
                followers: v["channel_follower_count"].as_u64(),
            })
        })
        .collect()
}

/// Salida de `-J --flat-playlist`: un objeto con `title` y `entries`.
fn parse_playlist(out: &str) -> Result<(String, Vec<SearchResult>)> {
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).map_err(|e| Error::Parse(e.to_string()))?;
    let title = v["title"]
        .as_str()
        .unwrap_or("Playlist de YouTube")
        .to_owned();
    let entries = v["entries"]
        .as_array()
        .map(|a| a.iter().filter_map(search_result).collect())
        .unwrap_or_default();
    Ok((title, entries))
}

/// Para la URL de búsqueda: espacios como `+` y el resto con `%XX`.
fn encode_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len() * 2);
    for b in q.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Una línea JSON por resultado. Las líneas que no se entienden se ignoran.
fn parse_search(out: &str) -> Vec<SearchResult> {
    out.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| search_result(&v))
        .collect()
}

fn search_result(v: &serde_json::Value) -> Option<SearchResult> {
    let id = v["id"].as_str()?.to_owned();
    // Los resultados de canales o playlists no sirven.
    if id.len() != 11 {
        return None;
    }
    let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    Some(SearchResult {
        id,
        title: text("title")?,
        channel: text("channel").or_else(|| text("uploader")),
        duration: v["duration"]
            .as_f64()
            .filter(|d| d.is_finite() && *d > 0.0)
            .map(Duration::from_secs_f64),
        views: v["view_count"].as_u64(),
    })
}

/// Parsea la salida de `--print ext --print duration --print title
/// --print http_headers --print url` (también sin los encabezados). Los
/// encabezados vienen en JSON; de ahí sale el `User-Agent`.
fn parse_stream_info(out: &str) -> Result<StreamInfo> {
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let (ext, duration, title, user_agent, url) = match lines.as_slice() {
        [ext, duration, title, ua, url] => (ext, duration, title, Some(ua), url),
        [ext, duration, title, url] => (ext, duration, title, None, url),
        _ => {
            return Err(Error::Parse(format!(
                "se esperaban 5 líneas de yt-dlp, llegaron {}",
                lines.len()
            )))
        }
    };
    if !url.starts_with("http") {
        return Err(Error::Parse(format!("URL inválida: {url}")));
    }
    let not_na = |s: &&str| !s.is_empty() && *s != "NA";
    Ok(StreamInfo {
        url: (*url).to_owned(),
        user_agent: user_agent.and_then(|h| user_agent_of(h)),
        ext: (*ext).to_owned(),
        title: Some(*title).filter(not_na).map(str::to_owned),
        duration: Some(*duration)
            .filter(not_na)
            .and_then(|d| d.parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(Duration::from_secs_f64),
    })
}

/// `User-Agent` del JSON de `http_headers` que imprime yt-dlp.
fn user_agent_of(headers: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(headers).ok()?;
    let ua = v.get("User-Agent")?.as_str()?.trim();
    (!ua.is_empty()).then(|| ua.to_owned())
}

/// Runtime de JavaScript que va a usar yt-dlp (`deno`, `node`, `bun` o
/// `quickjs`), o `None` si no hay ninguno: sin él YouTube suele rechazar el
/// audio con 403.
pub fn js_runtime() -> Option<&'static str> {
    js_runtime_in(search_path().as_deref())
}

fn js_runtime_in(path_var: Option<&OsStr>) -> Option<&'static str> {
    JS_RUNTIMES
        .iter()
        .find(|(bin, _)| {
            let exe = if cfg!(windows) {
                format!("{bin}.exe")
            } else {
                (*bin).to_owned()
            };
            find_in_path(&exe, path_var).is_some()
        })
        .map(|(_, name)| *name)
}

const JS_RUNTIMES: [(&str, &str); 4] = [
    ("deno", "deno"),
    ("node", "node"),
    ("bun", "bun"),
    ("qjs", "quickjs"),
];

/// Desde fines de 2025, yt-dlp necesita un runtime de JavaScript externo para
/// resolver los desafíos de YouTube. Por defecto solo prueba `deno`; si no está
/// pero hay otro soportado en el `PATH`, se lo indicamos con `--js-runtimes`.
fn js_runtime_args(path_var: Option<&OsStr>) -> Vec<String> {
    match js_runtime_in(path_var) {
        // deno es el default de yt-dlp: no hace falta pasar nada.
        None | Some("deno") => Vec::new(),
        Some(name) => vec!["--js-runtimes".to_owned(), name.to_owned()],
    }
}

/// YouTube a veces exige "confirmar que no sos un bot" (típico en IPs de
/// datacenter o VPN). No es un error de la app.
fn is_bot_check(stderr: &str) -> bool {
    stderr.contains("not a bot") || stderr.contains("HTTP Error 429")
}

/// `PATH` más las carpetas donde suelen instalarse yt-dlp y deno.
///
/// Una app abierta desde el Finder de macOS (o desde el menú de algunos
/// escritorios Linux) no hereda el `PATH` de la terminal, así que Homebrew
/// (`/opt/homebrew/bin`) o `~/.deno/bin` no aparecen. Se agregan al final.
pub fn search_path() -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(unix) {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for sub in [".deno/bin", ".bun/bin", ".local/bin"] {
                dirs.push(home.join(sub));
            }
        }
    }
    // En Windows, el instalador de deno lo pone en %USERPROFILE%\.deno\bin y
    // cambia el PATH del usuario, pero Surco puede haberse abierto antes.
    if cfg!(windows) {
        if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
            for sub in [".deno\\bin", ".bun\\bin"] {
                dirs.push(home.join(sub));
            }
        }
    }
    if cfg!(target_os = "macos") {
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    // El deno que bajó Surco.
    if let Some(dir) = crate::deno::tools_dir() {
        dirs.push(dir.to_path_buf());
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    std::env::join_paths(dirs).ok()
}

fn find_in_path(name: &str, path_var: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// En Windows, que los procesos hijos no abran una ventana de consola.
pub(crate) fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(unix)]
pub(crate) async fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) async fn make_executable(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tag_from_redirect() {
        assert_eq!(
            tag_from_location(
                "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/download/2026.11.24.232653/yt-dlp_linux.zip"
            ),
            Some("2026.11.24.232653")
        );
        assert_eq!(
            tag_from_location(
                "https://github.com/denoland/deno/releases/download/v2.5.6/deno-x86_64-unknown-linux-gnu.zip"
            ),
            Some("v2.5.6")
        );
        assert_eq!(tag_from_location("https://github.com/login"), None);
    }

    /// `cargo test -p surco-provider real_latest_tag -- --ignored`.
    #[tokio::test]
    #[ignore = "usa la red"]
    async fn real_latest_tag() {
        let t = latest_tag(NIGHTLY_RELEASES, ONEDIR_ASSET).await.unwrap();
        assert!(t.starts_with("20"), "{t}");
    }

    /// Baja el yt-dlp en carpeta de verdad y lo ejecuta:
    /// `cargo test -p surco-provider real_onedir -- --ignored`.
    #[tokio::test]
    #[ignore = "usa la red"]
    async fn real_onedir() {
        let dir = std::env::temp_dir().join(format!("surco-onedir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let y = YtDlp::download(&reqwest::Client::new(), &dir)
            .await
            .unwrap();
        assert!(y.path().starts_with(dir.join(ONEDIR_DIR)));
        assert!(!y.version().await.unwrap().is_empty());
        assert_eq!(YtDlp::locate(Some(&dir)).unwrap().path(), y.path());
        // Recién bajado: no hace falta otro.
        assert!(y
            .refresh_onedir(&reqwest::Client::new(), &dir)
            .await
            .unwrap()
            .is_none());
        YtDlp::remove_old_onedirs(&dir);
        assert!(y.path().is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_playlist_and_listings() {
        let out = r#"{"id":"PLx","title":"Grandes éxitos","entries":[
            {"id":"AVEDgT_lG60","title":"Soda Stereo - En la Ciudad de la Furia","channel":"Soda Stereo","duration":377},
            {"id":"PLnested","title":"no es un video"},
            {"id":"OX-us7PEfkc","title":"De Música Ligera","duration":290.0}]}"#;
        let (title, list) = parse_playlist(out).unwrap();
        assert_eq!(title, "Grandes éxitos");
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].duration, Some(Duration::from_secs(290)));

        let out = concat!(
            r#"{"id":"UC_x","title":"Soda Stereo","url":"https://www.youtube.com/channel/UC_x","channel_follower_count":3040000}"#,
            "\n",
            r#"{"id":"PLy","title":"Mix","url":"https://www.youtube.com/playlist?list=PLy","channel":"Alguien"}"#,
        );
        let l = parse_listings(out);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].followers, Some(3_040_000));
        assert_eq!(l[1].channel.as_deref(), Some("Alguien"));
    }

    #[test]
    fn encodes_queries() {
        assert_eq!(encode_query(" soda stereo "), "soda+stereo");
        assert_eq!(encode_query("ñ&?"), "%C3%B1%26%3F");
    }

    #[test]
    fn parses_stream_info() {
        let out = "m4a\n212.5\nRick Astley - Never Gonna Give You Up\n{\"User-Agent\": \"Mozilla/5.0 (X11)\", \"Accept\": \"*/*\"}\nhttps://rr1.googlevideo.com/x?y=1\n";
        let info = parse_stream_info(out).unwrap();
        assert_eq!(info.user_agent.as_deref(), Some("Mozilla/5.0 (X11)"));
        assert_eq!(info.ext, "m4a");
        assert!(info.is_supported());
        assert_eq!(info.duration, Some(Duration::from_millis(212_500)));
        assert_eq!(
            info.title.as_deref(),
            Some("Rick Astley - Never Gonna Give You Up")
        );
        assert!(info.url.starts_with("https://"));
    }

    #[test]
    fn parses_missing_fields_as_none() {
        let info = parse_stream_info("webm\nNA\nNA\nhttps://a/b\n").unwrap();
        assert_eq!(info.user_agent, None);
        assert_eq!(info.duration, None);
        assert_eq!(info.title, None);
        assert!(!info.is_supported());
    }

    #[test]
    fn rejects_bad_output() {
        assert!(parse_stream_info("").is_err());
        assert!(parse_stream_info("m4a\n1\nt\nnot-a-url").is_err());
    }

    #[test]
    fn parses_search_results() {
        let out = r#"{"id":"a01QQZyl-_I","title":"Queen - Under Pressure","channel":"Queen Official","duration":254.0,"view_count":10}
{"id":"UCxxxxxxxxxxxxxxxxxxxxxx","title":"Un canal"}
no es json
{"id":"dN3gax_Z7tw","title":"Live","uploader":"Alguien","duration":null}"#;
        let r = parse_search(out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].channel.as_deref(), Some("Queen Official"));
        assert_eq!(r[0].duration, Some(Duration::from_secs(254)));
        assert_eq!(r[1].channel.as_deref(), Some("Alguien"));
        assert_eq!(r[1].duration, None);
    }

    #[test]
    fn detects_bot_check() {
        assert!(is_bot_check(
            "ERROR: [youtube] x: Sign in to confirm you’re not a bot. Use --cookies"
        ));
        assert!(!is_bot_check("ERROR: Video unavailable"));
    }

    #[test]
    fn picks_js_runtime() {
        let dir = std::env::temp_dir().join(format!("surco-js-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::env::join_paths([dir.clone()]).unwrap();
        assert!(js_runtime_args(Some(&path)).is_empty());
        let exe = |n: &str| {
            dir.join(if cfg!(windows) {
                format!("{n}.exe")
            } else {
                n.to_owned()
            })
        };
        std::fs::write(exe("node"), b"").unwrap();
        assert_eq!(js_runtime_args(Some(&path)), ["--js-runtimes", "node"]);
        std::fs::write(exe("deno"), b"").unwrap();
        assert!(js_runtime_args(Some(&path)).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finds_binary_in_path() {
        let dir = std::env::temp_dir().join(format!("surco-path-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fake-bin"), b"").unwrap();
        let path = std::env::join_paths([PathBuf::from("/nonexistent"), dir.clone()]).unwrap();
        assert_eq!(
            find_in_path("fake-bin", Some(&path)),
            Some(dir.join("fake-bin"))
        );
        assert_eq!(find_in_path("missing", Some(&path)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
