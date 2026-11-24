//! Lo que se guarda entre sesiones: playlists, matches, historial y ajustes
//! (volumen, shuffle, repetir, playlist abierta). Todo pasa por
//! `surco_storage::Store`; si la base no se puede abrir, la app sigue sin
//! guardar nada.

use std::path::PathBuf;

use surco_core::{Playlist, RepeatMode};
use surco_player::{Command, Source};
use surco_resolver::{cache_key, Match};
use surco_storage::Store;

use crate::playback::TrackRes;
use crate::Surco;

const VOLUME: &str = "volume";
const SHUFFLE: &str = "shuffle";
const REPEAT: &str = "repeat";
const SELECTED: &str = "selected_playlist";

/// Ruta de la base: `<datos>/surco.db`. `SURCO_DB` la cambia (tests, perfiles).
pub fn db_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SURCO_DB") {
        return Some(PathBuf::from(p));
    }
    crate::data_dir().map(|d| d.join("surco.db"))
}

pub fn open() -> Option<Store> {
    let path = db_path()?;
    match Store::open(&path) {
        Ok(s) => {
            tracing::info!(path = %path.display(), "biblioteca abierta");
            Some(s)
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "no se pudo abrir la biblioteca");
            None
        }
    }
}

fn log_err<T>(what: &str, r: surco_storage::Result<T>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(error = %e, "{what}");
            None
        }
    }
}

impl Surco {
    /// Carga lo guardado al arrancar.
    pub(crate) fn restore(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        let playlists = log_err("leer playlists", store.playlists()).unwrap_or_default();
        let matches = log_err("leer matches", store.matches()).unwrap_or_default();
        let volume = log_err("leer ajustes", store.setting(VOLUME))
            .flatten()
            .and_then(|v| v.parse::<f32>().ok());
        let shuffle = log_err("leer ajustes", store.setting(SHUFFLE)).flatten();
        let repeat = log_err("leer ajustes", store.setting(REPEAT)).flatten();
        let selected = log_err("leer ajustes", store.setting(SELECTED)).flatten();

        tracing::info!(
            playlists = playlists.len(),
            matches = matches.len(),
            "biblioteca cargada"
        );
        self.playlists = playlists
            .into_iter()
            .map(|p| (p.playlist, p.source))
            .collect();
        self.selected = selected.and_then(|id| self.playlists.iter().position(|(p, _)| p.id == id));
        for (key, m) in &matches {
            self.resolutions
                .insert(key.clone(), TrackRes::Done(m.clone()));
        }
        if let Some(r) = &self.resolver {
            r.seed(matches);
        }
        if let Some(v) = volume {
            let v = v.clamp(0.0, 1.0);
            self.status.volume = v;
            self.send(Command::SetVolume(v));
        }
        self.shuffle = shuffle.as_deref() == Some("1");
        self.repeat = match repeat.as_deref() {
            Some("all") => RepeatMode::All,
            Some("one") => RepeatMode::One,
            _ => RepeatMode::Off,
        };
    }

    /// Le pasa al resolver (nuevo, p. ej. después de bajar yt-dlp) los matches conocidos.
    pub(crate) fn seed_resolver(&self) {
        let Some(r) = &self.resolver else {
            return;
        };
        r.seed(self.resolutions.iter().filter_map(|(k, v)| match v {
            TrackRes::Done(m) => Some((k.clone(), m.clone())),
            _ => None,
        }));
    }

    pub(crate) fn persist_playlist(&mut self, playlist: &Playlist, source: &str) {
        if let Some(store) = &mut self.store {
            log_err("guardar playlist", store.save_playlist(playlist, source));
        }
    }

    pub(crate) fn persist_match(&self, key: &str, m: &Match) {
        if let Some(store) = &self.store {
            log_err("guardar match", store.save_match(key, m));
        }
    }

    pub(crate) fn persist_volume(&self) {
        self.set(VOLUME, &format!("{:.2}", self.status.volume));
    }

    pub(crate) fn persist_modes(&self) {
        self.set(SHUFFLE, if self.shuffle { "1" } else { "0" });
        self.set(
            REPEAT,
            match self.repeat {
                RepeatMode::Off => "off",
                RepeatMode::All => "all",
                RepeatMode::One => "one",
            },
        );
    }

    pub(crate) fn persist_selected(&self) {
        let id = self
            .selected
            .and_then(|i| self.playlists.get(i))
            .map(|(p, _)| p.id.as_str())
            .unwrap_or("");
        self.set(SELECTED, id);
    }

    fn set(&self, key: &str, value: &str) {
        if let Some(store) = &self.store {
            log_err("guardar ajuste", store.set_setting(key, value));
        }
    }

    /// Anota en el historial la canción que acaba de empezar a sonar.
    pub(crate) fn record_history(&mut self) {
        let source = self.status.source.clone();
        if source.is_none() || source == self.recorded {
            return;
        }
        self.recorded = source.clone();
        let Some(store) = &self.store else {
            return;
        };
        let Some(track) = self.now_track() else {
            return;
        };
        // Solo si lo que suena es la canción actual de la cola.
        let video = match self.resolutions.get(&cache_key(track)) {
            Some(TrackRes::Done(m)) => Some(m.youtube_id.as_str()),
            _ => None,
        };
        if source != video.map(|v| Source::YouTube(v.to_owned())) {
            return;
        }
        let playlist_id = self
            .active
            .as_ref()
            .and_then(|a| a.origin)
            .and_then(|p| self.playlists.get(p))
            .map(|(p, _)| p.id.as_str());
        log_err(
            "guardar historial",
            store.record_play(track, playlist_id, video),
        );
    }

    /// Saca una playlist de la biblioteca.
    pub(crate) fn remove_playlist(&mut self, index: usize) {
        if index >= self.playlists.len() {
            return;
        }
        let (removed, _) = self.playlists.remove(index);
        if let Some(store) = &mut self.store {
            log_err("borrar playlist", store.delete_playlist(&removed.id));
        }
        // Los índices de las que siguen se corren uno.
        // La cola sigue sonando; solo deja de apuntar a esa playlist.
        if let Some(a) = &mut self.active {
            a.origin = match a.origin {
                Some(p) if p == index => None,
                Some(p) if p > index => Some(p - 1),
                other => other,
            };
        }
        if let Some((p, i)) = self.choosing {
            self.choosing = match p.cmp(&index) {
                std::cmp::Ordering::Less => Some((p, i)),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some((p - 1, i)),
            };
        }
        self.selected = None;
        self.filter = None;
        self.persist_selected();
    }
}
