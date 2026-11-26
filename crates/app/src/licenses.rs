//! Botón "Licencias" de Ajustes: abre `THIRD_PARTY_LICENSES.html`, la lista
//! de dependencias con el texto de cada licencia. La arma `cargo about` en el
//! workflow de release (ver `about.toml`) y va dentro de cada paquete, en una
//! carpeta que depende de cómo se instaló Surco.

use std::path::{Path, PathBuf};

const FILE: &str = "THIRD_PARTY_LICENSES.html";

/// Si no se encuentra el archivo (por ejemplo, compilado a mano): la sección
/// de licencia del README.
const FALLBACK_URL: &str = "https://github.com/devMathiuX/Surco#licencia";

/// Dónde puede estar el archivo, según la carpeta del ejecutable.
fn candidates(exe_dir: &Path) -> [PathBuf; 3] {
    [
        // Windows (instalador y .zip) y .tar.gz de Linux: al lado del ejecutable.
        exe_dir.join(FILE),
        // .deb (/usr/bin -> /usr/share/doc/surco) y AppImage (igual, adentro).
        exe_dir.join("../share/doc/surco").join(FILE),
        // macOS: Surco.app/Contents/MacOS -> Contents/Resources.
        exe_dir.join("../Resources").join(FILE),
    ]
}

/// El archivo de licencias de esta instalación, si está.
pub fn local_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    candidates(dir).into_iter().find(|p| p.is_file())
}

/// Abre las licencias de terceros (el archivo local o, si no está, la web).
pub fn open() {
    match local_file() {
        Some(path) => crate::updater::open_in_browser(&path.to_string_lossy()),
        None => {
            tracing::info!("no está {FILE}; se abre la web");
            crate::updater::open_in_browser(FALLBACK_URL);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn finds_file_in_each_layout() {
        let root = std::env::temp_dir().join(format!("surco-licenses-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("usr/bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert!(candidates(&bin).iter().all(|p| !p.is_file()));
        let doc = root.join("usr/share/doc/surco");
        std::fs::create_dir_all(&doc).unwrap();
        std::fs::write(doc.join(FILE), "x").unwrap();
        assert!(candidates(&bin).iter().any(|p| p.is_file()));
        let _ = std::fs::remove_dir_all(root);
    }
}
