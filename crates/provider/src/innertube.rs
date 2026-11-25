//! Consultas directas a la API interna de YouTube (InnerTube), sin pasar por
//! yt-dlp: buscar tarda ~0,6 s en vez de ~1,5 s, porque no hay que arrancar
//! un proceso de Python. Si YouTube cambia algo y falla, quien llama vuelve a
//! yt-dlp; tras varios fallos seguidos se deja de usar en la sesión.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::{Error, Listing, Result, SearchResult, StreamInfo};

const API: &str = "https://www.youtube.com/youtubei/v1";
const WEB_VERSION: &str = "2.20251120.01.00";
const TIMEOUT: Duration = Duration::from_secs(8);

/// Filtros de búsqueda (`params`): solo videos, solo canales, solo playlists.
const ONLY_VIDEOS: &str = "EgIQAQ%3D%3D";
const ONLY_CHANNELS: &str = "EgIQAg%3D%3D";
const ONLY_PLAYLISTS: &str = "EgIQAw%3D%3D";

/// Cliente de YouTube para pedir el audio: el de los visores de realidad
/// virtual no necesita resolver el desafío de JavaScript ni PO token.
const VR_NAME: &str = "ANDROID_VR";
const VR_ID: &str = "28";
const VR_VERSION: &str = "1.65.10";
pub(crate) const VR_USER_AGENT: &str = "com.google.android.apps.youtube.vr.oculus/1.65.10 \
     (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";

/// Fallos seguidos tras los cuales se deja de usar en la sesión.
const MAX_FAILURES: u32 = 3;

static SEARCH_FAILURES: AtomicU32 = AtomicU32::new(0);
/// `visitorData` de la última respuesta: YouTube trata mejor a un visitante
/// conocido que a uno nuevo en cada consulta.
static VISITOR: Mutex<Option<String>> = Mutex::new(None);

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

/// La búsqueda directa sigue habilitada en esta sesión.
pub(crate) fn search_enabled() -> bool {
    SEARCH_FAILURES.load(Ordering::Relaxed) < MAX_FAILURES
}

fn search_outcome<T>(r: Result<T>) -> Result<T> {
    match &r {
        Ok(_) => SEARCH_FAILURES.store(0, Ordering::Relaxed),
        Err(e) => {
            let n = SEARCH_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::warn!(error = %e, "falló la búsqueda directa en YouTube; se usa yt-dlp");
            if n == MAX_FAILURES {
                tracing::warn!("la búsqueda directa falló varias veces; se deja de usar");
            }
        }
    }
    r
}

async fn post(
    client: &reqwest::Client,
    endpoint: &str,
    body: Value,
    extra: &[(&str, &str)],
) -> Result<Value> {
    let mut req = client
        .post(format!("{API}/{endpoint}?prettyPrint=false"))
        .header("Content-Type", "application/json")
        .timeout(TIMEOUT);
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let visitor = VISITOR.lock().ok().and_then(|v| v.clone());
    if let Some(v) = &visitor {
        req = req.header("X-Goog-Visitor-Id", v);
    }
    let resp = req.body(body.to_string()).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let bytes = resp.bytes().await?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| Error::Parse(e.to_string()))?;
    if let Some(new) = v["responseContext"]["visitorData"].as_str() {
        if visitor.as_deref() != Some(new) {
            if let Ok(mut slot) = VISITOR.lock() {
                *slot = Some(new.to_owned());
            }
        }
    }
    Ok(v)
}

async fn search_raw(query: &str, params: &str) -> Result<Value> {
    // `params` va sin escapar en el cuerpo JSON.
    let params = params.replace("%3D", "=");
    let body = json!({
        "context": {"client": {"clientName": "WEB", "clientVersion": WEB_VERSION, "hl": "en", "gl": "US"}},
        "query": query,
        "params": params,
    });
    post(client(), "search", body, &[]).await
}

/// Videos que coinciden con `query` (como `ytsearch` de yt-dlp).
pub(crate) async fn search(query: &str, n: usize) -> Result<Vec<SearchResult>> {
    let r = search_raw(query, ONLY_VIDEOS).await.and_then(|v| {
        let videos: Vec<_> = items(&v).filter_map(video).take(n).collect();
        if videos.is_empty() {
            Err(Error::Parse("la búsqueda directa no trajo videos".into()))
        } else {
            Ok(videos)
        }
    });
    search_outcome(r)
}

/// Canales que coinciden con `query`.
pub(crate) async fn search_channels(query: &str, n: usize) -> Result<Vec<Listing>> {
    let r = search_raw(query, ONLY_CHANNELS)
        .await
        .map(|v| items(&v).filter_map(channel).take(n).collect());
    search_outcome(r)
}

/// Playlists que coinciden con `query`.
pub(crate) async fn search_playlists(query: &str, n: usize) -> Result<Vec<Listing>> {
    let r = search_raw(query, ONLY_PLAYLISTS)
        .await
        .map(|v| items(&v).filter_map(playlist).take(n).collect());
    search_outcome(r)
}

/// Los resultados de una respuesta de búsqueda, sin estantes de "otros
/// también miraron" ni shorts.
fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v["contents"]["twoColumnSearchResultsRenderer"]["primaryContents"]["sectionListRenderer"]
        ["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["itemSectionRenderer"]["contents"].as_array())
        .flatten()
}

/// Texto de un campo `{simpleText}` o `{runs: [{text}]}`.
fn text(v: &Value) -> Option<String> {
    let s = match v["simpleText"].as_str() {
        Some(s) => s.to_owned(),
        None => v["runs"]
            .as_array()?
            .iter()
            .filter_map(|r| r["text"].as_str())
            .collect(),
    };
    let s = s.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

fn video(item: &Value) -> Option<SearchResult> {
    let v = item.get("videoRenderer")?;
    let id = v["videoId"]
        .as_str()
        .filter(|id| id.len() == 11)?
        .to_owned();
    Some(SearchResult {
        id,
        title: text(&v["title"])?,
        channel: text(&v["ownerText"]).or_else(|| text(&v["longBylineText"])),
        duration: text(&v["lengthText"]).and_then(|t| parse_length(&t)),
        views: text(&v["viewCountText"]).and_then(|t| parse_count(&t)),
    })
}

fn channel(item: &Value) -> Option<Listing> {
    let c = item.get("channelRenderer")?;
    let id = c["channelId"].as_str()?.to_owned();
    // Con el diseño actual, `videoCountText` trae los suscriptores y
    // `subscriberCountText` el @usuario; se prueba con los dos.
    let followers = [&c["videoCountText"], &c["subscriberCountText"]]
        .into_iter()
        .filter_map(text)
        .find(|t| t.contains("subscriber"))
        .and_then(|t| parse_count(&t));
    Some(Listing {
        url: format!("https://www.youtube.com/channel/{id}"),
        title: text(&c["title"])?,
        id,
        channel: None,
        followers,
    })
}

fn playlist(item: &Value) -> Option<Listing> {
    if let Some(p) = item.get("playlistRenderer") {
        let id = p["playlistId"].as_str()?.to_owned();
        return Some(Listing {
            url: format!("https://www.youtube.com/playlist?list={id}"),
            title: text(&p["title"])?,
            channel: text(&p["shortBylineText"]).or_else(|| text(&p["longBylineText"])),
            id,
            followers: None,
        });
    }
    let l = item.get("lockupViewModel")?;
    if l["contentType"].as_str() != Some("LOCKUP_CONTENT_TYPE_PLAYLIST") {
        return None;
    }
    let id = l["contentId"].as_str()?;
    // Los "Mix" son personalizados y no se pueden importar como playlist.
    if id.starts_with("RD") {
        return None;
    }
    let meta = &l["metadata"]["lockupMetadataViewModel"];
    let title = meta["title"]["content"].as_str()?.trim().to_owned();
    let owner = meta["metadata"]["contentMetadataViewModel"]["metadataRows"][0]["metadataParts"][0]
        ["text"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some(Listing {
        url: format!("https://www.youtube.com/playlist?list={id}"),
        id: id.to_owned(),
        title,
        channel: owner,
        followers: None,
    })
}

/// `"4:16"` o `"1:02:03"`.
fn parse_length(t: &str) -> Option<Duration> {
    let mut secs = 0u64;
    for part in t.trim().split(':') {
        secs = secs * 60 + part.trim().parse::<u64>().ok()?;
    }
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// `"1,234,567 views"`, `"6.28M subscribers"`, `"152K subscribers"`.
fn parse_count(t: &str) -> Option<u64> {
    let t = t.trim();
    let end = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(t.len());
    let (num, rest) = t.split_at(end);
    let num = num.replace(',', "");
    if num.is_empty() {
        return None;
    }
    let mult = match rest.trim_start().chars().next() {
        Some('K') => 1e3,
        Some('M') => 1e6,
        Some('B') => 1e9,
        _ => 1.0,
    };
    let n: f64 = num.parse().ok()?;
    Some((n * mult).round() as u64)
}

/// Sugerencias de búsqueda de YouTube para lo que se lleva escrito.
pub async fn suggestions(query: &str, lang: &str) -> Result<Vec<String>> {
    let resp = client()
        .get("https://suggestqueries.google.com/complete/search")
        .query(&[
            ("client", "firefox"),
            ("ds", "yt"),
            ("hl", lang),
            ("q", query),
        ])
        .timeout(Duration::from_secs(3))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(Error::Status(resp.status().as_u16()));
    }
    let bytes = resp.bytes().await?;
    parse_suggestions(&bytes)
}

fn parse_suggestions(bytes: &[u8]) -> Result<Vec<String>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| Error::Parse(e.to_string()))?;
    Ok(v[1]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect())
}

/// URL del audio pedida directamente a YouTube con el cliente de visores de
/// realidad virtual. `client` tiene que salir por IPv4, igual que la descarga
/// (la URL queda atada a la IP que la pidió).
pub(crate) async fn player(client: &reqwest::Client, video: &str) -> Result<StreamInfo> {
    let body = json!({
        "context": {"client": {
            "clientName": VR_NAME,
            "clientVersion": VR_VERSION,
            "deviceMake": "Oculus",
            "deviceModel": "Quest 3",
            "androidSdkVersion": 32,
            "osName": "Android",
            "osVersion": "12L",
            "hl": "en",
            "gl": "US",
        }},
        "videoId": video,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let v = post(
        client,
        "player",
        body,
        &[
            ("User-Agent", VR_USER_AGENT),
            ("X-YouTube-Client-Name", VR_ID),
            ("X-YouTube-Client-Version", VR_VERSION),
        ],
    )
    .await?;
    parse_player(&v)
}

fn parse_player(v: &Value) -> Result<StreamInfo> {
    let status = &v["playabilityStatus"];
    match status["status"].as_str() {
        Some("OK") => {}
        other => {
            let reason = text(&status["reason"])
                .or_else(|| status["reason"].as_str().map(str::to_owned))
                .unwrap_or_else(|| other.unwrap_or("sin estado").to_owned());
            if other == Some("LOGIN_REQUIRED") || reason.contains("bot") {
                return Err(Error::BotCheck(reason));
            }
            return Err(Error::Parse(format!("YouTube no da el audio: {reason}")));
        }
    }
    let best = v["streamingData"]["adaptiveFormats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| {
            f["mimeType"]
                .as_str()
                .is_some_and(|m| m.starts_with("audio/mp4"))
        })
        .filter(|f| f["url"].is_string())
        .max_by_key(|f| f["bitrate"].as_u64().unwrap_or(0))
        .ok_or_else(|| Error::Parse("YouTube no dio un audio m4a con URL directa".into()))?;
    let details = &v["videoDetails"];
    Ok(StreamInfo {
        url: best["url"].as_str().unwrap_or_default().to_owned(),
        ext: "m4a".into(),
        title: details["title"].as_str().map(str::to_owned),
        duration: details["lengthSeconds"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|s| *s > 0)
            .map(Duration::from_secs),
        user_agent: Some(VR_USER_AGENT.to_owned()),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_counts_and_lengths() {
        assert_eq!(parse_count("1,234,567 views"), Some(1_234_567));
        assert_eq!(parse_count("6.28M subscribers"), Some(6_280_000));
        assert_eq!(parse_count("152K subscribers"), Some(152_000));
        assert_eq!(parse_count("No views"), None);
        assert_eq!(parse_length("4:16"), Some(Duration::from_secs(256)));
        assert_eq!(parse_length("1:02:03"), Some(Duration::from_secs(3723)));
        assert_eq!(parse_length("LIVE"), None);
    }

    #[test]
    fn parses_search_items() {
        let v = json!({"contents": {"twoColumnSearchResultsRenderer": {"primaryContents": {"sectionListRenderer": {"contents": [
            {"itemSectionRenderer": {"contents": [
                {"videoRenderer": {"videoId": "4qdBojzaKsk", "title": {"runs": [{"text": "Mar Azul"}]},
                    "ownerText": {"runs": [{"text": "Natanael Cano"}]}, "lengthText": {"simpleText": "4:16"},
                    "viewCountText": {"simpleText": "12,345 views"}}},
                {"shelfRenderer": {"content": {"videoRenderer": {"videoId": "xxxxxxxxxxx"}}}},
                {"channelRenderer": {"channelId": "UCW2", "title": {"simpleText": "Natanael Cano"},
                    "subscriberCountText": {"simpleText": "@natanaelcano777"},
                    "videoCountText": {"simpleText": "6.28M subscribers"}}},
                {"lockupViewModel": {"contentId": "PLD964", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
                    "metadata": {"lockupMetadataViewModel": {"title": {"content": "Vol. 1"},
                        "metadata": {"contentMetadataViewModel": {"metadataRows": [{"metadataParts": [{"text": {"content": "Natanael Cano"}}]}]}}}}}},
                {"lockupViewModel": {"contentId": "RD4qdBojzaKsk", "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
                    "metadata": {"lockupMetadataViewModel": {"title": {"content": "Mix"}}}}}
            ]}}
        ]}}}}});
        let videos: Vec<_> = items(&v).filter_map(video).collect();
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].id, "4qdBojzaKsk");
        assert_eq!(videos[0].channel.as_deref(), Some("Natanael Cano"));
        assert_eq!(videos[0].duration, Some(Duration::from_secs(256)));
        assert_eq!(videos[0].views, Some(12_345));
        let channels: Vec<_> = items(&v).filter_map(channel).collect();
        assert_eq!(channels[0].followers, Some(6_280_000));
        assert_eq!(channels[0].url, "https://www.youtube.com/channel/UCW2");
        let lists: Vec<_> = items(&v).filter_map(playlist).collect();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].title, "Vol. 1");
        assert_eq!(lists[0].channel.as_deref(), Some("Natanael Cano"));
    }

    #[test]
    fn parses_player_responses() {
        let ok = json!({"playabilityStatus": {"status": "OK"},
        "videoDetails": {"title": "t", "lengthSeconds": "200"},
        "streamingData": {"adaptiveFormats": [
            {"mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 130000, "url": "https://a/hi"},
            {"mimeType": "audio/mp4; codecs=\"mp4a.40.5\"", "bitrate": 50000, "url": "https://a/lo"},
            {"mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 160000, "url": "https://a/opus"},
            {"mimeType": "audio/mp4", "bitrate": 999999, "signatureCipher": "s=..."}
        ]}});
        let info = parse_player(&ok).unwrap();
        assert_eq!(info.url, "https://a/hi");
        assert_eq!(info.duration, Some(Duration::from_secs(200)));
        assert_eq!(info.user_agent.as_deref(), Some(VR_USER_AGENT));

        let bot = json!({"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm you’re not a bot"}});
        assert!(matches!(parse_player(&bot), Err(Error::BotCheck(_))));
    }

    #[test]
    fn parses_suggestions() {
        let got = parse_suggestions(br#"["caf",["cafe tacvba","cafe con aroma"],[],{}]"#).unwrap();
        assert_eq!(got, ["cafe tacvba", "cafe con aroma"]);
    }

    #[tokio::test]
    #[ignore = "usa la red"]
    async fn real_parallel() {
        for q in ["cafe tacv", "soda stereo", "natanael"] {
            let (a, b, c) =
                tokio::join!(search(q, 15), search_channels(q, 6), search_playlists(q, 8));
            println!(
                "{q}: {:?} {:?} {:?}",
                a.map(|v| v.len()),
                b.map(|v| v.len()),
                c.map(|v| v.len())
            );
        }
    }

    /// Contra YouTube de verdad: `cargo test -p surco-provider innertube -- --ignored`.
    #[tokio::test]
    #[ignore = "usa la red"]
    async fn real_search() {
        let v = search("natanael cano mar azul", 10).await.unwrap();
        assert!(!v.is_empty());
        let c = search_channels("natanael cano", 5).await.unwrap();
        assert!(!c.is_empty());
        let p = search_playlists("natanael cano", 5).await.unwrap();
        assert!(!p.is_empty());
        assert!(!suggestions("natanael", "es").await.unwrap().is_empty());
    }
}
