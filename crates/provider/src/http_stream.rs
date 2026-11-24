//! Lector HTTP en streaming con soporte de `Range`.
//!
//! Una tarea de tokio baja el archivo en bloques con pedidos `Range` (YouTube
//! limita la velocidad de las descargas sin rango). Los bloques se guardan en
//! memoria. El lector ([`HttpStream`]) es bloqueante e implementa
//! `Read + Seek`: si se pide una posición que todavía no llegó, le da
//! prioridad a ese bloque y espera. Así el seek funciona antes de que termine
//! la descarga.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use tokio::task::JoinHandle;

use crate::{Error, Result};

/// Tamaño de cada pedido `Range`.
const CHUNK: usize = 512 * 1024;
/// Reintentos por bloque ante errores de red.
const RETRIES: u32 = 3;
/// Cuánto espera el lector un bloque antes de rendirse.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Progreso de la descarga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamProgress {
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
enum Failure {
    Expired,
    Other(String),
}

struct State {
    data: Vec<u8>,
    have: Vec<bool>,
    /// Bloque que el lector está esperando (prioridad para el descargador).
    want: Option<usize>,
    failure: Option<Failure>,
    closed: bool,
    downloaded: u64,
}

struct Shared {
    state: Mutex<State>,
    cond: Condvar,
}

impl Shared {
    fn lock(&self) -> io::Result<MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("estado del stream envenenado"))
    }
}

/// Archivo remoto leído en streaming. Ver la documentación del módulo.
pub struct HttpStream {
    shared: Arc<Shared>,
    len: u64,
    pos: u64,
    task: JoinHandle<()>,
}

impl HttpStream {
    /// Abre `url`: baja el primer bloque (para conocer el tamaño total) y
    /// lanza la descarga del resto en segundo plano. Debe llamarse dentro de
    /// un runtime de tokio.
    pub async fn open(client: reqwest::Client, url: String) -> Result<Self> {
        let first = fetch_range(&client, &url, 0, CHUNK as u64 - 1).await?;
        let len = first.total;
        let chunks = (len as usize).div_ceil(CHUNK);

        let mut data = vec![0u8; len as usize];
        let mut have = vec![false; chunks];
        let n = first.body.len().min(data.len());
        data[..n].copy_from_slice(&first.body[..n]);
        // Si el servidor ignoró el Range, `body` es el archivo entero.
        for (i, h) in have.iter_mut().enumerate() {
            let end = ((i + 1) * CHUNK).min(len as usize);
            *h = end <= n;
        }

        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                data,
                have,
                want: None,
                failure: None,
                closed: false,
                downloaded: n as u64,
            }),
            cond: Condvar::new(),
        });
        let task = tokio::spawn(download(client, url, Arc::clone(&shared), len));
        tracing::debug!(len, chunks, "stream HTTP abierto");
        Ok(Self {
            shared,
            len,
            pos: 0,
            task,
        })
    }

    /// Tamaño total en bytes.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn progress(&self) -> StreamProgress {
        let downloaded = self.shared.lock().map(|s| s.downloaded).unwrap_or(0);
        StreamProgress {
            downloaded,
            total: self.len,
        }
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.closed = true;
        }
        self.task.abort();
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let chunk = (self.pos / CHUNK as u64) as usize;
        let mut state = self.shared.lock()?;
        let mut waited = Duration::ZERO;
        while !state.have.get(chunk).copied().unwrap_or(false) {
            match &state.failure {
                Some(Failure::Expired) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        Error::Expired.to_string(),
                    ))
                }
                Some(Failure::Other(msg)) => return Err(io::Error::other(msg.clone())),
                None => {}
            }
            if waited >= READ_TIMEOUT {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "el stream no respondió a tiempo",
                ));
            }
            state.want = Some(chunk);
            let step = Duration::from_millis(250);
            state = self
                .shared
                .cond
                .wait_timeout(state, step)
                .map_err(|_| io::Error::other("estado del stream envenenado"))?
                .0;
            waited += step;
        }
        let start = self.pos as usize;
        let chunk_end = ((chunk + 1) * CHUNK).min(self.len as usize);
        let n = buf.len().min(chunk_end - start);
        buf[..n].copy_from_slice(&state.data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(off) => self.len.checked_add_signed(off),
            SeekFrom::Current(off) => self.pos.checked_add_signed(off),
        };
        let target = target.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "seek a posición negativa")
        })?;
        self.pos = target;
        Ok(target)
    }
}

/// Tarea de descarga: baja los bloques que faltan, priorizando el que pida
/// el lector.
async fn download(client: reqwest::Client, url: String, shared: Arc<Shared>, len: u64) {
    let mut cursor = 0usize;
    loop {
        let next = {
            let Ok(state) = shared.lock() else { return };
            if state.closed {
                return;
            }
            match pick_next(&state.have, state.want, cursor) {
                Some(c) => c,
                None => {
                    tracing::debug!(len, "descarga completa");
                    return;
                }
            }
        };

        let start = (next * CHUNK) as u64;
        let end = (start + CHUNK as u64).min(len) - 1;
        let result = fetch_with_retries(&client, &url, start, end).await;

        let Ok(mut state) = shared.lock() else { return };
        match result {
            Ok(body) => {
                let end = (start as usize + body.len()).min(state.data.len());
                let n = end - start as usize;
                state.data[start as usize..end].copy_from_slice(&body[..n]);
                state.have[next] = true;
                state.downloaded += n as u64;
                if state.want == Some(next) {
                    state.want = None;
                }
                cursor = next + 1;
            }
            Err(e) => {
                tracing::warn!(error = %e, chunk = next, "falló la descarga del stream");
                state.failure = Some(match e {
                    Error::Expired => Failure::Expired,
                    other => Failure::Other(other.to_string()),
                });
                shared.cond.notify_all();
                return;
            }
        }
        drop(state);
        shared.cond.notify_all();
    }
}

/// Elige el próximo bloque: el pedido por el lector, o el primero que falte
/// desde `cursor` (dando la vuelta).
fn pick_next(have: &[bool], want: Option<usize>, cursor: usize) -> Option<usize> {
    if let Some(w) = want.filter(|&w| have.get(w) == Some(&false)) {
        return Some(w);
    }
    let n = have.len();
    (0..n).map(|i| (cursor + i) % n).find(|&i| !have[i])
}

async fn fetch_with_retries(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>> {
    let mut attempt = 0;
    loop {
        match fetch_range(client, url, start, end).await {
            Ok(r) => return Ok(r.body),
            Err(e @ (Error::Expired | Error::Status(_))) => return Err(e),
            Err(e) if attempt + 1 >= RETRIES => return Err(e),
            Err(e) => {
                attempt += 1;
                tracing::debug!(error = %e, attempt, "reintentando bloque");
                tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
            }
        }
    }
}

struct RangeResponse {
    body: Vec<u8>,
    total: u64,
}

async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<RangeResponse> {
    let resp = client
        .get(url)
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?;
    let status = resp.status();
    if status == StatusCode::FORBIDDEN || status == StatusCode::GONE {
        return Err(Error::Expired);
    }
    if !status.is_success() {
        return Err(Error::Status(status.as_u16()));
    }
    let header = |name| {
        resp.headers()
            .get(name)
            .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
            .map(str::to_owned)
    };
    let total = if status == StatusCode::PARTIAL_CONTENT {
        header(CONTENT_RANGE)
            .as_deref()
            .and_then(parse_content_range_total)
    } else {
        header(CONTENT_LENGTH).and_then(|v| v.parse().ok())
    };
    let body = resp.bytes().await?.to_vec();
    let total = total.unwrap_or(body.len() as u64);
    Ok(RangeResponse { body, total })
}

/// `bytes 0-524287/3456789` -> `3456789`.
fn parse_content_range_total(v: &str) -> Option<u64> {
    v.rsplit_once('/')?.1.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-99/1234"), Some(1234));
        assert_eq!(parse_content_range_total("bytes 0-99/*"), None);
        assert_eq!(parse_content_range_total("garbage"), None);
    }

    #[test]
    fn picks_wanted_chunk_first() {
        let have = [true, false, false, false];
        assert_eq!(pick_next(&have, Some(3), 1), Some(3));
        assert_eq!(pick_next(&have, Some(0), 1), Some(1));
        assert_eq!(pick_next(&have, None, 2), Some(2));
        assert_eq!(pick_next(&[true, false, true], None, 2), Some(1));
        assert_eq!(pick_next(&[true, true], None, 0), None);
    }
}
