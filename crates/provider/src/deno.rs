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

/// Saca el archivo `name` de un zip (guardado o comprimido con deflate),
/// leyendo el directorio central del final.
pub(crate) fn extract(zip: &[u8], name: &str) -> Result<Vec<u8>> {
    let bad = |what: &str| Error::Parse(format!("zip inválido: {what}"));
    let u16_at = |at: usize| -> Result<usize> {
        zip.get(at..at + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .ok_or_else(|| bad("corto"))
    };
    let u32_at = |at: usize| -> Result<usize> {
        zip.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| bad("corto"))
    };

    // Fin del directorio central: firma PK\x05\x06, buscada desde el final.
    let eocd = (0..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| zip[i..].starts_with(b"PK\x05\x06"))
        .ok_or_else(|| bad("sin directorio central"))?;
    let entries = u16_at(eocd + 10)?;
    let mut at = u32_at(eocd + 16)?;
    for _ in 0..entries {
        if u32_at(at)? != 0x0201_4b50 {
            return Err(bad("entrada rota"));
        }
        let method = u16_at(at + 10)?;
        let compressed = u32_at(at + 20)?;
        let size = u32_at(at + 24)?;
        let name_len = u16_at(at + 28)?;
        let extra_len = u16_at(at + 30)?;
        let comment_len = u16_at(at + 32)?;
        let local = u32_at(at + 42)?;
        let entry = zip
            .get(at + 46..at + 46 + name_len)
            .ok_or_else(|| bad("nombre"))?;
        at += 46 + name_len + extra_len + comment_len;
        if entry != name.as_bytes() {
            continue;
        }
        if u32_at(local)? != 0x0403_4b50 {
            return Err(bad("encabezado local"));
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let data = zip
            .get(start..start + compressed)
            .ok_or_else(|| bad("datos"))?;
        return match method {
            0 => Ok(data.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(data).read_to_end(&mut out)?;
                Ok(out)
            }
            m => Err(bad(&format!("compresión {m}"))),
        };
    }
    Err(bad(&format!("no tiene {name}")))
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
