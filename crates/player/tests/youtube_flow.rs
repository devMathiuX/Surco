//! Flujo "YouTube" completo sin red: un yt-dlp falso (script de shell) devuelve
//! una URL de un servidor HTTP local con soporte de Range. La primera URL da
//! 403 (como una URL vencida) para probar la re-resolución.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use surco_core::PlayerState;
use surco_player::{Command, Options, Source};
use surco_provider::{Provider, YtDlp};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve(data: Arc<Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let data = Arc::clone(&data);
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = [0u8; 2048];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let req = String::from_utf8_lossy(&req).to_lowercase();
                if req.starts_with("get /expired") {
                    let _ = sock
                        .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .await;
                    return;
                }
                let range = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .unwrap_or("0-");
                let (a, b) = range.trim().split_once('-').unwrap();
                let a: usize = a.parse().unwrap();
                let b = b.parse::<usize>().unwrap_or(usize::MAX).min(data.len() - 1);
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {a}-{b}/{}\r\nconnection: close\r\n\r\n",
                    b + 1 - a,
                    data.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&data[a..=b]).await;
            });
        }
    });
    format!("http://{addr}")
}

#[test]
fn resolves_with_ytdlp_and_recovers_from_expired_url() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone-dash.m4a");
    let base = rt.block_on(serve(Arc::new(std::fs::read(fixture).unwrap())));

    // yt-dlp falso: la primera vez da una URL "vencida", después una buena.
    let dir = std::env::temp_dir().join(format!("surco-fake-ytdlp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("yt-dlp");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ -f '{c}' ]; then p=ok; else touch '{c}'; p=expired; fi\nprintf 'm4a\\n8.0\\nTono de prueba\\n{base}/%s/audio.m4a\\n' \"$p\"\n",
            c = dir.join("called").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

    let provider = Arc::new(Provider::new(YtDlp::new(&script)));
    let p = surco_player::spawn(
        Some(provider),
        rt.handle().clone(),
        Options { null_output: true },
    )
    .unwrap();
    p.send(Command::Load(Source::YouTube("aqz-KE-bpKQ".into())));

    let start = Instant::now();
    let s = loop {
        let s = p.status();
        if s.state == PlayerState::Playing {
            break s;
        }
        assert!(
            !matches!(s.state, PlayerState::Error(_)) && start.elapsed() < Duration::from_secs(10),
            "{s:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(s.title.as_deref(), Some("Tono de prueba"));
    assert_eq!(s.duration, Some(Duration::from_secs(8)));
    std::fs::remove_dir_all(dir).unwrap();
}
