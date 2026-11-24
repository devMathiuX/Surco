//! Persistencia en SQLite: biblioteca (playlists importadas), caché de
//! matches Spotify → YouTube, historial de reproducción y ajustes chicos
//! (volumen, shuffle, repetir…).
//!
//! Un solo archivo `surco.db` en la carpeta de datos de la app. El esquema se
//! versiona con `PRAGMA user_version` y se migra al abrir.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use surco_core::{Playlist, Track};
use surco_resolver::Match;

pub use surco_core as core;

/// Errores de este crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de datos: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("datos guardados ilegibles: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no se pudo crear la carpeta de datos: {0}")]
    Io(#[from] std::io::Error),
    #[error("la base de datos es de una versión más nueva de Surco (esquema {0})")]
    TooNew(i64),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Versión actual del esquema.
const SCHEMA_VERSION: i64 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE playlists (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    source      TEXT NOT NULL,
    position    INTEGER NOT NULL,
    imported_at INTEGER NOT NULL
);
CREATE TABLE tracks (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    title       TEXT NOT NULL,
    artists     TEXT NOT NULL,      -- JSON: [\"a\", \"b\"]
    album       TEXT,
    duration_ms INTEGER NOT NULL,
    isrc        TEXT,
    spotify_id  TEXT,
    PRIMARY KEY (playlist_id, idx)
);
CREATE TABLE matches (
    key        TEXT PRIMARY KEY,    -- surco_resolver::cache_key
    youtube_id TEXT NOT NULL,
    score      INTEGER NOT NULL,
    manual     INTEGER NOT NULL,
    data       TEXT NOT NULL,       -- JSON del Match completo (con candidatos)
    updated_at INTEGER NOT NULL
);
CREATE TABLE history (
    id          INTEGER PRIMARY KEY,
    played_at   INTEGER NOT NULL,
    playlist_id TEXT,
    title       TEXT NOT NULL,
    artists     TEXT NOT NULL,
    youtube_id  TEXT
);
CREATE INDEX history_played_at ON history(played_at);
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Esquema 2: el historial guarda la canción completa, para volver a ponerla
/// desde "Escuchadas hace poco" con el mismo video elegido.
const SCHEMA_V2: &str = "
ALTER TABLE history ADD COLUMN album TEXT;
ALTER TABLE history ADD COLUMN duration_ms INTEGER;
ALTER TABLE history ADD COLUMN spotify_id TEXT;
";

/// Cuántas entradas de historial se guardan como máximo.
const HISTORY_LIMIT: i64 = 5_000;

/// Una playlist guardada, con la estrategia con la que se importó.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPlaylist {
    pub playlist: Playlist,
    pub source: String,
}

/// Una entrada del historial.
#[derive(Debug, Clone, PartialEq)]
pub struct Played {
    /// Segundos desde la época Unix.
    pub played_at: i64,
    pub playlist_id: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub youtube_id: Option<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub spotify_id: Option<String>,
}

impl Played {
    /// La canción tal como sonó (misma clave de caché que la original).
    pub fn track(&self) -> Track {
        Track {
            title: self.title.clone(),
            artists: self.artists.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            isrc: None,
            spotify_id: self.spotify_id.clone(),
        }
    }
}

pub struct Store {
    conn: Connection,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Store {
    /// Abre (o crea) la base en `path` y la migra al esquema actual.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // WAL: las escrituras no bloquean lecturas y sobrevive a un corte.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// Base en memoria (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::TooNew(version));
        }
        if version < 1 {
            conn.execute_batch(&format!(
                "BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"
            ))?;
            tracing::info!("base de datos creada (esquema 1)");
        }
        if version < 2 {
            conn.execute_batch(&format!(
                "BEGIN; {SCHEMA_V2} PRAGMA user_version = 2; COMMIT;"
            ))?;
            tracing::info!("base de datos migrada al esquema 2");
        }
        Ok(Self { conn })
    }

    // ── Biblioteca ──────────────────────────────────────────────────────────

    /// Guarda una playlist (si ya existía, la reemplaza manteniendo su lugar).
    pub fn save_playlist(&mut self, playlist: &Playlist, source: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        let position: i64 = match tx
            .query_row(
                "SELECT position FROM playlists WHERE id = ?1",
                [&playlist.id],
                |r| r.get(0),
            )
            .optional()?
        {
            Some(p) => p,
            None => tx.query_row(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM playlists",
                [],
                |r| r.get(0),
            )?,
        };
        tx.execute("DELETE FROM playlists WHERE id = ?1", [&playlist.id])?;
        tx.execute(
            "INSERT INTO playlists (id, name, source, position, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![playlist.id, playlist.name, source, position, now()],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO tracks
                 (playlist_id, idx, title, artists, album, duration_ms, isrc, spotify_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for (i, t) in playlist.tracks.iter().enumerate() {
                stmt.execute(params![
                    playlist.id,
                    i as i64,
                    t.title,
                    serde_json::to_string(&t.artists)?,
                    t.album,
                    t.duration_ms as i64,
                    t.isrc,
                    t.spotify_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Todas las playlists, en el orden en que se importaron.
    pub fn playlists(&self) -> Result<Vec<StoredPlaylist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, source FROM playlists ORDER BY position")?;
        let heads = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut tracks = self.conn.prepare(
            "SELECT title, artists, album, duration_ms, isrc, spotify_id
             FROM tracks WHERE playlist_id = ?1 ORDER BY idx",
        )?;
        let mut out = Vec::with_capacity(heads.len());
        for (id, name, source) in heads {
            let rows = tracks
                .query_map([&id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut list = Vec::with_capacity(rows.len());
            for (title, artists, album, duration_ms, isrc, spotify_id) in rows {
                list.push(Track {
                    title,
                    artists: serde_json::from_str(&artists)?,
                    album,
                    duration_ms: duration_ms.max(0) as u64,
                    isrc,
                    spotify_id,
                });
            }
            out.push(StoredPlaylist {
                playlist: Playlist {
                    id,
                    name,
                    tracks: list,
                },
                source,
            });
        }
        Ok(out)
    }

    /// Borra una playlist (los matches quedan: sirven si se vuelve a importar).
    pub fn delete_playlist(&mut self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Cambia el nombre de una playlist.
    pub fn rename_playlist(&self, id: &str, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE playlists SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// Cambia el ID de una playlist (la radio pasa a ser una playlist común).
    pub fn change_playlist_id(&mut self, old: &str, new: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("PRAGMA defer_foreign_keys = ON", [])?;
        tx.execute(
            "UPDATE playlists SET id = ?2 WHERE id = ?1",
            params![old, new],
        )?;
        tx.execute(
            "UPDATE tracks SET playlist_id = ?2 WHERE playlist_id = ?1",
            params![old, new],
        )?;
        tx.execute(
            "UPDATE history SET playlist_id = ?2 WHERE playlist_id = ?1",
            params![old, new],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Guarda el orden de la barra lateral: `ids` en el orden nuevo.
    pub fn set_playlist_order(&mut self, ids: &[&str]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for (i, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE playlists SET position = ?2 WHERE id = ?1",
                params![id, i as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// IDs de las playlists escuchadas hace poco, la más reciente primero.
    pub fn recent_playlists(&self, n: usize) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.playlist_id FROM history h
             JOIN playlists p ON p.id = h.playlist_id
             GROUP BY h.playlist_id ORDER BY MAX(h.id) DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([n as i64], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ── Matches ─────────────────────────────────────────────────────────────

    /// Guarda (o reemplaza) el match de una canción.
    pub fn save_match(&self, key: &str, m: &Match) -> Result<()> {
        self.conn.execute(
            "INSERT INTO matches (key, youtube_id, score, manual, data, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(key) DO UPDATE SET
               youtube_id = excluded.youtube_id, score = excluded.score,
               manual = excluded.manual, data = excluded.data,
               updated_at = excluded.updated_at",
            params![
                key,
                m.youtube_id,
                m.score,
                m.manual,
                serde_json::to_string(m)?,
                now()
            ],
        )?;
        Ok(())
    }

    /// Todos los matches guardados. Las filas ilegibles se saltean.
    pub fn matches(&self) -> Result<Vec<(String, Match)>> {
        let mut stmt = self.conn.prepare("SELECT key, data FROM matches")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(k, data)| match serde_json::from_str(&data) {
                Ok(m) => Some((k, m)),
                Err(e) => {
                    tracing::warn!(key = %k, error = %e, "match guardado ilegible");
                    None
                }
            })
            .collect())
    }

    // ── Historial ───────────────────────────────────────────────────────────

    /// Anota que empezó a sonar una canción.
    pub fn record_play(
        &self,
        track: &Track,
        playlist_id: Option<&str>,
        youtube_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO history
             (played_at, playlist_id, title, artists, youtube_id, album, duration_ms, spotify_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now(),
                playlist_id,
                track.title,
                serde_json::to_string(&track.artists)?,
                youtube_id,
                track.album,
                track.duration_ms as i64,
                track.spotify_id,
            ],
        )?;
        // Que no crezca sin límite.
        self.conn.execute(
            "DELETE FROM history WHERE id <= (SELECT MAX(id) FROM history) - ?1",
            [HISTORY_LIMIT],
        )?;
        Ok(())
    }

    /// Las últimas `n` reproducciones, la más reciente primero.
    pub fn recent(&self, n: usize) -> Result<Vec<Played>> {
        let mut stmt = self.conn.prepare(
            "SELECT played_at, playlist_id, title, artists, youtube_id, album,
                    duration_ms, spotify_id
             FROM history ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([n as i64], |r| {
                Ok((
                    (
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ),
                    (
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<i64>>(6)?,
                        r.get::<_, Option<String>>(7)?,
                    ),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(
                    (played_at, playlist_id, title, artists),
                    (youtube_id, album, ms, spotify_id),
                )| {
                    Ok(Played {
                        played_at,
                        playlist_id,
                        title,
                        artists: serde_json::from_str(&artists)?,
                        youtube_id,
                        album,
                        duration_ms: ms.unwrap_or(0).max(0) as u64,
                        spotify_id,
                    })
                },
            )
            .collect()
    }

    // ── Ajustes ─────────────────────────────────────────────────────────────

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, id: &str) -> Track {
        Track {
            title: title.into(),
            artists: vec!["Soda Stereo".into(), "Gustavo Cerati".into()],
            album: Some("Canción Animal".into()),
            duration_ms: 215_000,
            isrc: Some("ARX000000001".into()),
            spotify_id: Some(id.into()),
        }
    }

    fn playlist(id: &str, n: usize) -> Playlist {
        Playlist {
            id: id.into(),
            name: format!("Lista {id}"),
            tracks: (0..n)
                .map(|i| track(&format!("Tema {i}"), &format!("s{i}")))
                .collect(),
        }
    }

    fn a_match(id: &str) -> Match {
        Match {
            youtube_id: id.into(),
            score: 87,
            manual: false,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn playlists_round_trip_in_order() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        s.save_playlist(&playlist("b", 3), "CSV")?;
        s.save_playlist(&playlist("a", 120), "página embed")?;
        let got = s.playlists()?;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].playlist, playlist("b", 3));
        assert_eq!(got[0].source, "CSV");
        assert_eq!(got[1].playlist.tracks.len(), 120);
        assert_eq!(got[1].playlist.tracks[7], track("Tema 7", "s7"));
        Ok(())
    }

    #[test]
    fn reimport_replaces_and_keeps_position() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        s.save_playlist(&playlist("a", 5), "CSV")?;
        s.save_playlist(&playlist("b", 5), "CSV")?;
        s.save_playlist(&playlist("a", 2), "Web API de Spotify")?;
        let got = s.playlists()?;
        assert_eq!(got[0].playlist.id, "a");
        assert_eq!(got[0].playlist.tracks.len(), 2);
        assert_eq!(got[0].source, "Web API de Spotify");
        s.delete_playlist("a")?;
        assert_eq!(s.playlists()?.len(), 1);
        Ok(())
    }

    #[test]
    fn matches_round_trip_and_update() -> Result<()> {
        let s = Store::open_in_memory()?;
        s.save_match("s1", &a_match("aaaaaaaaaaa"))?;
        let mut manual = a_match("bbbbbbbbbbb");
        manual.manual = true;
        s.save_match("s1", &manual)?;
        s.save_match("s2", &a_match("ccccccccccc"))?;
        let mut got = s.matches()?;
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![("s1".into(), manual), ("s2".into(), a_match("ccccccccccc"))]
        );
        Ok(())
    }

    #[test]
    fn history_is_newest_first_and_capped() -> Result<()> {
        let s = Store::open_in_memory()?;
        for i in 0..3 {
            s.record_play(&track(&format!("T{i}"), "x"), Some("p"), Some("vid"))?;
        }
        let recent = s.recent(2)?;
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].title, "T2");
        assert_eq!(recent[1].title, "T1");
        assert_eq!(recent[0].artists.len(), 2);
        Ok(())
    }

    #[test]
    fn history_keeps_the_whole_track() -> Result<()> {
        let s = Store::open_in_memory()?;
        let t = track("Persiana Americana", "s9");
        s.record_play(&t, None, Some("vid"))?;
        assert_eq!(s.recent(1)?[0].track(), Track { isrc: None, ..t });
        Ok(())
    }

    #[test]
    fn rename_reorder_and_recent() -> Result<()> {
        let mut s = Store::open_in_memory()?;
        for id in ["a", "b", "radio"] {
            s.save_playlist(&playlist(id, 2), "CSV")?;
        }
        s.rename_playlist("a", "Mía")?;
        s.set_playlist_order(&["radio", "a", "b"])?;
        s.record_play(&track("x", "1"), Some("b"), None)?;
        s.record_play(&track("y", "2"), Some("radio"), None)?;
        s.record_play(&track("z", "3"), Some("gone"), None)?;
        s.change_playlist_id("radio", "user:1")?;
        let got = s.playlists()?;
        let ids: Vec<_> = got.iter().map(|p| p.playlist.id.as_str()).collect();
        assert_eq!(ids, ["user:1", "a", "b"]);
        assert_eq!(got[1].playlist.name, "Mía");
        assert_eq!(got[0].playlist.tracks.len(), 2);
        assert_eq!(s.recent_playlists(5)?, ["user:1", "b"]);
        Ok(())
    }

    #[test]
    fn migrates_schema_1() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(&format!("{SCHEMA_V1} PRAGMA user_version = 1;"))?;
        conn.execute(
            "INSERT INTO history (played_at, title, artists) VALUES (1, 'T', '[\"A\"]')",
            [],
        )?;
        let s = Store::init(conn)?;
        let old = &s.recent(1)?[0];
        assert_eq!((old.title.as_str(), old.duration_ms), ("T", 0));
        Ok(())
    }

    #[test]
    fn settings() -> Result<()> {
        let s = Store::open_in_memory()?;
        assert_eq!(s.setting("volume")?, None);
        s.set_setting("volume", "0.5")?;
        s.set_setting("volume", "0.7")?;
        assert_eq!(s.setting("volume")?.as_deref(), Some("0.7"));
        Ok(())
    }

    #[test]
    fn reopen_keeps_data_and_rejects_newer_schema() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("surco-store-{}", std::process::id()));
        let path = dir.join("surco.db");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut s = Store::open(&path)?;
            s.save_playlist(&playlist("a", 4), "CSV")?;
            s.save_match("s0", &a_match("ddddddddddd"))?;
        }
        {
            let s = Store::open(&path)?;
            assert_eq!(s.playlists()?.len(), 1);
            assert_eq!(s.matches()?.len(), 1);
            s.conn.pragma_update(None, "user_version", 99)?;
        }
        assert!(matches!(Store::open(&path), Err(Error::TooNew(99))));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
