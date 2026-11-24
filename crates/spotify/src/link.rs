//! Parser de links de playlists de Spotify.

/// Extrae el ID de una playlist de Spotify desde lo que pegue el usuario.
///
/// Acepta:
/// - `https://open.spotify.com/playlist/{id}` (con o sin `?si=…`, `#…`, `/` final)
/// - variantes con idioma (`/intl-es/playlist/{id}`) o embed (`/embed/playlist/{id}`)
/// - `spotify:playlist:{id}` y el formato viejo `spotify:user:{u}:playlist:{id}`
/// - el ID solo (22 caracteres base62)
pub fn parse_playlist_id(input: &str) -> Option<String> {
    let s = input.trim();

    if let Some(rest) = s.strip_prefix("spotify:") {
        let parts: Vec<&str> = rest.split(':').collect();
        return match parts.as_slice() {
            ["playlist", id] | ["user", _, "playlist", id] => valid_id(id),
            _ => None,
        };
    }

    let without_scheme = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    if let Some(path) = without_scheme
        .strip_prefix("open.spotify.com/")
        .or_else(|| without_scheme.strip_prefix("play.spotify.com/"))
    {
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let mut segments = path.split('/').filter(|p| !p.is_empty());
        // Buscamos el segmento que sigue a "playlist", sin importar prefijos
        // como "intl-es", "embed" o "user/{u}".
        while let Some(seg) = segments.next() {
            if seg == "playlist" {
                return segments.next().and_then(valid_id);
            }
        }
        return None;
    }

    valid_id(s)
}

/// Los IDs de Spotify son 22 caracteres base62.
fn valid_id(id: &str) -> Option<String> {
    (id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then(|| id.to_owned())
}

/// ID de una canción a partir de `spotify:track:{id}` o de un link.
pub(crate) fn track_id(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if let Some(id) = uri.strip_prefix("spotify:track:") {
        return valid_id(id);
    }
    let path = uri.split(['?', '#']).next().unwrap_or_default();
    path.rsplit_once("/track/").and_then(|(_, id)| valid_id(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "37i9dQZF1DXcBWIGoYBM5M";

    #[test]
    fn accepts_all_formats() {
        for input in [
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=a1b2c3d4e5f6",
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=x&pi=y#frag",
            "  https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M/  ",
            "http://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/intl-es/playlist/37i9dQZF1DXcBWIGoYBM5M?si=1",
            "https://open.spotify.com/embed/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/user/spotify/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "spotify:user:spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "37i9dQZF1DXcBWIGoYBM5M",
        ] {
            assert_eq!(parse_playlist_id(input).as_deref(), Some(ID), "{input}");
        }
    }

    #[test]
    fn rejects_other_things() {
        for input in [
            "",
            "hola",
            "https://open.spotify.com/album/37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/track/37i9dQZF1DXcBWIGoYBM5M",
            "spotify:album:37i9dQZF1DXcBWIGoYBM5M",
            "https://open.spotify.com/playlist/",
            "https://open.spotify.com/playlist/corto",
            "https://example.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "37i9dQZF1DXcBWIGoYBM5M!",
        ] {
            assert_eq!(parse_playlist_id(input), None, "{input}");
        }
    }

    #[test]
    fn parses_track_ids() {
        assert_eq!(
            track_id("spotify:track:4HCOHRCSQelz2GPX8iTamO").as_deref(),
            Some("4HCOHRCSQelz2GPX8iTamO")
        );
        assert_eq!(
            track_id("https://open.spotify.com/track/4HCOHRCSQelz2GPX8iTamO?si=1").as_deref(),
            Some("4HCOHRCSQelz2GPX8iTamO")
        );
        assert_eq!(track_id("spotify:local:a:b:c:1"), None);
    }
}
