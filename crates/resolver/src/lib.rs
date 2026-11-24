//! Resolución Spotify -> YouTube: búsqueda con yt-dlp + scoring.
//!
//! Para cada canción se arma la query `"{artista principal} - {título limpio}"`,
//! se buscan 5 candidatos y se puntúa cada uno (0-100, ver [`score`]). Si el
//! mejor queda por debajo de [`DOUBTFUL`], se hace una segunda búsqueda
//! (`"… audio"`) y se suman sus candidatos. Los matches se cachean en memoria
//! (en disco desde la Fase 6). Como mucho [`MAX_CONCURRENT`] búsquedas a la vez.

pub use surco_core as core;

pub mod clean;
pub mod score;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use surco_core::{Resolution, Track};
use surco_provider::YtDlp;
use tokio::sync::Semaphore;

pub use clean::{clean_title, CleanTitle};
pub use score::Breakdown;

/// Errores de este crate.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("búsqueda en YouTube: {0}")]
    Search(String),
    #[error("no encontré ningún video para esta canción")]
    NoResults,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Por debajo de este puntaje el match es dudoso.
pub const DOUBTFUL: u8 = Resolution::DOUBTFUL_THRESHOLD;
/// Resultados por búsqueda.
pub const SEARCH_RESULTS: usize = 5;
/// Búsquedas simultáneas como máximo.
pub const MAX_CONCURRENT: usize = 3;

/// Un video candidato.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub channel: Option<String>,
    pub duration_secs: Option<f64>,
    pub views: Option<u64>,
}

/// Candidato con su puntaje.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub candidate: Candidate,
    pub score: u8,
    pub breakdown: Breakdown,
}

/// Resultado de resolver una canción.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// Video elegido (el mejor, o el que eligió el usuario).
    pub youtube_id: String,
    pub score: u8,
    /// `true` si lo eligió el usuario a mano.
    pub manual: bool,
    /// Todos los candidatos, de mejor a peor.
    pub candidates: Vec<Scored>,
}

impl Match {
    pub fn is_doubtful(&self) -> bool {
        !self.manual && self.score < DOUBTFUL
    }

    pub fn resolution(&self) -> Resolution {
        Resolution::Resolved {
            youtube_id: self.youtube_id.clone(),
            // Lo elegido a mano cuenta como seguro.
            score: if self.manual { 100 } else { self.score },
        }
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Algo que busca videos. En la app es yt-dlp; en los tests, un falso.
pub trait Searcher: Send + Sync {
    fn search<'a>(&'a self, query: &'a str, n: usize) -> BoxFuture<'a, Result<Vec<Candidate>>>;
}

impl Searcher for YtDlp {
    fn search<'a>(&'a self, query: &'a str, n: usize) -> BoxFuture<'a, Result<Vec<Candidate>>> {
        Box::pin(async move {
            let results = YtDlp::search(self, query, n)
                .await
                .map_err(|e| Error::Search(e.to_string()))?;
            Ok(results
                .into_iter()
                .map(|r| Candidate {
                    id: r.id,
                    title: r.title,
                    channel: r.channel,
                    duration_secs: r.duration.map(|d: Duration| d.as_secs_f64()),
                    views: r.views,
                })
                .collect())
        })
    }
}

/// Query de búsqueda: `"{artista principal} - {título limpio}"`.
pub fn build_query(track: &Track) -> String {
    let clean = clean_title(&track.title);
    match track.main_artist() {
        "" => clean.title,
        artist => format!("{artist} - {}", clean.title),
    }
}

/// Clave de caché: el ID de Spotify, o artista|título|duración.
pub fn cache_key(track: &Track) -> String {
    track.spotify_id.clone().unwrap_or_else(|| {
        format!(
            "{}|{}|{}",
            track.main_artist().to_lowercase(),
            track.title.to_lowercase(),
            track.duration_ms / 1000
        )
    })
}

/// Puntúa y ordena candidatos (mejor primero; a igual puntaje, más vistas).
pub fn rank(track: &Track, candidates: Vec<Candidate>) -> Vec<Scored> {
    let clean = clean_title(&track.title);
    let mut scored: Vec<Scored> = candidates
        .into_iter()
        .map(|c| {
            let breakdown = score::score(track, &clean, &c);
            Scored {
                score: breakdown.total(),
                breakdown,
                candidate: c,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score.cmp(&a.score).then(
            b.candidate
                .views
                .unwrap_or(0)
                .cmp(&a.candidate.views.unwrap_or(0)),
        )
    });
    scored
}

/// Resolver con caché en memoria y límite de concurrencia.
pub struct Resolver {
    searcher: Arc<dyn Searcher>,
    cache: Mutex<HashMap<String, Match>>,
    limit: Semaphore,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver").finish_non_exhaustive()
    }
}

impl Resolver {
    pub fn new(searcher: Arc<dyn Searcher>) -> Self {
        Self {
            searcher,
            cache: Mutex::new(HashMap::new()),
            limit: Semaphore::new(MAX_CONCURRENT),
        }
    }

    /// Match cacheado, si hay.
    pub fn cached(&self, track: &Track) -> Option<Match> {
        self.cache.lock().ok()?.get(&cache_key(track)).cloned()
    }

    /// Resuelve una canción (o devuelve el match cacheado).
    pub async fn resolve(&self, track: &Track) -> Result<Match> {
        if let Some(m) = self.cached(track) {
            return Ok(m);
        }
        let _permit = self
            .limit
            .acquire()
            .await
            .map_err(|e| Error::Search(e.to_string()))?;
        // Otra tarea pudo resolverla mientras esperábamos.
        if let Some(m) = self.cached(track) {
            return Ok(m);
        }

        let query = build_query(track);
        let mut candidates = match self.searcher.search(&query, SEARCH_RESULTS).await {
            Ok(c) => c,
            // YouTube a veces corta una búsqueda (403/429 sueltos): un reintento.
            Err(e) => {
                tracing::debug!(error = %e, "falló la búsqueda, reintento");
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                self.searcher.search(&query, SEARCH_RESULTS).await?
            }
        };
        let mut ranked = rank(track, candidates.clone());

        if ranked.first().is_none_or(|b| b.score < DOUBTFUL) {
            // Segunda vuelta: suele traer el "Topic" o un "Official Audio".
            let retry = format!("{query} audio");
            tracing::debug!(%retry, "match dudoso, busco de nuevo");
            if let Ok(more) = self.searcher.search(&retry, SEARCH_RESULTS).await {
                for c in more {
                    if !candidates.iter().any(|x| x.id == c.id) {
                        candidates.push(c);
                    }
                }
                ranked = rank(track, candidates);
            }
        }

        let best = ranked.first().ok_or(Error::NoResults)?;
        let m = Match {
            youtube_id: best.candidate.id.clone(),
            score: best.score,
            manual: false,
            candidates: ranked,
        };
        tracing::info!(%query, id = %m.youtube_id, score = m.score, "canción resuelta");
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(cache_key(track), m.clone());
        }
        Ok(m)
    }

    /// El usuario eligió otro candidato a mano.
    pub fn choose(&self, track: &Track, youtube_id: &str) -> Option<Match> {
        let mut cache = self.cache.lock().ok()?;
        let m = cache.get_mut(&cache_key(track))?;
        let chosen = m.candidates.iter().find(|c| c.candidate.id == youtube_id)?;
        m.score = chosen.score;
        m.youtube_id = youtube_id.to_owned();
        m.manual = true;
        Some(m.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn track(title: &str, artist: &str, secs: u64, id: &str) -> Track {
        Track {
            title: title.into(),
            artists: vec![artist.into()],
            album: None,
            duration_ms: secs * 1000,
            isrc: None,
            spotify_id: Some(id.into()),
        }
    }

    fn cand(id: &str, title: &str, channel: &str, secs: f64) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            channel: Some(channel.into()),
            duration_secs: Some(secs),
            views: None,
        }
    }

    /// Buscador falso: primera búsqueda solo trae basura; la de "audio" trae el bueno.
    struct Fake {
        calls: AtomicUsize,
        running: AtomicUsize,
        max_running: AtomicUsize,
    }

    impl Searcher for Fake {
        fn search<'a>(&'a self, query: &'a str, _: usize) -> BoxFuture<'a, Result<Vec<Candidate>>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_running.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                self.running.fetch_sub(1, Ordering::SeqCst);
                if query.ends_with(" audio") {
                    return Ok(vec![cand("good", "Song", "Artist - Topic", 200.0)]);
                }
                Ok(vec![
                    cand("live", "Artist - Song (Live)", "Fan", 260.0),
                    cand("cover", "Song (cover)", "Someone", 200.0),
                ])
            })
        }
    }

    fn fake() -> Arc<Fake> {
        Arc::new(Fake {
            calls: AtomicUsize::new(0),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        })
    }

    #[test]
    fn query_uses_clean_title() {
        let t = track("Under Pressure - Remastered 2011", "Queen", 248, "x");
        assert_eq!(build_query(&t), "Queen - Under Pressure");
        let t = track("Bad Guy (feat. Justin Bieber)", "Billie Eilish", 194, "x");
        assert_eq!(build_query(&t), "Billie Eilish - Bad Guy");
    }

    #[tokio::test]
    async fn retries_when_doubtful_and_caches() {
        let f = fake();
        let r = Resolver::new(f.clone());
        let t = track("Song", "Artist", 200, "abc");
        let m = r.resolve(&t).await.unwrap();
        assert_eq!(m.youtube_id, "good");
        assert!(!m.is_doubtful());
        assert_eq!(m.candidates.len(), 3);
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
        // Segunda vez: desde la caché, sin buscar.
        r.resolve(&t).await.unwrap();
        assert_eq!(f.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn limits_concurrency() {
        let f = fake();
        let r = Arc::new(Resolver::new(f.clone()));
        let jobs: Vec<_> = (0..8)
            .map(|i| {
                let r = r.clone();
                tokio::spawn(async move {
                    r.resolve(&track("Song", "Artist", 200, &format!("id{i}")))
                        .await
                })
            })
            .collect();
        for j in jobs {
            j.await.unwrap().unwrap();
        }
        assert!(f.max_running.load(Ordering::SeqCst) <= MAX_CONCURRENT);
    }

    #[tokio::test]
    async fn manual_choice() {
        let r = Resolver::new(fake());
        let t = track("Song", "Artist", 200, "abc");
        r.resolve(&t).await.unwrap();
        let m = r.choose(&t, "cover").unwrap();
        assert_eq!(m.youtube_id, "cover");
        assert!(m.manual && !m.is_doubtful());
        assert!(r.choose(&t, "nope").is_none());
        assert_eq!(r.cached(&t).unwrap().youtube_id, "cover");
    }
}
