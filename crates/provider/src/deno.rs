//! deno: el runtime de JavaScript que yt-dlp necesita para resolver los
//! desafíos de YouTube. Sin él, YouTube rechaza el audio (403).
//!
//! Si el usuario no lo tiene, Surco lo baja de los releases oficiales a su
//! carpeta de datos (al lado de yt-dlp) y la agrega al `PATH` de yt-dlp.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::{Error, Result};

/// Nombre del binario según el SO.
#[cfg(windows)]
pub const DENO_BINARY: &str = "deno.exe";
#[cfg(not(windows))]
pub const DENO_BINARY: &str = "deno";

/// Zip del release oficial para este SO y procesador.
const ASSET: Option<&str> = if cfg!(all(windows, target_arch = "x86_64")) {
    Some("deno-x86_64-pc-windows-msvc.zip")
} else if cfg!(all(windows, target_arch = "aarch64")) {
    Some("deno-aarch64-pc-windows-msvc.zip")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("deno-aarch64-apple-darwin.zip")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("deno-x86_64-apple-darwin.zip")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("deno-x86_64-unknown-linux-gnu.zip")
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some("deno-aarch64-unknown-linux-gnu.zip")
} else {
    None
};

const RELEASES_URL: &str = "https://github.com/denoland/deno/releases/latest/download/";

/// Carpeta de herramientas de Surco (donde quedan yt-dlp y deno).
static TOOLS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Indica la carpeta donde Surco guarda sus herramientas, para que yt-dlp
/// encuentre el deno que bajó Surco.
pub fn set_tools_dir(dir: PathBuf) {
    let _ = TOOLS_DIR.set(dir);
}

pub(crate) fn tools_dir() -> Option<&'static Path> {
    TOOLS_DIR.get().map(PathBuf::as_path)
}

/// Baja deno a `dir` y devuelve la ruta del binario.
pub async fn download_deno(client: &reqwest::Client, dir: &Path) -> Result<PathBuf> {
    let asset = ASSET.ok_or_else(|| Error::Parse("no hay deno para este sistema".into()))?;
    let url = format!("{RELEASES_URL}{asset}");
    tracing::info!(%url, "descargando deno");
    let zip = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let binary = extract(&zip, DENO_BINARY)?;

    tokio::fs::create_dir_all(dir).await?;
    let dest = dir.join(DENO_BINARY);
    let tmp = dir.join(format!("{DENO_BINARY}.part"));
    tokio::fs::write(&tmp, &binary).await?;
    crate::ytdlp::make_executable(&tmp).await?;
    tokio::fs::rename(&tmp, &dest).await?;
    tracing::info!(path = %dest.display(), size = binary.len(), "deno descargado");
    Ok(dest)
}

/// Una entrada del directorio central de un zip.
struct Entry<'a> {
    name: &'a [u8],
    method: usize,
    compressed: usize,
    size: usize,
    local: usize,
    /// Permisos de Unix (los 16 bits altos de los atributos externos).
    mode: u32,
}

fn bad(what: &str) -> Error {
    Error::Parse(format!("zip inválido: {what}"))
}

fn u16_at(zip: &[u8], at: usize) -> Result<usize> {
    zip.get(at..at + 2)
        .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
        .ok_or_else(|| bad("corto"))
}

fn u32_at(zip: &[u8], at: usize) -> Result<usize> {
    zip.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(|| bad("corto"))
}

/// Lee el directorio central del final del zip.
fn entries(zip: &[u8]) -> Result<Vec<Entry<'_>>> {
    // Fin del directorio central: firma PK\x05\x06, buscada desde el final.
    let eocd = (0..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| zip[i..].starts_with(b"PK\x05\x06"))
        .ok_or_else(|| bad("sin directorio central"))?;
    let count = u16_at(zip, eocd + 10)?;
    let mut at = u32_at(zip, eocd + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(zip, at)? != 0x0201_4b50 {
            return Err(bad("entrada rota"));
        }
        let name_len = u16_at(zip, at + 28)?;
        let extra_len = u16_at(zip, at + 30)?;
        let comment_len = u16_at(zip, at + 32)?;
        out.push(Entry {
            method: u16_at(zip, at + 10)?,
            compressed: u32_at(zip, at + 20)?,
            size: u32_at(zip, at + 24)?,
            mode: (u32_at(zip, at + 38)? >> 16) as u32,
            local: u32_at(zip, at + 42)?,
            name: zip
                .get(at + 46..at + 46 + name_len)
                .ok_or_else(|| bad("nombre"))?,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Datos descomprimidos de una entrada.
fn read_entry(zip: &[u8], e: &Entry<'_>) -> Result<Vec<u8>> {
    if u32_at(zip, e.local)? != 0x0403_4b50 {
        return Err(bad("encabezado local"));
    }
    let start = e.local + 30 + u16_at(zip, e.local + 26)? + u16_at(zip, e.local + 28)?;
    let data = zip
        .get(start..start + e.compressed)
        .ok_or_else(|| bad("datos"))?;
    match e.method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::with_capacity(e.size);
            flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)?;
            Ok(out)
        }
        m => Err(bad(&format!("compresión {m}"))),
    }
}

/// Cada cuánto se fija si hay un deno nuevo.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Si en `dir` hay un deno que bajó Surco y salió uno nuevo, lo actualiza
/// (como mucho una vez por día). Devuelve la ruta si lo actualizó.
pub async fn refresh_deno(client: &reqwest::Client, dir: &Path) -> Result<Option<PathBuf>> {
    let path = dir.join(DENO_BINARY);
    if !path.is_file() {
        return Ok(None);
    }
    let stamp = dir.join(".deno_check");
    let recent = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < CHECK_EVERY);
    if recent {
        return Ok(None);
    }
    let asset = ASSET.ok_or_else(|| Error::Parse("no hay deno para este sistema".into()))?;
    let latest =
        crate::ytdlp::latest_tag("https://github.com/denoland/deno/releases", asset).await?;
    let _ = std::fs::write(&stamp, &latest);
    let out = tokio::process::Command::new(&path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let installed = out
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| parse_deno_version(&s).map(str::to_owned));
    if installed.as_deref() == Some(latest.trim_start_matches('v')) {
        return Ok(None);
    }
    tracing::info!(?installed, %latest, "actualizando deno");
    download_deno(client, dir).await.map(Some)
}

/// `deno 2.5.6 (stable, release, x86_64-unknown-linux-gnu)` → `2.5.6`.
fn parse_deno_version(out: &str) -> Option<&str> {
    out.lines()
        .next()?
        .strip_prefix("deno ")?
        .split_whitespace()
        .next()
}

/// Saca el archivo `name` de un zip (guardado o comprimido con deflate),
/// leyendo el directorio central del final.
pub(crate) fn extract(zip: &[u8], name: &str) -> Result<Vec<u8>> {
    let entry = entries(zip)?
        .into_iter()
        .find(|e| e.name == name.as_bytes())
        .ok_or_else(|| bad(&format!("no tiene {name}")))?;
    read_entry(zip, &entry)
}

/// Descomprime todo el zip en `dir` (que tiene que existir). Rechaza rutas
/// que se salgan de `dir`. Devuelve cuántos archivos escribió.
pub(crate) fn extract_all(zip: &[u8], dir: &Path) -> Result<usize> {
    let mut files = 0;
    for e in entries(zip)? {
        let name = std::str::from_utf8(e.name).map_err(|_| bad("nombre no UTF-8"))?;
        let rel = Path::new(name);
        let safe = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !safe || name.contains('\\') {
            return Err(bad(&format!("ruta peligrosa: {name}")));
        }
        let dest = dir.join(rel);
        if name.ends_with('/') {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, read_entry(zip, &e)?)?;
        #[cfg(unix)]
        if e.mode & 0o111 != 0 {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(e.mode & 0o777))?;
        }
        #[cfg(not(unix))]
        let _ = e.mode;
        files += 1;
    }
    Ok(files)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const ZIP: &[u8] = include_bytes!("../testdata/deno.zip");

    #[test]
    fn extracts_deflated_entry() {
        let got = extract(ZIP, "deno").unwrap();
        assert_eq!(got, "binario de prueba ".repeat(20).as_bytes());
        assert_eq!(extract(ZIP, "README.txt").unwrap(), b"hola");
        assert!(extract(ZIP, "deno.exe").is_err());
        assert!(extract(b"no es un zip", "deno").is_err());
    }

    /// Un deno viejo se reemplaza por el último:
    /// `cargo test -p surco-provider real_refresh_deno -- --ignored`.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "usa la red"]
    async fn real_refresh_deno() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("surco-deno-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join(DENO_BINARY);
        std::fs::write(&old, "#!/bin/sh\necho 'deno 0.0.1 (stable)'\n").unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        let client = reqwest::Client::new();
        assert!(refresh_deno(&client, &dir).await.unwrap().is_some());
        // Ya revisado hoy: no vuelve a mirar.
        assert!(refresh_deno(&client, &dir).await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_deno_version() {
        assert_eq!(
            parse_deno_version("deno 2.5.6 (stable, release, x86_64-unknown-linux-gnu)\nv8 14.0"),
            Some("2.5.6")
        );
        assert_eq!(parse_deno_version("otra cosa"), None);
    }

    #[test]
    fn extracts_everything() {
        let dir = std::env::temp_dir().join(format!("surco-unzip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(extract_all(ZIP, &dir).unwrap(), 2);
        assert_eq!(std::fs::read(dir.join("README.txt")).unwrap(), b"hola");
        assert!(dir.join("deno").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Con el zip real de yt-dlp: `SURCO_YTDLP_ZIP=yt-dlp_linux.zip
    /// cargo test -p surco-provider real_ytdlp_zip -- --ignored`.
    #[test]
    #[ignore = "necesita el zip real de yt-dlp"]
    fn real_ytdlp_zip() {
        let zip = std::fs::read(std::env::var("SURCO_YTDLP_ZIP").unwrap()).unwrap();
        let dir = std::env::temp_dir().join(format!("surco-ytdlp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(extract_all(&zip, &dir).unwrap() > 100);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Con un zip real: `SURCO_DENO_ZIP=deno-x86_64-pc-windows-msvc.zip
    /// cargo test -p surco-provider real_deno_zip -- --ignored`.
    #[test]
    #[ignore = "necesita un zip real de deno"]
    fn real_deno_zip() {
        let path = std::env::var("SURCO_DENO_ZIP").unwrap();
        let zip = std::fs::read(path).unwrap();
        let exe = extract(&zip, "deno.exe").unwrap();
        assert!(exe.starts_with(b"MZ"));
        assert!(exe.len() > 10_000_000);
    }
}
