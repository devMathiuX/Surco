//! Caché en disco del audio de los videos: `<carpeta>/<id>.m4a`.
//!
//! Sirve para "precargar": las próximas canciones se bajan antes de que
//! suenen, así no hay que esperar ni depende de la red en ese momento. Se
//! borran las menos usadas cuando se pasa del tope de tamaño.
//!
//! Los videos "fijados" (los de las playlists descargadas) no cuentan para el
//! tope y nunca se borran solos.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::{Error, Result};

/// Tope por defecto: 2 GB (unas 500 canciones).
pub const DEFAULT_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct AudioCache {
    dir: PathBuf,
    limit: u64,
    /// Videos que no se borran solos (compartido entre las copias).
    pinned: Arc<Mutex<HashSet<String>>>,
    /// La última canción escuchada: tampoco se borra (para retomarla al
    /// abrir la app sin esperar).
    kept: Arc<Mutex<Option<String>>>,
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
            pinned: Arc::default(),
            kept: Arc::default(),
        }
    }

    /// Cambia la canción que se guarda para retomar al abrir. La anterior
    /// queda como precarga común (se borra al cerrar).
    pub fn keep(&self, video: Option<&str>) {
        if let Ok(mut k) = self.kept.lock() {
            *k = video.map(str::to_owned);
        }
    }

    /// Cambia los videos fijados (los de las playlists descargadas) y
    /// devuelve los que estaban fijados y ya no.
    pub fn set_pinned(&self, videos: HashSet<String>) -> Vec<String> {
        let Ok(mut p) = self.pinned.lock() else {
            return Vec::new();
        };
        let old = std::mem::replace(&mut *p, videos);
        old.into_iter().filter(|v| !p.contains(v)).collect()
    }

    pub fn is_pinned(&self, video: &str) -> bool {
        self.pinned.lock().is_ok_and(|p| p.contains(video))
            || self.kept.lock().is_ok_and(|k| k.as_deref() == Some(video))
    }

    /// Si el archivo es de un video fijado.
    fn pinned_path(&self, path: &Path) -> bool {
        path.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|v| self.is_pinned(v))
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
        entries.retain(|(path, _, _)| !self.pinned_path(path));
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

    /// Tamaño en disco del audio de `video` (`None` si no está bajado).
    pub fn size_of(&self, video: &str) -> Option<u64> {
        let meta = std::fs::metadata(self.path(video)?).ok()?;
        (meta.is_file() && meta.len() > 0).then_some(meta.len())
    }

    /// Borra el audio de `video`. Devuelve los bytes liberados.
    pub fn remove(&self, video: &str) -> u64 {
        let Some(len) = self.size_of(video) else {
            return 0;
        };
        match self.path(video).map(std::fs::remove_file) {
            Some(Ok(())) => len,
            _ => 0,
        }
    }

    /// Borra todo el audio guardado menos el fijado.
    pub fn clear(&self) {
        for (path, _, _) in self.entries() {
            if !self.pinned_path(&path) {
                let _ = std::fs::remove_file(path);
            }
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
    fn kept_song_survives_clear() {
        let dir = temp_dir("kept");
        let c = AudioCache::new(&dir, DEFAULT_LIMIT);
        c.put("aaaaaaaaaaa", b"uno").unwrap();
        c.put("bbbbbbbbbbb", b"dos").unwrap();
        c.keep(Some("aaaaaaaaaaa"));
        c.clear();
        assert!(c.contains("aaaaaaaaaaa"));
        assert!(!c.contains("bbbbbbbbbbb"));
        c.keep(None);
        c.clear();
        assert!(!c.contains("aaaaaaaaaaa"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pinned_are_never_evicted_nor_cleared() {
        let dir = temp_dir("pinned");
        let c = AudioCache::new(&dir, 15);
        assert!(c.set_pinned(["aaaaaaaaaaa".to_owned()].into()).is_empty());
        c.put("aaaaaaaaaaa", b"0123456789").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        c.put("bbbbbbbbbbb", b"0123456789").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        c.put("ccccccccccc", b"0123456789").unwrap();
        // El fijado no cuenta para el tope: se va el más viejo de los otros.
        assert!(c.contains("aaaaaaaaaaa"));
        assert!(!c.contains("bbbbbbbbbbb"));
        assert!(c.contains("ccccccccccc"));
        c.clear();
        assert!(c.contains("aaaaaaaaaaa"));
        assert!(!c.contains("ccccccccccc"));
        assert_eq!(c.set_pinned(HashSet::new()), vec!["aaaaaaaaaaa".to_owned()]);
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn removes_one_video() {
        let dir = temp_dir("remove");
        let c = AudioCache::new(&dir, 1000);
        c.put("aaaaaaaaaaa", &[1; 10]).unwrap();
        c.put("bbbbbbbbbbb", &[2; 7]).unwrap();
        assert_eq!(c.size_of("bbbbbbbbbbb"), Some(7));
        assert_eq!(c.remove("aaaaaaaaaaa"), 10);
        assert_eq!(c.remove("aaaaaaaaaaa"), 0);
        assert_eq!(c.size_of("aaaaaaaaaaa"), None);
        assert!(c.contains("bbbbbbbbbbb"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
