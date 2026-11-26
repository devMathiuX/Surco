//! Importación de punta a punta contra un servidor HTTP local que imita a
//! Spotify (token, paginación, 429 con Retry-After, restricciones de
//! Development Mode) y con un CSV de ejemplo.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use surco_spotify::{import_csv_file, Embed, Error, Importer, PlaylistSource, WebPlayer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ID: &str = "37i9dQZF1DX4UtSsGT1Sbe";

/// Respuesta: (status, headers extra, cuerpo).
type Reply = (u16, Vec<(&'static str, String)>, String);

/// Servidor mínimo: llama a `handler(método, ruta, headers)` por cada pedido.
async fn serve<F>(handler: F) -> String
where
    F: Fn(&str, &str, &str) -> Reply + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let req = String::from_utf8_lossy(&req).into_owned();
                let mut first = req.lines().next().unwrap_or_default().split(' ');
                let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                let (status, headers, body) = handler(method, path, &req.to_lowercase());
                let mut head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            });
        }
    });
    base
}

#[test]
fn embed_not_found_stops_the_cascade() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|_, _, _| (404, vec![], String::new())));
    let client = reqwest::Client::new();

    // La embed da 404: la playlist no existe, la cascada corta ahí.
    let importer = Importer::new(vec![Box::new(Embed::with_base(client, base))]);
    assert!(matches!(
        rt.block_on(importer.import_id(ID)),
        Err(Error::NotFound)
    ));
}

#[test]
fn imports_exportify_csv_sample() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/exportify_sample.csv");
    let imp = import_csv_file(&path).unwrap();
    assert_eq!(imp.playlist.name, "exportify sample");
    assert_eq!(imp.playlist.tracks.len(), 120);
    assert!(imp.warnings.is_empty());
    let t = &imp.playlist.tracks[32];
    assert_eq!(t.title, "Canción de prueba 33, con \"comillas\"");
    assert!(imp.playlist.tracks.iter().all(|t| t.spotify_id.is_some()
        && t.isrc.is_some()
        && t.duration_ms > 0
        && !t.artists.is_empty()));
    assert_eq!(imp.playlist.tracks[6].artists.len(), 2);
}

// --- Contra Spotify real (ignorados por defecto) ---------------------------
//
//   cargo test -p surco-spotify --test import -- --ignored --nocapture

/// "All Out 80s" (150 canciones): la embed devuelve las primeras 100.
#[test]
#[ignore = "necesita internet"]
fn real_embed_import() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let embed = Embed::new(reqwest::Client::new());
    let imp = rt.block_on(embed.fetch(ID)).unwrap();
    eprintln!(
        "{}: {} canciones; primera: {:?}",
        imp.playlist.name,
        imp.playlist.tracks.len(),
        imp.playlist.tracks.first()
    );
    assert_eq!(imp.playlist.tracks.len(), 100);
    assert!(!imp.warnings.is_empty());
    assert!(imp
        .playlist
        .tracks
        .iter()
        .all(|t| !t.artists.is_empty() && t.duration_ms > 0 && t.spotify_id.is_some()));
}

// --- Reproductor web sin sesión -------------------------------------------

const GOOD_HASH: &str = "abababababababababababababababababababababababababababababababab";

/// Decodifica `%XX` y `+` de una URL (suficiente para estos tests).
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap()
}

fn embed_with_token(token: &str) -> String {
    format!(
        r#"<html><script>{{"accessToken":"{token}","accessTokenExpirationTimestampMs":9999999999999,"isAnonymous":true}}</script></html>"#
    )
}

/// Página de `fetchPlaylist` con el formato del reproductor web.
fn player_page(offset: usize, limit: usize, total: usize) -> String {
    let end = (offset + limit).min(total);
    let items: Vec<Value> = (offset..end)
        .map(|i| {
            json!({"itemV2":{"__typename":"TrackResponseWrapper","data":{"__typename":"Track",
                "name":format!("Tema {i}"),"uri":format!("spotify:track:{i:022}"),
                "artists":{"items":[{"profile":{"name":"Artista"}}]},
                "albumOfTrack":{"name":"Disco"},
                "trackDuration":{"totalMilliseconds":180_000 + i}}}})
        })
        .collect();
    json!({"data":{"playlistV2":{"__typename":"Playlist","name":"Larga","content":{
        "totalCount":total,"items":items,"pagingInfo":{"offset":offset,"limit":limit}}}}})
    .to_string()
}

/// Lee hash, offset y limit de una consulta `fetchPlaylist`.
fn query_args(path: &str) -> (String, usize, usize) {
    let path = decode(path);
    let num = |key: &str| -> usize {
        let i = path.find(key).unwrap() + key.len();
        path[i..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap()
    };
    let h = path.find("\"sha256Hash\":\"").unwrap() + "\"sha256Hash\":\"".len();
    let hash = path[h..h + 64].to_owned();
    (hash, num("\"offset\":"), num("\"limit\":"))
}

#[test]
fn web_player_reads_the_whole_playlist_with_an_anonymous_token() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let embeds = Arc::new(AtomicUsize::new(0));
    let e = embeds.clone();
    let base_cell = Arc::new(std::sync::OnceLock::<String>::new());
    let b = base_cell.clone();
    let base = rt.block_on(serve(move |_, path, headers| {
        let base = b.get().cloned().unwrap_or_default();
        if path == format!("/embed/playlist/{ID}") {
            e.fetch_add(1, Ordering::SeqCst);
            return (200, vec![], embed_with_token("anon"));
        }
        // Página principal y JavaScript del reproductor (para el hash actual).
        if path == "/" {
            let html = format!(
                r#"<script src="{base}/cdn/web-player/vendor~web-player.1.js"></script><script src="{base}/cdn/web-player/web-player.2.js"></script>"#
            );
            return (200, vec![], html);
        }
        if path == "/cdn/web-player/web-player.2.js" {
            let js = format!(r#"x=new Q("fetchPlaylist","query","{GOOD_HASH}",null)"#);
            return (200, vec![], js);
        }
        if path.starts_with("/query?") {
            assert!(headers.contains("authorization: bearer anon"), "{headers}");
            let (hash, offset, limit) = query_args(path);
            // Los hashes de fábrica "vencieron".
            if hash != GOOD_HASH {
                return (
                    200,
                    vec![],
                    json!({"errors":[{"message":"PersistedQueryNotFound"}]}).to_string(),
                );
            }
            return (200, vec![], player_page(offset, limit, 250));
        }
        (404, vec![], String::new())
    }));
    base_cell.set(base.clone()).unwrap();

    let player = WebPlayer::with_endpoints(reqwest::Client::new(), &base, format!("{base}/query"));
    let imp = rt.block_on(player.fetch(ID)).unwrap();
    assert_eq!(imp.playlist.name, "Larga");
    assert_eq!(imp.playlist.tracks.len(), 250);
    assert_eq!(imp.playlist.tracks[249].title, "Tema 249");
    assert_eq!(imp.playlist.tracks[0].album.as_deref(), Some("Disco"));
    assert!(imp.warnings.is_empty(), "{:?}", imp.warnings);

    // La segunda vez reutiliza el token y el hash encontrado.
    let again = rt.block_on(player.fetch(ID)).unwrap();
    assert_eq!(again.playlist.tracks.len(), 250);
    assert_eq!(embeds.load(Ordering::SeqCst), 1, "el token se reutiliza");
}

#[test]
fn web_player_missing_playlist_stops_the_cascade() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|_, path, _| {
        if path.starts_with("/embed/") {
            return (200, vec![], embed_with_token("anon"));
        }
        let body = json!({"data":{"playlistV2":{"__typename":"NotFound","message":"no"}}});
        (200, vec![], body.to_string())
    }));
    let client = reqwest::Client::new();
    let importer = Importer::new(vec![
        Box::new(WebPlayer::with_endpoints(
            client.clone(),
            &base,
            format!("{base}/query"),
        )),
        Box::new(Embed::with_base(client, &base)),
    ]);
    assert!(matches!(
        rt.block_on(importer.import_id(ID)),
        Err(Error::NotFound)
    ));
}

#[test]
fn cascade_falls_back_to_embed_when_the_web_player_fails() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|_, path, _| {
        if path == format!("/embed/playlist/{ID}") {
            // Página embed sin token, pero con las canciones.
            let data = json!({"props":{"pageProps":{"state":{"data":{"entity":{"name":"Desde embed",
                "trackList":[{"uri":"spotify:track:0000000000000000000001","title":"T1","subtitle":"A","duration":1000}]}}}}}});
            let html = format!(
                "<html><script id=\"__NEXT_DATA__\" type=\"application/json\">{data}</script></html>"
            );
            return (200, vec![], html);
        }
        (500, vec![], String::new())
    }));
    let client = reqwest::Client::new();
    let importer = Importer::new(vec![
        Box::new(WebPlayer::with_endpoints(
            client.clone(),
            &base,
            format!("{base}/query"),
        )),
        Box::new(Embed::with_base(client, &base)),
    ]);
    let imp = rt.block_on(importer.import_id(ID)).unwrap();
    assert_eq!(imp.source, "página embed");
    assert_eq!(imp.playlist.tracks.len(), 1);
}

/// Contra Spotify de verdad: una editorial ("All Out 80s", 150 canciones) y
/// una de un usuario con más de 1 500, enteras y sin claves.
#[test]
#[ignore = "necesita internet"]
fn real_web_player_import() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let player = WebPlayer::new(reqwest::Client::new());
    for (id, min) in [(ID, 101), ("5jyQ13hyJBKCosn5giwyL2", 1500)] {
        let start = Instant::now();
        let imp = rt.block_on(player.fetch(id)).unwrap();
        eprintln!(
            "{}: {} canciones en {:?}; última: {:?}; avisos: {:?}",
            imp.playlist.name,
            imp.playlist.tracks.len(),
            start.elapsed(),
            imp.playlist.tracks.last(),
            imp.warnings
        );
        assert!(imp.playlist.tracks.len() >= min);
        assert!(imp
            .playlist
            .tracks
            .iter()
            .all(|t| !t.artists.is_empty() && t.duration_ms > 0 && t.spotify_id.is_some()));
    }
}
