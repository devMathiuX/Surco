//! Evalúa el resolver Spotify -> YouTube con búsquedas reales (yt-dlp).
//!
//! ```sh
//! cargo run -p eval_resolver --release
//! cargo run -p eval_resolver --release -- --report /tmp/report.tsv
//! ```
//!
//! - `songs.csv`: las canciones (formato Exportify: `Track URI`, `Track Name`,
//!   `Artist Name(s)`, `Duration (ms)`), tomadas tal cual de playlists públicas.
//! - `labels.txt`: para cada canción, los videos que cuentan como correctos
//!   (la grabación de estudio correcta: canal "Topic", audio oficial, video
//!   con letra, o video oficial con la misma duración). Una línea por canción:
//!   `<spotify_id> <video_id>[,<video_id>…]  # comentario`.
//!
//! Un match es correcto si el video elegido está en las etiquetas. El
//! criterio de aceptación de la Fase 4 es ≥ 90 %. Sale con código 1 si no se
//! llega.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context};
use surco_provider::YtDlp;
use surco_resolver::{Resolver, DOUBTFUL};

const TARGET: f64 = 0.90;

fn main() -> anyhow::Result<()> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut songs = dir.join("songs.csv");
    let mut labels = dir.join("labels.txt");
    let mut report: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--songs" => songs = args.next().context("falta la ruta de --songs")?.into(),
            "--labels" => labels = args.next().context("falta la ruta de --labels")?.into(),
            "--report" => report = Some(args.next().context("falta la ruta de --report")?.into()),
            "-h" | "--help" => {
                println!("uso: eval_resolver [--songs songs.csv] [--labels labels.txt] [--report salida.tsv]");
                return Ok(());
            }
            other => bail!("argumento desconocido: {other}"),
        }
    }

    let playlist = surco_spotify::import_csv_file(&songs)
        .with_context(|| format!("leyendo {}", songs.display()))?
        .playlist;
    let labels = read_labels(&labels)?;
    let ytdlp = YtDlp::locate(None).context("no encontré yt-dlp en el PATH")?;
    let resolver = Arc::new(Resolver::new(Arc::new(ytdlp)));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    let start = Instant::now();
    // Todas a la vez: el resolver limita a 3 búsquedas simultáneas.
    let results = rt.block_on(async {
        let jobs: Vec<_> = playlist
            .tracks
            .iter()
            .cloned()
            .map(|t| {
                let r = resolver.clone();
                tokio::spawn(async move {
                    let m = r.resolve(&t).await;
                    (t, m)
                })
            })
            .collect();
        let mut out = Vec::new();
        for j in jobs {
            out.push(j.await?);
        }
        anyhow::Ok(out)
    })?;

    let (mut ok, mut bad, mut unlabeled, mut doubtful) = (0usize, 0usize, 0usize, 0usize);
    let mut tsv = String::from(
        "estado\tscore\tspotify_id\tcanción\tvideo\ttítulo\tcanal\tduración\totros candidatos\n",
    );
    for (i, (t, m)) in results.iter().enumerate() {
        let id = t.spotify_id.clone().unwrap_or_default();
        let song = format!("{} - {}", t.artists.join(", "), t.title);
        let m = match m {
            Ok(m) => m,
            Err(e) => {
                bad += 1;
                println!("{:>2}. ✗ ERROR {song}: {e}", i + 1);
                let _ = writeln!(tsv, "error\t\t{id}\t{song}\t\t{e}\t\t\t");
                continue;
            }
        };
        let best = &m.candidates[0];
        let status = match labels.get(&id) {
            Some(good) if good.contains(&m.youtube_id) => {
                ok += 1;
                "✓"
            }
            Some(_) => {
                bad += 1;
                "✗"
            }
            None => {
                unlabeled += 1;
                "?"
            }
        };
        if m.score < DOUBTFUL {
            doubtful += 1;
        }
        let dur = best.candidate.duration_secs.unwrap_or(0.0);
        println!(
            "{:>2}. {status} {:>3} {song}\n         -> {} | {} | {}s (spotify {}s)",
            i + 1,
            m.score,
            best.candidate.title,
            best.candidate.channel.as_deref().unwrap_or("?"),
            dur.round(),
            t.duration_ms / 1000,
        );
        let others: Vec<String> = m.candidates[1..]
            .iter()
            .map(|c| {
                format!(
                    "{} [{}] {} | {} | {}s",
                    c.candidate.id,
                    c.score,
                    c.candidate.title,
                    c.candidate.channel.as_deref().unwrap_or("?"),
                    c.candidate.duration_secs.unwrap_or(0.0).round()
                )
            })
            .collect();
        let _ = writeln!(
            tsv,
            "{status}\t{}\t{id}\t{song}\t{}\t{}\t{}\t{}\t{}",
            m.score,
            m.youtube_id,
            best.candidate.title,
            best.candidate.channel.as_deref().unwrap_or(""),
            dur.round(),
            others.join(" || ")
        );
    }

    let labeled = ok + bad;
    let acc = if labeled > 0 {
        ok as f64 / labeled as f64
    } else {
        0.0
    };
    println!(
        "\n{ok}/{labeled} correctas ({:.1} %), {unlabeled} sin etiqueta, {doubtful} dudosas (< {DOUBTFUL}), {:.1} s",
        acc * 100.0,
        start.elapsed().as_secs_f32()
    );
    if let Some(path) = report {
        std::fs::write(&path, tsv)?;
        println!("reporte: {}", path.display());
    }
    if labeled > 0 && acc < TARGET {
        println!("No se llega al {:.0} % pedido.", TARGET * 100.0);
        std::process::exit(1);
    }
    Ok(())
}

fn read_labels(path: &PathBuf) -> anyhow::Result<HashMap<String, HashSet<String>>> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(HashMap::new());
    };
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        let mut parts = line.split_whitespace();
        let (Some(id), Some(videos)) = (parts.next(), parts.next()) else {
            continue;
        };
        map.insert(
            id.to_owned(),
            videos.split(',').map(|v| v.trim().to_owned()).collect(),
        );
    }
    Ok(map)
}
