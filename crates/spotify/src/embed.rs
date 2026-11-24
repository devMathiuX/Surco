//! Estrategia 2: página embed pública, sin credenciales.
//!
//! `open.spotify.com/embed/playlist/{id}` es una app Next.js que trae los
//! datos de la playlist en `<script id="__NEXT_DATA__">`. **No es una API
//! pública**: si Spotify cambia el HTML, esto se rompe (y la cascada sigue con
//! el CSV). Limitaciones conocidas: muestra como mucho 100 canciones y no trae
//! álbum ni ISRC.

use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;
use surco_core::{Playlist, Track};

use crate::{link, Error, Imported, PlaylistSource, Result};

/// Máximo de canciones que muestra hoy la página embed.
pub const EMBED_LIMIT: usize = 100;

const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// Fuente basada en la página embed.
#[derive(Debug, Clone)]
pub struct Embed {
    client: reqwest::Client,
    base: String,
}

impl Embed {
    pub fn new(client: reqwest::Client) -> Self {
        Self::with_base(client, "https://open.spotify.com")
    }

    /// Para tests: otra URL base.
    pub fn with_base(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into(),
        }
    }

    async fn fetch_impl(&self, id: &str) -> Result<Imported> {
        let url = format!("{}/embed/playlist/{id}", self.base);
        let resp = self
            .client
            .get(&url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en")
            .send()
            .await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(Error::NotFound);
        }
        if !status.is_success() {
            return Err(Error::Status(status.as_u16()));
        }
        let html = resp.text().await?;
        let playlist = parse_embed_html(&html, id)?;
        let mut warnings = Vec::new();
        if playlist.tracks.len() >= EMBED_LIMIT {
            warnings.push(format!(
                "Sin credenciales de Spotify solo se pueden leer las primeras {EMBED_LIMIT} canciones. \
                 Para la playlist completa, importá un CSV de Exportify."
            ));
        }
        Ok(Imported {
            playlist,
            source: self.name(),
            warnings,
        })
    }
}

impl PlaylistSource for Embed {
    fn name(&self) -> &'static str {
        "página embed"
    }

    fn fetch<'a>(
        &'a self,
        id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Imported>> + Send + 'a>> {
        Box::pin(self.fetch_impl(id))
    }
}

#[derive(Deserialize)]
struct NextData {
    props: Props,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Props {
    page_props: PageProps,
}
#[derive(Deserialize)]
struct PageProps {
    state: State,
}
#[derive(Deserialize)]
struct State {
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    entity: Entity,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entity {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    track_list: Vec<EmbedTrack>,
}
#[derive(Deserialize)]
struct EmbedTrack {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    title: String,
    /// Artistas separados por coma (con espacios duros U+00A0).
    #[serde(default)]
    subtitle: String,
    #[serde(default)]
    duration: u64,
}

/// Extrae la playlist del HTML de la página embed.
pub fn parse_embed_html(html: &str, id: &str) -> Result<Playlist> {
    const MARKER: &str = r#"<script id="__NEXT_DATA__" type="application/json">"#;
    let start = html
        .find(MARKER)
        .map(|i| i + MARKER.len())
        .ok_or_else(|| Error::Parse("la página embed no trae __NEXT_DATA__".into()))?;
    let len = html[start..]
        .find("</script>")
        .ok_or_else(|| Error::Parse("__NEXT_DATA__ sin cerrar".into()))?;
    let data: NextData = serde_json::from_str(&html[start..start + len])
        .map_err(|e| Error::Parse(format!("JSON de la página embed: {e}")))?;
    let entity = data.props.page_props.state.data.entity;

    let tracks = entity
        .track_list
        .into_iter()
        .filter(|t| !t.title.trim().is_empty())
        .map(|t| Track {
            title: t.title.trim().to_owned(),
            artists: split_artists(&t.subtitle),
            album: None,
            duration_ms: t.duration,
            isrc: None,
            spotify_id: link::track_id(&t.uri),
        })
        .collect();

    Ok(Playlist {
        id: id.to_owned(),
        name: entity
            .name
            .or(entity.title)
            .unwrap_or_else(|| "Playlist de Spotify".into()),
        tracks,
    })
}

fn split_artists(subtitle: &str) -> Vec<String> {
    subtitle
        .replace('\u{a0}', " ")
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"<html><head></head><body><div id="__next"></div>
<script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{"type":"playlist","name":"Clásicos","uri":"spotify:playlist:37i9dQZF1DX4UtSsGT1Sbe","trackList":[
{"uri":"spotify:track:2LlQb7Uoj1kKyGhlkBf9aC","title":"Thriller","subtitle":"Michael Jackson","duration":358807,"isPlayable":true},
{"uri":"spotify:track:4HCOHRCSQelz2GPX8iTamO","title":"Under Pressure","subtitle":"Queen, David Bowie","duration":248440},
{"uri":"spotify:local:x","title":"   ","subtitle":"","duration":0}
]}}}}}}</script></body></html>"#;

    #[test]
    fn parses_sample_page() {
        let p = parse_embed_html(SAMPLE, "37i9dQZF1DX4UtSsGT1Sbe").unwrap();
        assert_eq!(p.name, "Clásicos");
        assert_eq!(p.tracks.len(), 2);
        let t = &p.tracks[1];
        assert_eq!(t.title, "Under Pressure");
        assert_eq!(t.artists, ["Queen", "David Bowie"]);
        assert_eq!(t.duration_ms, 248_440);
        assert_eq!(t.spotify_id.as_deref(), Some("4HCOHRCSQelz2GPX8iTamO"));
    }

    #[test]
    fn missing_data_is_a_parse_error() {
        assert!(matches!(
            parse_embed_html("<html></html>", "x"),
            Err(Error::Parse(_))
        ));
    }
}
