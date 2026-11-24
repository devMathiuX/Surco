//! Estrategia 3: importar un CSV (formato de Exportify, viejo y nuevo).
//!
//! Las columnas se buscan por nombre, sin importar el orden, así que también
//! sirve un CSV armado a mano con `title,artist,album,duration`.

use std::path::Path;

use surco_core::{Playlist, Track};

use crate::{link, Error, Imported, Result};

/// Nombres de columna aceptados para cada campo (en minúsculas).
const TITLE: &[&str] = &["track name", "title", "name", "título", "titulo", "song"];
const ARTISTS: &[&str] = &[
    "artist name(s)",
    "artist names",
    "artists",
    "artist",
    "artista",
    "artistas",
];
const ALBUM: &[&str] = &["album name", "album", "álbum"];
const DURATION: &[&str] = &[
    "track duration (ms)",
    "duration (ms)",
    "duration_ms",
    "duration",
    "duración",
    "duracion",
];
const ISRC: &[&str] = &["isrc"];
const URI: &[&str] = &["track uri", "uri", "spotify uri", "spotify_id", "id"];

/// Lee un CSV de disco. El nombre de la playlist es el del archivo.
pub fn import_csv_file(path: &Path) -> Result<Imported> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_else(|| "Playlist importada".into());
    import_csv(&text, &name)
}

/// Parsea el contenido de un CSV.
pub fn import_csv(text: &str, name: &str) -> Result<Imported> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = parse_rows(text).into_iter();
    let header: Vec<String> = rows
        .next()
        .ok_or_else(|| Error::Parse("el CSV está vacío".into()))?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let col = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| header.iter().position(|h| h == n))
    };

    let title_col = col(TITLE)
        .ok_or_else(|| Error::Parse("el CSV no tiene columna de título (\"Track Name\")".into()))?;
    let artists_col = col(ARTISTS).ok_or_else(|| {
        Error::Parse("el CSV no tiene columna de artistas (\"Artist Name(s)\")".into())
    })?;
    let (album_col, duration_col, isrc_col, uri_col) =
        (col(ALBUM), col(DURATION), col(ISRC), col(URI));

    let mut tracks = Vec::new();
    let mut skipped = 0usize;
    for row in rows {
        if row.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let get = |i: Option<usize>| {
            i.and_then(|i| row.get(i))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        };
        let Some(title) = get(Some(title_col)) else {
            skipped += 1;
            continue;
        };
        tracks.push(Track {
            title: title.to_owned(),
            artists: split_artists(get(Some(artists_col)).unwrap_or_default()),
            album: get(album_col).map(str::to_owned),
            duration_ms: get(duration_col).and_then(parse_duration).unwrap_or(0),
            isrc: get(isrc_col).map(str::to_owned),
            spotify_id: get(uri_col).and_then(link::track_id),
        });
    }
    if tracks.is_empty() {
        return Err(Error::Parse("el CSV no tiene canciones".into()));
    }

    let mut warnings = Vec::new();
    if skipped > 0 {
        warnings.push(format!("Se saltearon {skipped} filas sin título."));
    }
    Ok(Imported {
        playlist: Playlist {
            id: format!("csv-{:016x}", fnv1a(text.as_bytes())),
            name: name.trim().to_owned(),
            tracks,
        },
        source: "CSV",
        warnings,
    })
}

/// Exportify separa artistas con coma (o con `;` en versiones viejas).
fn split_artists(s: &str) -> Vec<String> {
    let sep = if s.contains(';') { ';' } else { ',' };
    s.split(sep)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Milisegundos (`212000`) o `m:ss` / `h:mm:ss`.
fn parse_duration(s: &str) -> Option<u64> {
    if s.contains(':') {
        s.split(':')
            .try_fold(0u64, |acc, p| {
                Some(acc * 60 + p.trim().parse::<u64>().ok()?)
            })
            .map(|secs| secs * 1000)
    } else {
        s.parse::<f64>()
            .ok()
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(|d| d as u64)
    }
}

/// Parser CSV mínimo (RFC 4180): comillas, comillas dobles escapadas,
/// saltos de línea dentro de campos, CRLF.
fn parse_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => in_quotes = false,
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' => in_quotes = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Hash estable (FNV-1a de 64 bits) para el ID de una playlist CSV.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_and_newlines() {
        let rows = parse_rows("a,\"b, c\",\"d \"\"e\"\"\"\r\n\"multi\nline\",x,\n");
        assert_eq!(rows[0], ["a", "b, c", "d \"e\""]);
        assert_eq!(rows[1], ["multi\nline", "x", ""]);
    }

    #[test]
    fn parses_old_exportify() {
        let csv = "\u{feff}\"Track URI\",\"Track Name\",\"Artist URI(s)\",\"Artist Name(s)\",\"Album URI\",\"Album Name\",\"Track Duration (ms)\",\"ISRC\"\n\
            \"spotify:track:4HCOHRCSQelz2GPX8iTamO\",\"Under Pressure - Remastered 2011\",\"x\",\"Queen;David Bowie\",\"y\",\"Hot Space\",\"248440\",\"GBUM71029604\"\n";
        let imp = import_csv(csv, "Mis temas").unwrap();
        let t = &imp.playlist.tracks[0];
        assert_eq!(t.title, "Under Pressure - Remastered 2011");
        assert_eq!(t.artists, ["Queen", "David Bowie"]);
        assert_eq!(t.album.as_deref(), Some("Hot Space"));
        assert_eq!(t.duration_ms, 248_440);
        assert_eq!(t.isrc.as_deref(), Some("GBUM71029604"));
        assert_eq!(t.spotify_id.as_deref(), Some("4HCOHRCSQelz2GPX8iTamO"));
        assert_eq!(imp.playlist.name, "Mis temas");
        assert!(imp.playlist.id.starts_with("csv-"));
    }

    #[test]
    fn parses_new_exportify_and_hand_made() {
        let csv = "Track URI,Track Name,Album Name,Artist Name(s),Release Date,Duration (ms)\n\
            spotify:track:4HCOHRCSQelz2GPX8iTamO,Song,Album,\"A,B\",2020,1000\n";
        let t = &import_csv(csv, "x").unwrap().playlist.tracks[0];
        assert_eq!(t.artists, ["A", "B"]);
        assert_eq!(t.duration_ms, 1000);
        assert_eq!(t.isrc, None);

        let csv = "title,artist,duration\nDe música ligera,Soda Stereo,3:32\n,nadie,1:00\n\n";
        let imp = import_csv(csv, "x").unwrap();
        assert_eq!(imp.playlist.tracks.len(), 1);
        assert_eq!(imp.playlist.tracks[0].duration_ms, 212_000);
        assert_eq!(imp.warnings.len(), 1);
    }

    #[test]
    fn rejects_bad_csv() {
        assert!(import_csv("", "x").is_err());
        assert!(import_csv("foo,bar\n1,2\n", "x").is_err());
        assert!(import_csv("title,artist\n", "x").is_err());
    }

    #[test]
    fn same_content_same_id() {
        let a = import_csv("title,artist\nx,y\n", "a").unwrap();
        let b = import_csv("title,artist\nx,y\n", "b").unwrap();
        assert_eq!(a.playlist.id, b.playlist.id);
    }
}
