//! Caché en disco del audio de los videos: `<carpeta>/<id>.m4a`.
//!
//! Sirve para "precargar": las próximas canciones se bajan antes de que
//! suenen, así no hay que esperar ni depende de la red en ese momento. Se
//! borran las menos usadas cuando se pasa del tope de tamaño.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{Error, Result};

/// Tope por defecto: 2 GB (unas 500 canciones).
pub const DEFAULT_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct AudioCache {
    dir: PathBuf,
    limit: u64,
}

/// Uso actual de la caché.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheUsage {
    pub files: usize,
    pub bytes: u64,
}

/// IDs de YouTube: 11 caracteres `[A-Za-z0-9_-]`. Evita rutas raras.
pub fn is_video_id(s: &str) -> bool {
    s.len() == 11
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl AudioCache {
    pub fn new(dir: impl Into<PathBuf>, limit: u64) -> Self {
        Self {
            dir: dir.into(),
            limit,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, video: &str) -> Option<PathBuf> {
        is_video_id(video).then(|| self.dir.join(format!("{video}.m4a")))
    }

    /// Archivo ya bajado de `video`, si existe. Lo marca como usado.
    pub fn get(&self, video: &str) -> Option<PathBuf> {
        let path = self.path(video)?;
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() == 0 {
            return None;
        }
        // Fecha de modificación = último uso (para borrar las más viejas).
        if let Ok(f) = std::fs::File::options().append(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(path)
    }

    pub fn contains(&self, video: &str) -> bool {
        self.path(video).is_some_and(|p| p.is_file())
    }

    /// Guarda `data` como el audio de `video` (escritura atómica) y hace lugar.
    pub fn put(&self, video: &str, data: &[u8]) -> Result<PathBuf> {
        let path = self
            .path(video)
            .ok_or_else(|| Error::Parse(format!("ID de video inválido: {video}")))?;
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!("{video}.part"));
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &path)?;
        self.evict(Some(&path));
        Ok(path)
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        dir.filter_map(|e| {
            let e = e.ok()?;
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("m4a") {
                return None;
            }
            let meta = e.metadata().ok()?;
            Some((
                path,
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        })
        .collect()
    }

    pub fn usage(&self) -> CacheUsage {
        let entries = self.entries();
        CacheUsage {
            files: entries.len(),
            bytes: entries.iter().map(|(_, len, _)| len).sum(),
        }
    }

    /// Borra las menos usadas hasta quedar debajo del tope (nunca `keep`).
    fn evict(&self, keep: Option<&Path>) {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
        if total <= self.limit {
            return;
        }
        entries.sort_by_key(|(_, _, at)| *at);
        for (path, len, _) in entries {
            if total <= self.limit {
                break;
            }
            if Some(path.as_path()) == keep {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
                tracing::debug!(path = %path.display(), "caché de audio: borrado");
            }
        }
    }

    /// Borra todo el audio guardado.
    pub fn clear(&self) {
        for (path, _, _) in self.entries() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("surco-audio-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn rejects_odd_ids() {
        let c = AudioCache::new(temp_dir("ids"), 1000);
        assert!(c.put("../../etc/x", b"a").is_err());
        assert!(c.get("../../etc/x").is_none());
        assert!(!is_video_id("abc"));
        assert!(is_video_id("dQw4w9WgXcQ"));
    }

    #[test]
    fn put_get_and_evict_oldest() {
        let dir = temp_dir("evict");
        let c = AudioCache::new(&dir, 25);
        c.put("aaaaaaaaaaa", &[1; 10]).unwrap();
        let old = SystemTime::now() - Duration::from_secs(100);
        std::fs::File::options()
            .append(true)
            .open(dir.join("aaaaaaaaaaa.m4a"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        c.put("bbbbbbbbbbb", &[2; 10]).unwrap();
        assert!(c.get("aaaaaaaaaaa").is_some());
        assert!(c.get("bbbbbbbbbbb").is_some());
        // Usar "a" la vuelve la más reciente: al pasarse se va "b".
        std::fs::File::options()
            .append(true)
            .open(dir.join("bbbbbbbbbbb.m4a"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        c.put("ccccccccccc", &[3; 10]).unwrap();
        assert!(c.contains("aaaaaaaaaaa"));
        assert!(!c.contains("bbbbbbbbbbb"));
        assert!(c.contains("ccccccccccc"));
        assert_eq!(
            c.usage(),
            CacheUsage {
                files: 2,
                bytes: 20
            }
        );
        c.clear();
        assert_eq!(c.usage().files, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
