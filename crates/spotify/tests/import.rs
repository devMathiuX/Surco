//! Importación de punta a punta contra un servidor HTTP local que imita a
//! Spotify (token, paginación, 429 con Retry-After, restricciones de
//! Development Mode) y con un CSV de ejemplo.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use surco_spotify::{import_csv_file, Credentials, Embed, Error, Importer, PlaylistSource, WebApi};
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

fn creds() -> Credentials {
    Credentials {
        client_id: "id".into(),
        client_secret: "secret".into(),
    }
}

fn track_json(i: usize) -> Value {
    json!({"type":"track","id":format!("{i:022}"),"name":format!("Tema {i}"),"duration_ms":180_000 + i,
        "artists":[{"name":"Artista"}],"album":{"name":"Disco"},"external_ids":{"isrc":format!("ISRC{i}")}})
}

/// Página con el formato nuevo (`items` / `item`) a partir de `offset`.
fn page(base: &str, offset: usize, total: usize) -> Value {
    let end = (offset + 100).min(total);
    let next =
        (end < total).then(|| format!("{base}/v1/playlists/{ID}/items?offset={end}&limit=100"));
    json!({
        "total": total,
        "next": next,
        "items": (offset..end).map(|i| json!({"item": track_json(i)})).collect::<Vec<_>>(),
    })
}

#[test]
fn web_api_paginates_and_respects_retry_after() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let hits_429 = Arc::new(AtomicUsize::new(0));
    let tokens = Arc::new(AtomicUsize::new(0));
    let base_cell = Arc::new(std::sync::OnceLock::<String>::new());

    let (h, t, b) = (hits_429.clone(), tokens.clone(), base_cell.clone());
    let base = rt.block_on(serve(move |method, path, headers| {
        let base = b.get().cloned().unwrap_or_default();
        if method == "POST" && path == "/token" {
            assert!(headers.contains("authorization: basic "));
            t.fetch_add(1, Ordering::SeqCst);
            return (
                200,
                vec![],
                json!({"access_token":"tok","expires_in":3600}).to_string(),
            );
        }
        assert!(headers.contains("authorization: bearer tok"), "{headers}");
        if path == format!("/v1/playlists/{ID}") {
            let mut meta = json!({"name": "Todo el rock"});
            meta["items"] = page(&base, 0, 250);
            return (200, vec![], meta.to_string());
        }
        // La segunda página responde 429 una vez.
        if path.contains("offset=100") && h.fetch_add(1, Ordering::SeqCst) == 0 {
            return (429, vec![("retry-after", "1".into())], String::new());
        }
        if let Some(off) = path.split("offset=").nth(1) {
            let off: usize = off.split('&').next().unwrap().parse().unwrap();
            return (200, vec![], page(&base, off, 250).to_string());
        }
        (404, vec![], String::new())
    }));
    base_cell.set(base.clone()).unwrap();

    let api = WebApi::with_endpoints(
        reqwest::Client::new(),
        creds(),
        format!("{base}/v1"),
        format!("{base}/token"),
    );
    let start = Instant::now();
    let imp = rt.block_on(api.fetch(ID)).unwrap();
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "respetó Retry-After"
    );
    assert_eq!(imp.playlist.name, "Todo el rock");
    assert_eq!(imp.playlist.tracks.len(), 250);
    assert_eq!(imp.playlist.tracks[249].title, "Tema 249");
    assert_eq!(imp.playlist.tracks[0].isrc.as_deref(), Some("ISRC0"));
    assert_eq!(hits_429.load(Ordering::SeqCst), 2);
    assert_eq!(tokens.load(Ordering::SeqCst), 1, "el token se reutiliza");
}

#[test]
fn web_api_old_format_still_works() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|method, path, _| {
        if method == "POST" {
            return (
                200,
                vec![],
                json!({"access_token":"tok","expires_in":3600}).to_string(),
            );
        }
        assert_eq!(path, format!("/v1/playlists/{ID}"));
        let body = json!({"name":"Vieja","tracks":{"total":2,"next":null,
            "items":[{"track":track_json(1)},{"track":null}]}});
        (200, vec![], body.to_string())
    }));
    let api = WebApi::with_endpoints(
        reqwest::Client::new(),
        creds(),
        format!("{base}/v1"),
        format!("{base}/token"),
    );
    let imp = rt.block_on(api.fetch(ID)).unwrap();
    assert_eq!(imp.playlist.tracks.len(), 1);
}

#[test]
fn cascade_falls_back_to_embed_when_api_hides_tracks() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|method, path, _| {
        if method == "POST" {
            return (200, vec![], json!({"access_token":"tok","expires_in":3600}).to_string());
        }
        if path.starts_with("/v1/") {
            // Development Mode: metadatos sin canciones y /items prohibido.
            if path.ends_with(ID) {
                return (200, vec![], json!({"name":"Ajena"}).to_string());
            }
            return (403, vec![], String::new());
        }
        if path == format!("/embed/playlist/{ID}") {
            let tracks: Vec<Value> = (0..3)
                .map(|i| json!({"uri":format!("spotify:track:{i:022}"),"title":format!("T{i}"),"subtitle":"A,\u{a0}B","duration":1000}))
                .collect();
            let data = json!({"props":{"pageProps":{"state":{"data":{"entity":{"name":"Desde embed","trackList":tracks}}}}}});
            let html = format!(
                "<html><script id=\"__NEXT_DATA__\" type=\"application/json\">{data}</script></html>"
            );
            return (200, vec![], html);
        }
        (404, vec![], String::new())
    }));
    let client = reqwest::Client::new();
    let importer = Importer::new(vec![
        Box::new(WebApi::with_endpoints(
            client.clone(),
            creds(),
            format!("{base}/v1"),
            format!("{base}/token"),
        )),
        Box::new(Embed::with_base(client, base)),
    ]);
    let link = format!("https://open.spotify.com/playlist/{ID}?si=abc123");
    let imp = rt.block_on(importer.import_link(&link)).unwrap();
    assert_eq!(imp.source, "página embed");
    assert_eq!(imp.playlist.name, "Desde embed");
    assert_eq!(imp.playlist.tracks[0].artists, ["A", "B"]);

    assert!(matches!(
        rt.block_on(importer.import_link("cualquier cosa")),
        Err(Error::InvalidLink)
    ));
}

#[test]
fn bad_credentials_and_not_found() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = rt.block_on(serve(|method, _, _| {
        if method == "POST" {
            return (400, vec![], json!({"error":"invalid_client"}).to_string());
        }
        (404, vec![], String::new())
    }));
    let client = reqwest::Client::new();
    let api = WebApi::with_endpoints(
        client.clone(),
        creds(),
        format!("{base}/v1"),
        format!("{base}/token"),
    );
    assert!(matches!(
        rt.block_on(api.fetch(ID)),
        Err(Error::BadCredentials)
    ));

    // La API da 404 (playlist editorial para una app nueva, o inexistente):
    // la cascada sigue con la embed, que decide.
    let importer = Importer::new(vec![
        Box::new(WebApi::with_endpoints(
            client.clone(),
            creds(),
            format!("{base}/v1"),
            format!("{base}/token"),
        )),
        Box::new(Embed::with_base(client.clone(), base.clone())),
    ]);
    assert!(matches!(
        rt.block_on(importer.import_id(ID)),
        Err(Error::AllFailed(_)) | Err(Error::NotFound)
    ));

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
//   SPOTIFY_CLIENT_ID=… SPOTIFY_CLIENT_SECRET=… cargo test -p surco-spotify --test import -- --ignored

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

#[test]
#[ignore = "necesita internet y credenciales de Spotify"]
fn real_web_api_import() {
    let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("SPOTIFY_CLIENT_ID"),
        std::env::var("SPOTIFY_CLIENT_SECRET"),
    ) else {
        eprintln!("SPOTIFY_CLIENT_ID/SECRET no definidos; salteado");
        return;
    };
    let playlist = std::env::var("SPOTIFY_TEST_PLAYLIST").unwrap_or_else(|_| ID.into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let api = WebApi::new(
        reqwest::Client::new(),
        Credentials {
            client_id,
            client_secret,
        },
    );
    match rt.block_on(api.fetch(&playlist)) {
        Ok(imp) => eprintln!(
            "{}: {} canciones",
            imp.playlist.name,
            imp.playlist.tracks.len()
        ),
        Err(e) => panic!("{e}"),
    }
}
