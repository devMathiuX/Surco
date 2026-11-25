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

/// Pre-carga: al terminar la primera, la segunda arranca sola sin pasar por
/// `Idle` (sin silencio entre canciones).
#[test]
fn preloaded_track_follows_without_gap() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    let first = Source::File(fixture("tone.m4a"));
    let second = Source::File(fixture("tone-dash.m4a"));
    p.send(Command::Load(first.clone()));
    wait_for(&p, "Playing", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing
    });
    p.send(Command::Preload(Some(second.clone())));
    // Saltamos casi al final para no esperar 8 s.
    p.send(Command::Seek(Duration::from_secs(6)));

    let start = Instant::now();
    let mut rx = p.subscribe();
    loop {
        let s = rx.borrow_and_update().clone();
        assert_ne!(s.state, PlayerState::Idle, "pasó por Idle: hubo corte");
        assert!(!matches!(s.state, PlayerState::Error(_)), "{s:?}");
        if s.auto_advances == 1 {
            assert_eq!(s.source.as_ref(), Some(&second));
            assert_eq!(s.state, PlayerState::Playing);
            assert!(s.position < Duration::from_secs(2), "{:?}", s.position);
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "no avanzó: {s:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// `Load` de lo mismo que está pre-cargado lo aprovecha (arranca enseguida).
#[test]
fn load_reuses_preloaded_source() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    let second = Source::File(fixture("tone-dash.m4a"));
    p.send(Command::Load(Source::File(fixture("tone.m4a"))));
    wait_for(&p, "Playing", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing
    });
    p.send(Command::Preload(Some(second.clone())));
    std::thread::sleep(Duration::from_millis(500));
    p.send(Command::Load(second.clone()));
    let s = wait_for(&p, "Playing la segunda", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing && s.source.as_ref() == Some(&second)
    });
    assert_eq!(s.auto_advances, 0);
    wait_for(&p, "posición avanza", Duration::from_secs(3), |s| {
        s.position > Duration::from_millis(300)
    });
    // Y termina normalmente.
    p.send(Command::Seek(Duration::from_secs(7)));
    wait_for(&p, "fin (Idle)", Duration::from_secs(8), |s| {
        s.state == PlayerState::Idle
    });
}

/// El silencio del principio y del final del video no suena: el archivo tiene
/// 1,5 s mudos, 2 s de tono y 3 s mudos.
#[test]
fn skips_leading_and_trailing_silence() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::Load(Source::File(fixture("padded.m4a"))));
    let s = wait_for(&p, "Playing", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing
    });
    // La posición arranca donde empieza el sonido.
    assert!(
        s.position >= Duration::from_millis(1300),
        "posición {:?}",
        s.position
    );
    let start = Instant::now();
    wait_for(&p, "fin (Idle)", Duration::from_secs(6), |s| {
        s.state == PlayerState::Idle
    });
    let played = start.elapsed();
    assert!(
        played < Duration::from_millis(3200),
        "sonó {played:?}: no se salteó el silencio final"
    );
}

/// Una intro muda más larga que el silencio que se saltea y que el que se
/// retiene no tiene que dejar la canción cargando para siempre.
#[test]
fn long_quiet_intro_still_starts() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::Load(Source::File(fixture("quiet-intro.m4a"))));
    wait_for(&p, "Playing", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing
    });
    let start = wait_for(&p, "posición", Duration::from_secs(2), |s| {
        s.position > Duration::ZERO
    })
    .position;
    wait_for(&p, "posición que avanza", Duration::from_secs(3), |s| {
        s.position > start + Duration::from_millis(500)
    });
}

/// Cambiar de canción muchas veces seguidas (mientras la anterior todavía
/// abre) no tiene que dejar la última "sonando" en silencio.
#[test]
fn rapid_switches_still_play_the_last_one() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    let a = Source::File(fixture("tone.m4a"));
    let b = Source::File(fixture("padded.m4a"));
    for round in 0..6 {
        p.send(Command::Load(a.clone()));
        if round % 2 == 0 {
            std::thread::sleep(Duration::from_millis(30));
        }
        p.send(Command::Load(b.clone()));
        std::thread::sleep(Duration::from_millis(round * 15));
    }
    p.send(Command::Load(a));
    wait_for(&p, "Playing", Duration::from_secs(5), |s| {
        s.state == PlayerState::Playing
    });
    wait_for(&p, "posición que avanza", Duration::from_secs(3), |s| {
        s.position > Duration::from_millis(700)
    });
}

/// Una carga que falla justo después de otra (dos vaciados de la salida
/// seguidos) no tiene que comerse la canción que se pone después.
#[test]
fn song_after_a_failed_load_plays() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    let tone = Source::File(fixture("tone.m4a"));
    for _ in 0..5 {
        p.send(Command::Load(tone.clone()));
        wait_for(&p, "Playing", Duration::from_secs(5), |s| {
            s.state == PlayerState::Playing
        });
        p.send(Command::Load(Source::File(fixture("no-existe.m4a"))));
        wait_for(&p, "Error", Duration::from_secs(5), |s| {
            matches!(s.state, PlayerState::Error(_))
        });
        p.send(Command::Load(tone.clone()));
        wait_for(&p, "Playing", Duration::from_secs(5), |s| {
            s.state == PlayerState::Playing
        });
        wait_for(&p, "posición que avanza", Duration::from_secs(3), |s| {
            s.position > Duration::from_millis(400)
        });
    }
}

/// Retomar la última canción: carga en pausa donde había quedado y suena
/// desde ahí al darle a play.
#[test]
fn load_at_resumes_paused_where_it_was() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let p = surco_player::spawn(None, rt.handle().clone(), Options { null_output: true }).unwrap();
    p.send(Command::LoadAt {
        source: Source::File(fixture("tone.m4a")),
        at: Duration::from_secs(4),
        paused: true,
    });
    let s = wait_for(&p, "Paused", Duration::from_secs(5), |s| {
        s.state == PlayerState::Paused
    });
    assert!(
        s.position >= Duration::from_secs(4) && s.position < Duration::from_millis(4500),
        "posición {:?}",
        s.position
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(p.status().state, PlayerState::Paused);
    p.send(Command::Play);
    wait_for(&p, "suena desde ~4 s", Duration::from_secs(3), |s| {
        s.state == PlayerState::Playing && s.position > Duration::from_millis(4200)
    });
}
