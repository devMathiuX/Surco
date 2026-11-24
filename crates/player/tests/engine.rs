//! Prueba de punta a punta del motor con salida nula: cargar, sonar, pausar,
//! seek y fin de canción. Usa archivos AAC generados con ffmpeg (ver README).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use surco_core::PlayerState;
use surco_player::{Command, Options, PlayerHandle, Source, Status};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn wait_for(
    p: &PlayerHandle,
    what: &str,
    timeout: Duration,
    pred: impl Fn(&Status) -> bool,
) -> Status {
    let start = Instant::now();
    loop {
        let s = p.status();
        if pred(&s) {
            return s;
        }
        assert!(
            start.elapsed() < timeout,
            "timeout esperando {what}; estado: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_scenario(rt: &tokio::runtime::Runtime, source: Source, expect_duration: bool) {
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::Load(source));

    let s = wait_for(&p, "Playing", Duration::from_secs(10), |s| {
        s.state == PlayerState::Playing
    });
    // En MP4 fragmentado (como los de YouTube) el contenedor no trae la
    // duración: en la app la aporta yt-dlp.
    if expect_duration {
        let d = s.duration.expect("duración conocida");
        assert!((d.as_secs_f32() - 8.0).abs() < 0.2, "duración {d:?}");
    }

    // La posición avanza en tiempo real.
    wait_for(&p, "posición > 0.5 s", Duration::from_secs(5), |s| {
        s.position > Duration::from_millis(500)
    });

    // Pausa: la posición queda quieta.
    p.send(Command::Pause);
    let paused = wait_for(&p, "Paused", Duration::from_secs(2), |s| {
        s.state == PlayerState::Paused
    });
    std::thread::sleep(Duration::from_millis(400));
    let later = p.status().position;
    assert!(
        later.saturating_sub(paused.position) < Duration::from_millis(150),
        "avanzó en pausa: {:?} -> {later:?}",
        paused.position
    );

    // Seek mientras está en pausa, después reanudar.
    p.send(Command::Seek(Duration::from_secs(5)));
    p.send(Command::Play);
    let s = wait_for(&p, "posición ~5 s", Duration::from_secs(3), |s| {
        s.state == PlayerState::Playing && s.position >= Duration::from_secs(5)
    });
    assert!(
        s.position < Duration::from_millis(6500),
        "seek impreciso: {:?}",
        s.position
    );

    // Llega al final y vuelve a Idle.
    wait_for(&p, "fin (Idle)", Duration::from_secs(8), |s| {
        s.state == PlayerState::Idle
    });
}

#[test]
fn plays_pauses_and_seeks_local_file() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    run_scenario(&rt, Source::File(fixture("tone.m4a")), true);
}

#[test]
fn plays_fragmented_mp4_like_youtube() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    run_scenario(&rt, Source::File(fixture("tone-dash.m4a")), false);
}

#[test]
fn missing_file_is_an_error_not_a_crash() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::Load(Source::File("/no/existe.m4a".into())));
    wait_for(&p, "Error", Duration::from_secs(5), |s| {
        matches!(s.state, PlayerState::Error(_))
    });
}

#[test]
fn youtube_without_ytdlp_is_an_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::Load(Source::YouTube("aqz-KE-bpKQ".into())));
    let s = wait_for(&p, "Error", Duration::from_secs(5), |s| {
        matches!(s.state, PlayerState::Error(_))
    });
    assert!(matches!(s.state, PlayerState::Error(e) if e.contains("yt-dlp")));
}
