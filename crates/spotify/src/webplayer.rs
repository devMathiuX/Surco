//! Estrategia 1: lo mismo que hace open.spotify.com sin sesión iniciada.
//!
//! La página embed (`open.spotify.com/embed/playlist/{id}`) trae un token
//! anónimo (`"accessToken":"BQ…"`). Con ese token, la API interna del
//! reproductor web (`api-partner.spotify.com/pathfinder/v1/query`, GraphQL con
//! consultas persistidas) devuelve la playlist **entera**, de a páginas. No hace
//! falta cuenta, Premium ni claves de desarrollador.
//!
//! **No es una API pública**: Spotify puede cambiarla sin aviso. Defensas:
//! - Si el hash de la consulta (`fetchPlaylist`) deja de existir, lo buscamos
//!   en el JavaScript actual del reproductor web y reintentamos.
//! - Si igual falla, la cascada sigue con las claves propias (si hay), la
//!   página embed (100 canciones) o el CSV.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use surco_core::{Playlist, Track};
use tokio::sync::Mutex;

use crate::{link, Error, Imported, PlaylistSource, Result};

/// Canciones por página. Si Spotify devuelve menos, se sigue desde donde quedó.
const PAGE: u64 = 300;
/// Tope de páginas para no colgarse con una respuesta rara.
const MAX_PAGES: u64 = 200;
/// Reintentos ante HTTP 429.
const MAX_RETRIES: u32 = 3;
/// Tope de espera por un `Retry-After`.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);
/// Margen antes del vencimiento del token para pedir otro.
const TOKEN_MARGIN: Duration = Duration::from_secs(60);

/// Hashes conocidos de la consulta `fetchPlaylist`, el más nuevo primero.
const KNOWN_HASHES: [&str; 2] = [
    "21ea11a6ac75a5f3e01fd365d29c18e0a4a8b7dc09f89bbb5d73910190c6acaf",
    "19ff1327c29e99c208c86d7a9d8f1929cfdf3d3202a0ff4253c821f1901aa94d",
];

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";

/// Fuente que lee la playlist como el reproductor web, sin sesión.
#[derive(Debug)]
pub struct WebPlayer {
    client: reqwest::Client,
    /// `https://open.spotify.com` (página embed y página principal).
    open_base: String,
    /// URL de la API GraphQL del reproductor.
    query_url: String,
    token: Mutex<Option<(String, Instant)>>,
    /// Hash encontrado en el JavaScript del reproductor (si hizo falta buscarlo).
    found_hash: Mutex<Option<String>>,
}

/// Una página de la playlist.
struct Page {
    name: String,
    total: u64,
    tracks: Vec<Track>,
    items: u64,
}

/// Lo que puede pasar con una consulta.
enum Query {
    Ok(Value),
    /// El hash ya no existe o pide otras variables: probar con otro.
    BadHash,
}

impl WebPlayer {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_endpoints(
            client,
            "https://open.spotify.com",
            "https://api-partner.spotify.com/pathfinder/v1/query",
        )
    }

    /// Para tests: otras URLs.
    pub fn with_endpoints(
        client: reqwest::Client,
        open_base: impl Into<String>,
        query_url: impl Into<String>,
    ) -> Self {
        Self {
            client,
            open_base: open_base.into(),
            query_url: query_url.into(),
            token: Mutex::new(None),
            found_hash: Mutex::new(None),
        }
    }

    /// Token anónimo, sacado de la página embed de la playlist.
    async fn token(&self, id: &str, force: bool) -> Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((tok, expires)) = guard.as_ref() {
            if !force && Instant::now() < *expires {
                return Ok(tok.clone());
            }
        }
        let resp = self
            .client
            .get(format!("{}/embed/playlist/{id}", self.open_base))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await?;
        match resp.status().as_u16() {
            200..=299 => {}
            404 => return Err(Error::NotFound),
            s => return Err(Error::Status(s)),
        }
        let html = resp.text().await?;
        let (tok, expires_ms) = parse_token(&html)
            .ok_or_else(|| Error::Parse("la página embed no trae el token anónimo".into()))?;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let left = expires_ms
            .map(|e| Duration::from_millis(e.saturating_sub(now_ms)))
            .unwrap_or(Duration::from_secs(600))
            .saturating_sub(TOKEN_MARGIN);
        *guard = Some((tok.clone(), Instant::now() + left));
        Ok(tok)
    }

    /// Una consulta `fetchPlaylist` con un hash, con renovación del token ante
    /// 401 y espera ante 429.
    async fn query(&self, id: &str, hash: &str, offset: u64) -> Result<Query> {
        let variables = json!({
            "uri": format!("spotify:playlist:{id}"),
            "offset": offset,
            "limit": PAGE,
            "enableWatchFeedEntrypoint": false,
        });
        let extensions = json!({"persistedQuery": {"version": 1, "sha256Hash": hash}});
        let mut retries = 0;
        let mut refreshed = false;
        loop {
            let tok = self.token(id, false).await?;
            let resp = self
                .client
                .get(&self.query_url)
                .query(&[
                    ("operationName", "fetchPlaylist"),
                    ("variables", &variables.to_string()),
                    ("extensions", &extensions.to_string()),
                ])
                .bearer_auth(&tok)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header("app-platform", "WebPlayer")
                .send()
                .await?;
            match resp.status().as_u16() {
                200..=299 => {
                    let v: Value = serde_json::from_str(&resp.text().await?)
                        .map_err(|e| Error::Parse(format!("respuesta del reproductor web: {e}")))?;
                    if v["data"]["playlistV2"].is_object() {
                        return Ok(Query::Ok(v));
                    }
                    if v["errors"].is_array() {
                        tracing::debug!(hash, errors = %v["errors"], "consulta rechazada");
                        return Ok(Query::BadHash);
                    }
                    return Err(Error::Parse(
                        "respuesta del reproductor web sin datos".into(),
                    ));
                }
                // Hash desconocido o variables que no coinciden.
                400 => return Ok(Query::BadHash),
                401 if !refreshed => {
                    refreshed = true;
                    self.token(id, true).await?;
                }
                429 if retries < MAX_RETRIES => {
                    retries += 1;
                    let wait = retry_after(resp.headers()).unwrap_or(Duration::from_secs(1));
                    if wait > MAX_RETRY_AFTER {
                        return Err(Error::RateLimited(wait));
                    }
                    tracing::warn!(?wait, retries, "Spotify devolvió 429, esperando");
                    tokio::time::sleep(wait).await;
                }
                429 => return Err(Error::RateLimited(Duration::ZERO)),
                s => return Err(Error::Status(s)),
            }
        }
    }

    /// Primera página: prueba los hashes conocidos y, si ninguno anda, busca
    /// el actual en el reproductor web. Devuelve el hash que funcionó.
    async fn first_page(&self, id: &str) -> Result<(String, Page)> {
        let mut hashes: Vec<String> = Vec::new();
        if let Some(h) = self.found_hash.lock().await.clone() {
            hashes.push(h);
        }
        hashes.extend(KNOWN_HASHES.iter().map(|h| (*h).to_owned()));
        for hash in &hashes {
            if let Query::Ok(v) = self.query(id, hash, 0).await? {
                return Ok((hash.clone(), parse_page(&v)?));
            }
        }
        let hash = self.discover_hash().await?;
        if hashes.contains(&hash) {
            return Err(Error::Parse(
                "el reproductor web rechazó la consulta".into(),
            ));
        }
        tracing::info!(%hash, "hash de fetchPlaylist actualizado");
        *self.found_hash.lock().await = Some(hash.clone());
        match self.query(id, &hash, 0).await? {
            Query::Ok(v) => Ok((hash, parse_page(&v)?)),
            Query::BadHash => Err(Error::Parse(
                "el reproductor web rechazó la consulta".into(),
            )),
        }
    }

    /// Busca el hash actual de `fetchPlaylist` en el JavaScript del reproductor.
    async fn discover_hash(&self) -> Result<String> {
        let home = self
            .client
            .get(format!("{}/", self.open_base))
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await?
            .text()
            .await?;
        let script = find_player_script(&home)
            .ok_or_else(|| Error::Parse("no se encontró el JavaScript del reproductor".into()))?;
        let js = self
            .client
            .get(script)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await?
            .text()
            .await?;
        find_query_hash(&js, "fetchPlaylist")
            .ok_or_else(|| Error::Parse("el reproductor no trae la consulta fetchPlaylist".into()))
    }

    async fn fetch_impl(&self, id: &str) -> Result<Imported> {
        let (hash, first) = self.first_page(id).await?;
        let Page {
            name,
            total,
            mut tracks,
            mut items,
        } = first;
        let mut offset = items;
        let mut pages = 1;
        while offset < total && pages < MAX_PAGES {
            let page = match self.query(id, &hash, offset).await? {
                Query::Ok(v) => parse_page(&v)?,
                Query::BadHash => {
                    return Err(Error::Parse(
                        "el reproductor web cortó la paginación".into(),
                    ))
                }
            };
            if page.items == 0 {
                break;
            }
            tracks.extend(page.tracks);
            items += page.items;
            offset += page.items;
            pages += 1;
        }
        let mut warnings = Vec::new();
        if items < total {
            warnings.push(format!(
                "Spotify devolvió {items} de {total} elementos; puede faltar alguna canción."
            ));
        }
        if tracks.is_empty() && total > 0 {
            return Err(Error::NoTracks);
        }
        Ok(Imported {
            playlist: Playlist {
                id: id.to_owned(),
                name,
                tracks,
            },
            source: self.name(),
            warnings,
        })
    }
}

impl PlaylistSource for WebPlayer {
    fn name(&self) -> &'static str {
        "reproductor web de Spotify"
    }

    fn fetch<'a>(
        &'a self,
        id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Imported>> + Send + 'a>> {
        Box::pin(self.fetch_impl(id))
    }
}

/// Token anónimo y su vencimiento (ms desde 1970) dentro del HTML de la
/// página embed.
pub fn parse_token(html: &str) -> Option<(String, Option<u64>)> {
    let tok = json_string_after(html, "\"accessToken\":\"")?;
    if tok.is_empty() {
        return None;
    }
    let expires = html
        .find("\"accessTokenExpirationTimestampMs\":")
        .map(|i| &html[i + "\"accessTokenExpirationTimestampMs\":".len()..])
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        });
    Some((tok, expires))
}

/// El texto que sigue a `marker` hasta la próxima comilla.
fn json_string_after(s: &str, marker: &str) -> Option<String> {
    let start = s.find(marker)? + marker.len();
    let len = s[start..].find('"')?;
    Some(s[start..start + len].to_owned())
}

/// URL del script principal del reproductor web en la página de inicio.
fn find_player_script(html: &str) -> Option<&str> {
    const NEEDLE: &str = "web-player/web-player.";
    let mut from = 0;
    while let Some(i) = html[from..].find(NEEDLE).map(|i| i + from) {
        let start = html[..i].rfind(['"', '\'']).map(|q| q + 1)?;
        let end = i + html[i..].find(['"', '\''])?;
        let url = &html[start..end];
        if url.ends_with(".js") && url.starts_with("http") {
            return Some(url);
        }
        from = end;
    }
    None
}

/// Hash de una consulta persistida: `"fetchPlaylist","query","<hash>"`.
fn find_query_hash(js: &str, operation: &str) -> Option<String> {
    let marker = format!("\"{operation}\",\"query\",\"");
    let hash = json_string_after(js, &marker)?;
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hash)
}

/// Una página de `fetchPlaylist`.
fn parse_page(v: &Value) -> Result<Page> {
    let p = &v["data"]["playlistV2"];
    match p["__typename"].as_str() {
        Some("NotFound") => return Err(Error::NotFound),
        Some("Playlist") | None => {}
        Some(other) => return Err(Error::Parse(format!("playlist no disponible ({other})"))),
    }
    let content = &p["content"];
    let items = content["items"]
        .as_array()
        .ok_or_else(|| Error::Parse("la playlist no trae canciones".into()))?;
    Ok(Page {
        name: p["name"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("Playlist de Spotify")
            .to_owned(),
        total: content["totalCount"].as_u64().unwrap_or(0),
        tracks: items.iter().filter_map(parse_item).collect(),
        items: items.len() as u64,
    })
}

/// Un elemento de la playlist. Ignora episodios, archivos locales y canciones
/// que ya no existen.
fn parse_item(item: &Value) -> Option<Track> {
    let wrapper = &item["itemV2"];
    if wrapper["__typename"].as_str() != Some("TrackResponseWrapper") {
        return None;
    }
    let t = &wrapper["data"];
    if t["__typename"].as_str().is_some_and(|ty| ty != "Track") {
        return None;
    }
    let title = t["name"].as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    let names = |v: &Value| -> Vec<String> {
        v["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["profile"]["name"].as_str())
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut artists = names(&t["artists"]);
    if artists.is_empty() {
        artists = names(&t["albumOfTrack"]["artists"]);
    }
    Some(Track {
        title: title.to_owned(),
        artists,
        album: t["albumOfTrack"]["name"]
            .as_str()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned),
        duration_ms: t["trackDuration"]["totalMilliseconds"]
            .as_u64()
            .unwrap_or(0),
        isrc: None,
        spotify_id: t["uri"].as_str().and_then(link::track_id),
    })
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_token_and_expiry() {
        let html = r#"<script>{"accessToken":"BQabc-_1","accessTokenExpirationTimestampMs":1795556284410,"isAnonymous":true}</script>"#;
        let (tok, exp) = parse_token(html).unwrap();
        assert_eq!(tok, "BQabc-_1");
        assert_eq!(exp, Some(1_795_556_284_410));
        assert!(parse_token("<html></html>").is_none());
        assert!(parse_token(r#"{"accessToken":""}"#).is_none());
    }

    #[test]
    fn finds_player_script_and_hash() {
        let html = r#"<link href="https://x.cdn/web-player/web-player.css"><script src="https://open.spotifycdn.com/cdn/build/web-player/vendor~web-player.81c07709.js"></script><script src="https://open.spotifycdn.com/cdn/build/web-player/web-player.17c0cc67.js"></script>"#;
        assert_eq!(
            find_player_script(html),
            Some("https://open.spotifycdn.com/cdn/build/web-player/web-player.17c0cc67.js")
        );
        let js = r#"let a=new X("fetchPlaylistMetadata","query","aa",null),b=new X("fetchPlaylist","query","21ea11a6ac75a5f3e01fd365d29c18e0a4a8b7dc09f89bbb5d73910190c6acaf",null)"#;
        assert_eq!(
            find_query_hash(js, "fetchPlaylist").as_deref(),
            Some(KNOWN_HASHES[0])
        );
        assert_eq!(find_query_hash(js, "otra"), None);
    }

    #[test]
    fn parses_items_and_skips_the_rest() {
        let v = json!({"data":{"playlistV2":{"__typename":"Playlist","name":" All Out 80s ","content":{
        "totalCount": 150,
        "items":[
            {"itemV2":{"__typename":"TrackResponseWrapper","data":{"__typename":"Track",
                "name":"Never Too Much","uri":"spotify:track:3nFJbZCHP4d9vduKjJLdBL",
                "artists":{"items":[{"profile":{"name":"Luther Vandross"}}]},
                "albumOfTrack":{"name":"Never Too Much"},
                "trackDuration":{"totalMilliseconds":230269}}}},
            {"itemV2":{"__typename":"EpisodeOrChapterResponseWrapper","data":{"name":"Podcast"}}},
            {"itemV2":{"__typename":"TrackResponseWrapper","data":{"__typename":"NotFound"}}},
            {"itemV2":{"__typename":"LocalTrackResponseWrapper","data":{"name":"local.mp3"}}}
        ]}}}});
        let page = parse_page(&v).unwrap();
        assert_eq!(page.name, "All Out 80s");
        assert_eq!(page.total, 150);
        assert_eq!(page.items, 4);
        assert_eq!(page.tracks.len(), 1);
        let t = &page.tracks[0];
        assert_eq!(t.artists, ["Luther Vandross"]);
        assert_eq!(t.album.as_deref(), Some("Never Too Much"));
        assert_eq!(t.duration_ms, 230_269);
        assert_eq!(t.spotify_id.as_deref(), Some("3nFJbZCHP4d9vduKjJLdBL"));
    }

    #[test]
    fn missing_playlist_is_not_found() {
        let v = json!({"data":{"playlistV2":{"__typename":"NotFound","message":"x"}}});
        assert!(matches!(parse_page(&v), Err(Error::NotFound)));
    }
}
