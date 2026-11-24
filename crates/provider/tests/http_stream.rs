//! Prueba de `HttpStream` contra un servidor HTTP local con soporte de Range.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use surco_provider::{Error, HttpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Servidor mínimo: responde `Range: bytes=a-b` con 206, o 403 si `forbid`.
async fn serve(data: Arc<Vec<u8>>, forbid: bool) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits2 = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let data = Arc::clone(&data);
            let hits = Arc::clone(&hits2);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let mut req = Vec::new();
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    req.extend_from_slice(&buf[..n]);
                    if req.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                hits.fetch_add(1, Ordering::SeqCst);
                if forbid {
                    let _ = sock
                        .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .await;
                    return;
                }
                let req = String::from_utf8_lossy(&req).to_lowercase();
                let range = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .unwrap()
                    .trim()
                    .to_owned();
                let (a, b) = range.split_once('-').unwrap();
                let a: usize = a.parse().unwrap();
                let b: usize = b.parse::<usize>().unwrap().min(data.len() - 1);
                let body = &data[a..=b];
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {a}-{b}/{}\r\nconnection: close\r\n\r\n",
                    body.len(),
                    data.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body).await;
            });
        }
    });
    (format!("http://{addr}/audio.m4a"), hits)
}

fn sample(len: usize) -> Arc<Vec<u8>> {
    Arc::new((0..len).map(|i| (i * 31 % 251) as u8).collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_and_seeks_whole_file() {
    let data = sample(3 * 1024 * 1024 + 12_345);
    let (url, hits) = serve(Arc::clone(&data), false).await;
    let mut stream = HttpStream::open(reqwest::Client::new(), url).await.unwrap();
    assert_eq!(stream.len(), data.len() as u64);

    let expected = Arc::clone(&data);
    tokio::task::spawn_blocking(move || {
        // Seek hacia el final antes de que termine la descarga.
        let tail = expected.len() as u64 - 1000;
        stream.seek(SeekFrom::Start(tail)).unwrap();
        let mut end = Vec::new();
        stream.read_to_end(&mut end).unwrap();
        assert_eq!(&end[..], &expected[tail as usize..]);

        stream.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        stream.read_to_end(&mut all).unwrap();
        assert_eq!(all, *expected);
        assert_eq!(stream.progress().downloaded, expected.len() as u64);
    })
    .await
    .unwrap();
    // 7 bloques de 512 KiB: sin descargas repetidas.
    assert_eq!(hits.load(Ordering::SeqCst), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_maps_to_expired() {
    let (url, _) = serve(sample(10), true).await;
    let err = HttpStream::open(reqwest::Client::new(), url)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, Error::Expired), "{err}");
}
