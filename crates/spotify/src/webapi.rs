//! Estrategia 1: Spotify Web API con Client Credentials.
//!
//! El usuario pone `client_id` y `client_secret` en la config. Desde febrero
//! de 2026 las apps en "Development Mode" tienen restricciones: el endpoint
//! pasó de `/playlists/{id}/tracks` a `/playlists/{id}/items` (y el campo
//! `tracks` a `items`), y para playlists ajenas la API puede devolver solo los
//! metadatos, sin canciones. En ese caso devolvemos [`Error::NoTracks`] y la
//! cascada sigue con la página embed. Aceptamos ambos formatos (viejo y nuevo).

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde_json::Value;
use surco_core::{Playlist, Track};
use tokio::sync::Mutex;

use crate::{Error, Imported, PlaylistSource, Result};

/// Reintentos ante HTTP 429 antes de rendirse.
const MAX_RETRIES: u32 = 5;
/// Tope de espera por un `Retry-After` (si piden más, mejor seguir con otra estrategia).
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Tope de páginas (100 canciones c/u) para no colgarse con una respuesta rara.
const MAX_PAGES: usize = 200;

/// Credenciales de una app de Spotify (developer.spotify.com).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

/// Fuente basada en la Web API oficial.
#[derive(Debug)]
pub struct WebApi {
    client: reqwest::Client,
    creds: Credentials,
    api_base: String,
    token_url: String,
    token: Mutex<Option<(String, Instant)>>,
}

impl WebApi {
    pub fn new(client: reqwest::Client, creds: Credentials) -> Self {
        Self::with_endpoints(
            client,
            creds,
            "https://api.spotify.com/v1",
            "https://accounts.spotify.com/api/token",
        )
    }

    /// Para tests: otras URLs.
    pub fn with_endpoints(
        client: reqwest::Client,
        creds: Credentials,
        api_base: impl Into<String>,
        token_url: impl Into<String>,
    ) -> Self {
        Self {
            client,
            creds,
            api_base: api_base.into(),
            token_url: token_url.into(),
            token: Mutex::new(None),
        }
    }

    async fn token(&self, force: bool) -> Result<String> {
        let mut guard = self.token.lock().await;
        if let Some((tok, expires)) = guard.as_ref() {
            if !force && Instant::now() < *expires {
                return Ok(tok.clone());
            }
        }
        let resp = self
            .client
            .post(&self.token_url)
            .basic_auth(&self.creds.client_id, Some(&self.creds.client_secret))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body("grant_type=client_credentials")
            .send()
            .await?;
        let status = resp.status();
        if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::UNAUTHORIZED
        {
            return Err(Error::BadCredentials);
        }
        if !status.is_success() {
            return Err(Error::Status(status.as_u16()));
        }
        let body: Value = serde_json::from_str(&resp.text().await?)
            .map_err(|e| Error::Parse(format!("token: {e}")))?;
        let tok = body["access_token"]
            .as_str()
            .ok_or_else(|| Error::Parse("token sin access_token".into()))?
            .to_owned();
        let secs = body["expires_in"].as_u64().unwrap_or(3600);
        // Un minuto de margen.
        let expires = Instant::now() + Duration::from_secs(secs.saturating_sub(60));
        *guard = Some((tok.clone(), expires));
        Ok(tok)
    }

    /// GET con token, renovación ante 401 y espera ante 429 (`Retry-After`).
    async fn get_json(&self, url: &str) -> Result<Value> {
        let mut retries = 0;
        let mut refreshed = false;
        loop {
            let tok = self.token(false).await?;
            let resp = self.client.get(url).bearer_auth(&tok).send().await?;
            let status = resp.status().as_u16();
            match status {
                200..=299 => {
                    return serde_json::from_str(&resp.text().await?)
                        .map_err(|e| Error::Parse(format!("respuesta de la API: {e}")));
                }
                401 if !refreshed => {
                    refreshed = true;
                    self.token(true).await?;
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
                404 => return Err(Error::NotFound),
                // 403: típico de las restricciones de Development Mode.
                403 => return Err(Error::NoTracks),
                _ => return Err(Error::Status(status)),
            }
        }
    }

    async fn fetch_impl(&self, id: &str) -> Result<Imported> {
        let meta = self
            .get_json(&format!("{}/playlists/{id}", self.api_base))
            .await?;
        let name = meta["name"]
            .as_str()
            .unwrap_or("Playlist de Spotify")
            .to_owned();

        // Primera página: viene embebida (`items` nuevo o `tracks` viejo).
        let mut page = match (&meta["items"], &meta["tracks"]) {
            (p @ Value::Object(_), _) | (_, p @ Value::Object(_)) => p.clone(),
            _ => Value::Null,
        };
        if !page["items"].is_array() {
            // Puede que solo hayan omitido la página: pedimos el endpoint nuevo.
            page = self
                .get_json(&format!(
                    "{}/playlists/{id}/items?limit=100&additional_types=track",
                    self.api_base
                ))
                .await?;
        }
        let total = page["total"].as_u64().unwrap_or(0);

        let mut tracks = Vec::new();
        let mut pages = 0;
        loop {
            let Some(items) = page["items"].as_array() else {
                return Err(Error::NoTracks);
            };
            tracks.extend(items.iter().filter_map(parse_item));
            pages += 1;
            match page["next"].as_str() {
                Some(next) if pages < MAX_PAGES => {
                    let next = next.to_owned();
                    page = self.get_json(&next).await?;
                }
                _ => break,
            }
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
            warnings: Vec::new(),
        })
    }
}

impl PlaylistSource for WebApi {
    fn name(&self) -> &'static str {
        "Web API de Spotify"
    }

    fn fetch<'a>(
        &'a self,
        id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Imported>> + Send + 'a>> {
        Box::pin(self.fetch_impl(id))
    }
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

/// Un elemento de la página (`{"item": {...}}` nuevo o `{"track": {...}}` viejo).
/// Ignora episodios de podcasts, canciones locales y elementos nulos.
fn parse_item(item: &Value) -> Option<Track> {
    let t = match (&item["item"], &item["track"]) {
        (t @ Value::Object(_), _) | (_, t @ Value::Object(_)) => t,
        _ => return None,
    };
    if t["type"].as_str().is_some_and(|ty| ty != "track") || t["is_local"] == Value::Bool(true) {
        return None;
    }
    let title = t["name"].as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    Some(Track {
        title: title.to_owned(),
        artists: t["artists"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        album: t["album"]["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        duration_ms: t["duration_ms"].as_u64().unwrap_or(0),
        isrc: t["external_ids"]["isrc"].as_str().map(str::to_owned),
        spotify_id: t["id"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_old_and_new_items() {
        let track = json!({"type":"track","id":"abc","name":"Song","duration_ms":1000,
            "artists":[{"name":"A"},{"name":"B"}],"album":{"name":"Al"},"external_ids":{"isrc":"US123"}});
        for item in [json!({"track": track.clone()}), json!({"item": track})] {
            let t = parse_item(&item).unwrap();
            assert_eq!(t.title, "Song");
            assert_eq!(t.artists, ["A", "B"]);
            assert_eq!(t.album.as_deref(), Some("Al"));
            assert_eq!(t.isrc.as_deref(), Some("US123"));
            assert_eq!(t.spotify_id.as_deref(), Some("abc"));
        }
    }

    #[test]
    fn skips_episodes_local_and_null() {
        assert!(parse_item(&json!({"track": null})).is_none());
        assert!(parse_item(&json!({"track": {"type":"episode","name":"Pod"}})).is_none());
        assert!(parse_item(&json!({"track": {"name":"x","is_local":true}})).is_none());
    }
}
