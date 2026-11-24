//! Pruebas contra la red real. Ignoradas por defecto (dependen de internet):
//!
//! ```sh
//! cargo test -p surco-player --test network -- --ignored --nocapture
//! SURCO_TEST_YOUTUBE=aqz-KE-bpKQ cargo test -p surco-player --test network -- --ignored
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use surco_core::PlayerState;
use surco_player::{Command, Options, PlayerHandle, Source, Status};
use surco_provider::{Provider, YtDlp};

const HTTPS_M4A: &str = "https://download.samplelib.com/m4a/sample-15s.m4a";

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
            !matches!(s.state, PlayerState::Error(_)) && start.elapsed() < timeout,
            "esperando {what}; estado: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Cargar, sonar, pausar y hacer seek hacia adelante (fuera de lo ya bajado,
/// así se ejercita un pedido con `Range`).
fn play_pause_seek(p: &PlayerHandle, source: Source) {
    p.send(Command::Load(source));
    let t0 = Instant::now();
    wait_for(p, "Playing", Duration::from_secs(30), |s| {
        s.state == PlayerState::Playing
    });
    eprintln!("sonando tras {:?}", t0.elapsed());
    wait_for(p, "posición > 1 s", Duration::from_secs(10), |s| {
        s.position > Duration::from_secs(1)
    });

    p.send(Command::Pause);
    let paused = wait_for(p, "Paused", Duration::from_secs(5), |s| {
        s.state == PlayerState::Paused
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(p.status().position.abs_diff(paused.position) < Duration::from_millis(100));

    p.send(Command::Seek(Duration::from_secs(10)));
    p.send(Command::Play);
    let s = wait_for(p, "posición tras seek", Duration::from_secs(15), |s| {
        s.state == PlayerState::Playing && s.position >= Duration::from_secs(10)
    });
    eprintln!("seek ok: {:?}", s.position);
    p.send(Command::Stop);
}

#[test]
#[ignore = "necesita internet"]
fn streams_real_https_m4a() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    play_pause_seek(&p, Source::Url(HTTPS_M4A.into()));
}

#[test]
#[ignore = "necesita internet y yt-dlp; YouTube bloquea muchas IPs de datacenter"]
fn streams_real_youtube() {
    let Ok(id) = std::env::var("SURCO_TEST_YOUTUBE") else {
        eprintln!("SURCO_TEST_YOUTUBE no definido; salteado");
        return;
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let bin = YtDlp::locate(None).expect("yt-dlp en el PATH");
    let p = surco_player::spawn(
        Some(Arc::new(Provider::new(bin))),
        rt.handle().clone(),
        Options { null_output: true },
    )
    .unwrap();
    play_pause_seek(&p, Source::YouTube(id));
}
