//! Limpieza de títulos de Spotify para armar la búsqueda en YouTube.

/// Título limpio y lo que se le sacó.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanTitle {
    /// Título sin sufijos de remaster, feat., "From …", etc.
    pub title: String,
    /// Solo el nombre de la canción, sin la parte de versión que sí se
    /// conserva en `title` (`"Labios Rotos - En Vivo…"` → `"Labios Rotos"`).
    pub base: String,
    /// Artistas invitados (`feat. X`, `with X`, `ft. X`).
    pub featured: Vec<String>,
    /// Palabras de versión que el original **sí** tiene (remix, live, acoustic…).
    /// Sirven para no penalizar a un candidato que también las tiene.
    pub version: Vec<&'static str>,
}

/// Palabras que marcan una versión distinta de la de estudio.
/// Se buscan en minúsculas, como palabra o frase completa.
pub const VERSION_WORDS: &[&str] = &[
    "live",
    "en vivo",
    "en directo",
    "directo",
    "en concierto",
    "acoustic",
    "acústico",
    "acustico",
    "unplugged",
    "remix",
    "mix",
    "cover",
    "karaoke",
    "instrumental",
    "8d",
    "slowed",
    "sped up",
    "speed up",
    "reverb",
    "nightcore",
    "432hz",
    "432 hz",
    "528hz",
    "bass boosted",
    "reversed",
    "demo",
    "edit",
    "version",
    "versión",
];

/// Palabras equivalentes: si el original dice "en vivo", un video que dice
/// "live" no se penaliza.
const SYNONYMS: &[&[&str]] = &[
    &["live", "en vivo", "en directo", "directo", "en concierto"],
    &["acoustic", "acústico", "acustico", "unplugged"],
    &["sped up", "speed up", "nightcore"],
    &["slowed", "reverb"],
];

/// `true` si `word` (o un sinónimo) está entre las palabras de versión del original.
pub fn allowed_by(version: &[&str], word: &str) -> bool {
    version.contains(&word)
        || SYNONYMS
            .iter()
            .any(|g| g.contains(&word) && g.iter().any(|w| version.contains(w)))
}

/// Prefijos (dentro de paréntesis o después de " - ") que se descartan.
const DROP_PREFIXES: &[&str] = &[
    "remaster",
    "remastered",
    "digital remaster",
    "single version",
    "album version",
    "radio edit",
    "mono",
    "stereo",
    "from ",
    "original mix",
    "bonus track",
    "explicit",
    "clean",
];

/// Limpia un título de Spotify.
///
/// - `"Under Pressure - Remastered 2011"` → `"Under Pressure"`
/// - `"Bad Guy (feat. Justin Bieber)"` → `"Bad Guy"`, featured `["Justin Bieber"]`
/// - `"Song (Live at Wembley)"` → `"Song (Live at Wembley)"` (se conserva) y version `["live"]`
pub fn clean_title(raw: &str) -> CleanTitle {
    let mut featured = Vec::new();
    let mut kept_parts: Vec<String> = Vec::new();

    // 1. Separar el título base de los grupos entre paréntesis/corchetes.
    let (base, groups) = split_groups(raw);
    let mut base = base.trim().to_owned();

    // 2. " - Remastered 2011", " - From \"Movie\"", " - Live", " - feat. X"…
    let mut kept_tail = None;
    if let Some((head, tail)) = base.clone().split_once(" - ") {
        let tail_l = tail.trim().to_lowercase();
        if let Some(names) = feat_names(&tail_l, tail.trim()) {
            featured.extend(names);
            base = head.trim().to_owned();
        } else if is_droppable(&tail_l) {
            base = head.trim().to_owned();
        } else {
            kept_tail = Some(head.trim().len());
        }
        // Si es otra cosa (" - Live", " - Acoustic"), queda en el título.
    }

    // 3. Grupos: feat. → aparte; remaster/From → afuera; el resto se conserva.
    for g in groups {
        let gl = g.trim().to_lowercase();
        if let Some(names) = feat_names(&gl, g.trim()) {
            featured.extend(names);
        } else if !is_droppable(&gl) {
            kept_parts.push(format!("({})", g.trim()));
        }
    }

    // 4. "feat." sin paréntesis en el título base.
    let lower = base.to_lowercase();
    for marker in [" feat. ", " ft. ", " featuring "] {
        if let Some(i) = lower.find(marker) {
            let names = &base[i + marker.len()..];
            featured.extend(split_names(names));
            base.truncate(i);
            break;
        }
    }

    let song_only = match kept_tail {
        Some(n) if n <= base.len() => base[..n].trim().to_owned(),
        _ => base.trim().to_owned(),
    };
    let mut title = base.trim().to_owned();
    for p in kept_parts {
        title.push(' ');
        title.push_str(&p);
    }
    let raw_l = raw.to_lowercase();
    let version = VERSION_WORDS
        .iter()
        .copied()
        .filter(|w| contains_word(&raw_l, w))
        // "Remastered 2011 Version" o "Single Version" no son versiones distintas.
        .filter(|w| {
            !(matches!(*w, "version" | "versión" | "edit" | "mix") && !kept_version(&title, w))
        })
        .collect();

    CleanTitle {
        title,
        base: song_only,
        featured,
        version,
    }
}

/// `true` si la palabra sobrevive en el título limpio (no venía de un remaster/radio edit).
fn kept_version(title: &str, word: &str) -> bool {
    contains_word(&title.to_lowercase(), word)
}

fn is_droppable(lower: &str) -> bool {
    let l = lower.trim();
    DROP_PREFIXES.iter().any(|p| l.starts_with(p))
        || l.ends_with("remaster")
        || l.ends_with("remastered")
        || l.contains("remastered")
        || (l.contains("remaster") && l.chars().any(|c| c.is_ascii_digit()))
        || l.starts_with("del ")
        || l.contains("original score")
        || l.contains("soundtrack")
}

/// Si el grupo es un "feat. X & Y", devuelve los nombres (con mayúsculas originales).
fn feat_names(lower: &str, original: &str) -> Option<Vec<String>> {
    for prefix in [
        "feat.",
        "feat ",
        "ft.",
        "ft ",
        "featuring ",
        "with ",
        "con ",
    ] {
        if lower.starts_with(prefix) {
            return Some(split_names(&original[prefix.len()..]));
        }
    }
    None
}

fn split_names(s: &str) -> Vec<String> {
    s.split([',', '&'])
        .flat_map(|p| p.split(" and "))
        .flat_map(|p| p.split(" y "))
        .map(|p| p.trim().trim_end_matches([')', ']']).trim())
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Separa `"Título (a) [b]"` en `("Título", ["a", "b"])`. Respeta anidamiento simple.
fn split_groups(s: &str) -> (String, Vec<String>) {
    let mut base = String::new();
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' => {
                if depth > 0 {
                    cur.push(c);
                }
                depth += 1;
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    groups.push(std::mem::take(&mut cur));
                } else {
                    cur.push(c);
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => base.push(c),
        }
    }
    if !cur.is_empty() {
        base.push_str(&cur);
    }
    (
        base.split_whitespace().collect::<Vec<_>>().join(" "),
        groups,
    )
}

/// `true` si `word` aparece en `haystack` como palabra (o frase) completa.
pub fn contains_word(haystack: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(i) = haystack[start..].find(word) {
        let a = start + i;
        let b = a + word.len();
        let before = haystack[..a].chars().next_back();
        let after = haystack[b..].chars().next();
        let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if boundary(before) && boundary(after) {
            return true;
        }
        start = a + word.len().max(1);
        while !haystack.is_char_boundary(start) {
            start += 1;
        }
    }
    false
}

/// Normaliza para comparar: minúsculas, sin acentos comunes, solo letras,
/// dígitos y espacios simples.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            '&' => ' ',
            c => c,
        };
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(raw: &str) -> CleanTitle {
        clean_title(raw)
    }

    #[test]
    fn drops_remaster_suffixes() {
        assert_eq!(
            t("Under Pressure - Remastered 2011").title,
            "Under Pressure"
        );
        assert_eq!(t("Heroes - 2017 Remaster").title, "Heroes");
        assert_eq!(
            t("Eyes Without A Face - Remastered 1999").title,
            "Eyes Without A Face"
        );
        assert_eq!(t("Song (Remastered 2009)").title, "Song");
        assert_eq!(t("Song - Single Version").title, "Song");
        assert_eq!(t("Song - Radio Edit").title, "Song");
        assert_eq!(
            t("Song - From \"Top Gun\" Original Soundtrack").title,
            "Song"
        );
        assert_eq!(
            t("Flower In A Hurricane (feat. Tame Impala, SZA) [From GTAVI: Original Score]").title,
            "Flower In A Hurricane"
        );
        assert!(t("Under Pressure - Remastered 2011").version.is_empty());
        assert!(t("Song - Single Version").version.is_empty());
    }

    #[test]
    fn extracts_featured_artists() {
        let c = t("Flower In A Hurricane (feat. Tame Impala, SZA & King Gizzard)");
        assert_eq!(c.featured, ["Tame Impala", "SZA", "King Gizzard"]);
        assert_eq!(
            t("Señorita (with Camila Cabello)").featured,
            ["Camila Cabello"]
        );
        let c = t("Rich Flex ft. 21 Savage");
        assert_eq!(c.title, "Rich Flex");
        assert_eq!(c.featured, ["21 Savage"]);
        assert_eq!(t("Song - feat. X").featured, ["X"]);
    }

    #[test]
    fn keeps_real_versions() {
        let c = t("Wonderwall (Live at Knebworth)");
        assert_eq!(c.title, "Wonderwall (Live at Knebworth)");
        assert_eq!(c.version, ["live"]);
        let c = t("Ella - Acústico");
        assert_eq!(c.title, "Ella - Acústico");
        assert!(c.version.contains(&"acústico"));
        assert!(t("Blinding Lights - Remix").version.contains(&"remix"));
        assert!(t("Liver").version.is_empty());
    }

    #[test]
    fn base_title_and_synonyms() {
        let c = t("Labios Rotos - En Vivo Desde México / 2010");
        assert_eq!(c.base, "Labios Rotos");
        assert!(allowed_by(&c.version, "live"));
        assert!(!allowed_by(&c.version, "acoustic"));
        let c = t("Wonderwall (Live at Knebworth)");
        assert_eq!(c.base, "Wonderwall");
        assert!(allowed_by(&c.version, "en vivo"));
        assert_eq!(t("Under Pressure - Remastered 2011").base, "Under Pressure");
    }

    #[test]
    fn word_boundaries_and_normalize() {
        assert!(contains_word("song (live)", "live"));
        assert!(!contains_word("oliver", "live"));
        assert!(contains_word("x - sped up", "sped up"));
        assert_eq!(normalize("De Música Ligera!"), "de musica ligera");
        assert_eq!(normalize("Rock & Roll"), "rock roll");
    }
}
