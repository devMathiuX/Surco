//! Puntaje (0-100) de un video de YouTube como match de una canción.

use strsim::jaro_winkler;
use surco_core::Track;

use crate::clean::{allowed_by, contains_word, normalize, CleanTitle, VERSION_WORDS};
use crate::Candidate;

/// Desglose del puntaje, útil para depurar y para `tools/eval_resolver`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Breakdown {
    pub duration: f32,
    pub title: f32,
    pub artist: f32,
    pub channel: f32,
    pub penalty: f32,
}

impl Breakdown {
    pub fn total(&self) -> u8 {
        let t = self.duration + self.title + self.artist + self.channel - self.penalty;
        t.round().clamp(0.0, 100.0) as u8
    }
}

/// Pesos máximos de cada parte.
const W_DURATION: f32 = 35.0;
const W_TITLE: f32 = 30.0;
const W_ARTIST: f32 = 20.0;
const W_CHANNEL: f32 = 15.0;

/// Calcula el puntaje de `cand` para `track` (con su título ya limpio).
pub fn score(track: &Track, clean: &CleanTitle, cand: &Candidate) -> Breakdown {
    let cand_title = normalize(&cand.title);
    let channel_raw = cand.channel.as_deref().unwrap_or_default();
    let channel = normalize(channel_raw);
    let artist = normalize(track.main_artist());
    let all_artists: Vec<String> = track
        .artists
        .iter()
        .chain(&clean.featured)
        .map(|a| normalize(a))
        .filter(|a| !a.is_empty())
        .collect();

    Breakdown {
        duration: duration_score(track.duration_ms, cand.duration_secs),
        // El título completo o, si tiene parte de versión, solo el nombre.
        title: title_score(&normalize(&clean.title), &cand_title, &all_artists).max(title_score(
            &normalize(&clean.base),
            &cand_title,
            &all_artists,
        )),
        artist: artist_score(&artist, &cand_title, &channel),
        channel: channel_score(&artist, channel_raw, &channel, &cand_title),
        penalty: penalty(clean, &cand.title.to_lowercase(), &channel),
    }
}

/// ±3 s: puntaje máximo. Baja lineal hasta 15 s. Más de 15 s: nada, y más
/// de 30 s resta (casi seguro es otra versión: video con intro, en vivo…).
fn duration_score(track_ms: u64, cand_secs: Option<f64>) -> f32 {
    let Some(secs) = cand_secs else {
        return W_DURATION * 0.4;
    };
    if track_ms == 0 {
        return W_DURATION * 0.4;
    }
    let diff = (track_ms as f64 / 1000.0 - secs).abs() as f32;
    match diff {
        d if d <= 3.0 => W_DURATION,
        d if d <= 15.0 => W_DURATION * (1.0 - (d - 3.0) / 12.0 * 0.7),
        d if d <= 30.0 => 0.0,
        _ => -20.0,
    }
}

/// Qué tanto el título del video contiene el de la canción.
fn title_score(clean: &str, cand: &str, artists: &[String]) -> f32 {
    if clean.is_empty() {
        return 0.0;
    }
    if contains_word(cand, clean) {
        return W_TITLE;
    }
    // Sacamos los artistas del título del video ("Queen - Under Pressure")
    // y comparamos lo que queda.
    let mut rest = cand.to_owned();
    for a in artists {
        if !a.is_empty() {
            rest = rest.replace(a.as_str(), " ");
        }
    }
    let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    let sim = jaro_winkler(clean, &rest).max(token_overlap(clean, cand)) as f32;
    // Por debajo de 0.6 es prácticamente otra canción.
    W_TITLE * ((sim - 0.6) / 0.4).clamp(0.0, 1.0)
}

/// Proporción de palabras del título de la canción presentes en el video.
fn token_overlap(clean: &str, cand: &str) -> f64 {
    let words: Vec<&str> = clean.split_whitespace().collect();
    if words.is_empty() {
        return 0.0;
    }
    let hits = words.iter().filter(|w| contains_word(cand, w)).count();
    hits as f64 / words.len() as f64
}

/// El artista principal aparece en el título o en el canal.
fn artist_score(artist: &str, cand_title: &str, channel: &str) -> f32 {
    if artist.is_empty() {
        return W_ARTIST * 0.5;
    }
    if contains_word(cand_title, artist) || contains_word(channel, artist) {
        return W_ARTIST;
    }
    let sim = jaro_winkler(artist, channel).max(token_overlap(artist, cand_title)) as f32;
    W_ARTIST * ((sim - 0.7) / 0.3).clamp(0.0, 1.0)
}

/// Canal "Artista - Topic" (YouTube Music), VEVO, canal oficial del artista,
/// o "official audio" en el título.
fn channel_score(artist: &str, channel_raw: &str, channel: &str, cand_title: &str) -> f32 {
    let artist_channel = !artist.is_empty()
        && (channel == artist
            || channel.strip_suffix(" topic") == Some(artist)
            || channel.strip_suffix("vevo").map(str::trim)
                == Some(artist.replace(' ', "").as_str())
            || channel.replace(' ', "") == format!("{}vevo", artist.replace(' ', "")));
    let mut s: f32 = 0.0;
    if channel_raw.ends_with(" - Topic") {
        s = if artist_channel {
            W_CHANNEL
        } else {
            W_CHANNEL * 0.5
        };
    } else if channel.ends_with("vevo") || artist_channel {
        s = W_CHANNEL * 0.8;
    } else if channel.contains("official") || channel.contains("oficial") {
        s = W_CHANNEL * 0.5;
    }
    if contains_word(cand_title, "official audio")
        || contains_word(cand_title, "audio oficial")
        || contains_word(cand_title, "audio")
    {
        s = s.max(W_CHANNEL * 0.6);
    }
    s.min(W_CHANNEL)
}

/// Resta por versiones que el original no es: live, cover, remix, 8D…
/// 40 por palabra (tope 60): una sola alcanza para que el match sea dudoso.
fn penalty(clean: &CleanTitle, cand_title_lower: &str, channel: &str) -> f32 {
    let mut p: f32 = 0.0;
    for w in VERSION_WORDS {
        // "version", "edit" y "mix" son demasiado genéricas para penalizar solas.
        if matches!(*w, "version" | "versión" | "edit" | "mix") {
            continue;
        }
        if contains_word(cand_title_lower, w) && !allowed_by(&clean.version, w) {
            p += 40.0;
        }
    }
    if channel.contains("karaoke") && !clean.version.contains(&"karaoke") {
        p += 40.0;
    }
    p.min(60.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clean::clean_title;

    fn track(title: &str, artist: &str, secs: u64) -> Track {
        Track {
            title: title.into(),
            artists: vec![artist.into()],
            album: None,
            duration_ms: secs * 1000,
            isrc: None,
            spotify_id: None,
        }
    }

    fn cand(title: &str, channel: &str, secs: f64) -> Candidate {
        Candidate {
            id: "x".into(),
            title: title.into(),
            channel: Some(channel.into()),
            duration_secs: Some(secs),
            views: None,
        }
    }

    fn s(t: &Track, c: &Candidate) -> u8 {
        score(t, &clean_title(&t.title), c).total()
    }

    #[test]
    fn topic_channel_wins() {
        let t = track("Under Pressure - Remastered 2011", "Queen", 248);
        let topic = cand("Under Pressure (Remastered 2011)", "Queen - Topic", 248.0);
        let live = cand(
            "Queen - Under Pressure (Live at Wembley 1986)",
            "Queen Official",
            249.0,
        );
        let lyrics = cand("Queen - Under Pressure (Lyrics)", "LyricLandia", 247.0);
        let long_video = cand(
            "Queen - Under Pressure (Official Video)",
            "Queen Official",
            290.0,
        );
        assert!(s(&t, &topic) >= 95, "{}", s(&t, &topic));
        assert!(s(&t, &live) < 60, "{}", s(&t, &live));
        assert!(s(&t, &lyrics) >= 60, "{}", s(&t, &lyrics));
        assert!(s(&t, &topic) > s(&t, &lyrics));
        assert!(s(&t, &lyrics) > s(&t, &long_video));
    }

    #[test]
    fn penalizes_bad_versions_but_not_when_original_has_them() {
        let t = track("Blinding Lights", "The Weeknd", 200);
        for bad in [
            "slowed + reverb",
            "sped up",
            "8D Audio",
            "Karaoke Version",
            "cover",
        ] {
            let c = cand(
                &format!("The Weeknd - Blinding Lights ({bad})"),
                "Random",
                200.0,
            );
            assert!(s(&t, &c) < 60, "{bad}: {}", s(&t, &c));
        }
        let t = track("Blinding Lights - Chromatics Remix", "The Weeknd", 200);
        let c = cand(
            "The Weeknd - Blinding Lights (Chromatics Remix)",
            "The Weeknd - Topic",
            200.0,
        );
        assert!(s(&t, &c) >= 90, "{}", s(&t, &c));
    }

    #[test]
    fn live_original_accepts_live_video() {
        let t = track("Labios Rotos - En Vivo Desde México / 2010", "Zoé", 243);
        let live = cand("Zoé - Labios Rotos (Live)", "Zoé", 245.0);
        let cover = cand("Labios rotos - Zoé // cover en français", "Fan", 238.0);
        assert!(s(&t, &live) >= 80, "{}", s(&t, &live));
        assert!(s(&t, &live) > s(&t, &cover));
    }

    #[test]
    fn duration_curve() {
        assert_eq!(duration_score(200_000, Some(202.0)), W_DURATION);
        assert!(duration_score(200_000, Some(210.0)) < W_DURATION);
        assert!(duration_score(200_000, Some(210.0)) > 0.0);
        assert_eq!(duration_score(200_000, Some(220.0)), 0.0);
        assert!(duration_score(200_000, Some(260.0)) < 0.0);
    }

    #[test]
    fn wrong_song_scores_low() {
        let t = track("Thriller", "Michael Jackson", 358);
        let c = cand(
            "Michael Jackson - Billie Jean (Official Video)",
            "Michael Jackson",
            294.0,
        );
        assert!(s(&t, &c) < 50, "{}", s(&t, &c));
    }

    #[test]
    fn spanish_titles_and_accents() {
        let t = track("De Música Ligera", "Soda Stereo", 212);
        let c = cand(
            "Soda Stereo - De Musica Ligera (Official Audio)",
            "Soda Stereo",
            213.0,
        );
        assert!(s(&t, &c) >= 90, "{}", s(&t, &c));
    }
}
