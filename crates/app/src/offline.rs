//! Playlists descargadas: su audio queda en disco para escucharlas sin
//! esperar ni depender de la red.
//!
//! - La flechita de la playlist la marca como descargada; las canciones se
//!   bajan de a una (con la misma cola que la precarga).
//! - Sus videos quedan "fijados" en la caché de audio: no cuentan para el
//!   tope y nunca se borran solos.
//! - Al dejar de descargarla, o al quitar la playlist, se borra el audio de
//!   las canciones que no estén en otra playlist descargada. Lo mismo pasa
//!   con una canción que sale de una playlist descargada (o a la que se le
//!   cambia el video).
//! - El resto de la caché (la precarga) es temporal: se borra al abrir
//!   Surco. Así, lo único que queda guardado de una vez para otra es lo de
//!   las playlists descargadas.

use std::collections::HashSet;

use surco_core::Track;
use surco_resolver::cache_key;

use crate::playback::TrackRes;
use crate::{Notice, Surco};

/// Clave de ajustes: IDs de playlists descargadas, una por línea.
pub const OFFLINE: &str = "offline_playlists";

impl Surco {
    pub(crate) fn is_offline(&self, p: usize) -> bool {
        self.playlists
            .get(p)
            .is_some_and(|(pl, _)| self.offline.contains(&pl.id))
    }

    pub(crate) fn load_offline(&mut self, saved: Option<String>) {
        self.offline = saved
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
    }

    fn save_offline(&self) {
        let mut ids: Vec<&str> = self.offline.iter().map(String::as_str).collect();
        ids.sort_unstable();
        self.save_setting(OFFLINE, &ids.join("\n"));
    }

    /// Videos de las playlists descargadas (menos la `except`).
    fn offline_videos(&self, except: Option<usize>) -> HashSet<String> {
        self.playlists
            .iter()
            .enumerate()
            .filter(|(i, (p, _))| Some(*i) != except && self.offline.contains(&p.id))
            .flat_map(|(_, (p, _))| p.tracks.iter())
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect()
    }

    /// Fija en la caché el audio de las playlists descargadas y borra el de
    /// las canciones que dejaron de estar en alguna (las sacaron de la
    /// playlist o se eligió otro video).
    pub(crate) fn update_pins(&self) {
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            for v in cache.set_pinned(self.offline_videos(None)) {
                if cache.remove(&v) > 0 {
                    tracing::info!(video = %v, "audio que ya no está en una playlist descargada, borrado");
                }
            }
        }
    }

    /// Al arrancar: borra la precarga de la vez anterior. Solo queda el audio
    /// de las playlists descargadas (tiene que ir después de cargar la
    /// biblioteca y fijar sus videos).
    pub(crate) fn drop_temporary_audio(&self) {
        if let Some(cache) = self.provider.as_ref().and_then(|p| p.audio_cache()) {
            let before = cache.usage();
            cache.clear();
            let after = cache.usage();
            if before.files > after.files {
                tracing::info!(
                    files = before.files - after.files,
                    "precarga de la vez anterior borrada"
                );
            }
        }
    }

    /// Flechita de la playlist: empieza a descargarla.
    pub(crate) fn start_offline(&mut self, p: usize) {
        let Some((pl, _)) = self.playlists.get(p) else {
            return;
        };
        self.offline.insert(pl.id.clone());
        self.save_offline();
        self.update_pins();
        self.precache_dirty = true;
    }

    /// Deja de descargar la playlist y borra su audio (salvo el que esté en
    /// otra playlist descargada).
    pub(crate) fn stop_offline(&mut self, p: usize) {
        let Some((pl, _)) = self.playlists.get(p) else {
            return;
        };
        let name = pl.name.clone();
        let videos: Vec<String> = pl
            .tracks
            .iter()
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect();
        self.offline.remove(&pl.id);
        self.save_offline();
        let (files, bytes) = self.drop_audio(&videos, Some(p));
        tracing::info!(files, bytes, playlist = %name, "descargas borradas");
        self.notice = Some(Notice {
            error: false,
            text: crate::i18n::deleted_downloads(files, bytes, &name),
        });
    }

    /// Antes de quitar la playlist `p`: deja de descargarla y borra el audio
    /// de sus canciones que no estén en otra playlist descargada.
    pub(crate) fn forget_offline(&mut self, p: usize) {
        let Some((pl, _)) = self.playlists.get(p) else {
            return;
        };
        let videos: Vec<String> = pl
            .tracks
            .iter()
            .filter_map(|t| self.video_of_track(t).map(str::to_owned))
            .collect();
        if self.offline.remove(&pl.id) {
            self.save_offline();
        }
        self.drop_audio(&videos, Some(p));
    }

    /// Borra el audio de `videos` que no esté en otra playlist descargada
    /// (sin contar la `except`). Devuelve canciones y bytes borrados.
    fn drop_audio(&mut self, videos: &[String], except: Option<usize>) -> (usize, u64) {
        let keep = self.offline_videos(except);
        let Some(cache) = self
            .provider
            .as_ref()
            .and_then(|p| p.audio_cache())
            .cloned()
        else {
            return (0, 0);
        };
        cache.set_pinned(keep.clone());
        let (mut files, mut bytes) = (0, 0);
        for v in videos {
            if keep.contains(v) {
                continue;
            }
            let freed = cache.remove(v);
            if freed > 0 {
                files += 1;
                bytes += freed;
            }
            self.precache.forget(v);
        }
        self.refresh_downloads();
        (files, bytes)
    }

    /// Próxima canción de una playlist descargada que falta bajar.
    pub(crate) fn next_offline_track(&self, skip: &dyn Fn(&str) -> bool) -> Option<Track> {
        let cache = self.provider.as_ref()?.audio_cache()?;
        self.playlists
            .iter()
            .filter(|(p, _)| self.offline.contains(&p.id))
            .flat_map(|(p, _)| p.tracks.iter())
            .find(|t| {
                let key = cache_key(t);
                if skip(&key) {
                    return false;
                }
                match self.resolutions.get(&key) {
                    Some(TrackRes::Done(m)) => !cache.contains(&m.youtube_id),
                    Some(TrackRes::Resolving) | Some(TrackRes::Failed(_)) => false,
                    None => true,
                }
            })
            .cloned()
    }

    /// Canciones bajadas y total de una playlist descargada.
    pub(crate) fn offline_progress(&self, p: usize) -> Option<(usize, usize)> {
        let (pl, _) = self.playlists.get(p)?;
        if !self.offline.contains(&pl.id) {
            return None;
        }
        let done = self.downloads.get(&pl.id).map_or(0, |(files, _)| *files);
        let total = pl
            .tracks
            .iter()
            .map(cache_key)
            .collect::<HashSet<_>>()
            .len();
        Some((done.min(total), total))
    }
}
