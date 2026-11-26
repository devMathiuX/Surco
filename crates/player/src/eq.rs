//! Ecualizador de 10 bandas.
//!
//! Filtros biquad (Audio EQ Cookbook de R. Bristow-Johnson): un estante de
//! graves en la primera banda, uno de agudos en la última y picos en el
//! medio. Se aplica a cada lote de muestras que lee la salida
//! ([`crate::decode::RingSource`]), así que cuesta lo mismo con o sin red.
//!
//! El ajuste vive en un global ([`set`]): cada fuente tiene su propio estado
//! de filtros y recalcula los coeficientes cuando cambia la versión. Las
//! bandas en 0 dB se saltean, y apagado (o todo en 0) no toca las muestras.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock};

/// Frecuencias centrales de las bandas, en Hz.
pub const BANDS: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// Ganancia máxima de cada banda, en dB (para arriba y para abajo).
pub const MAX_GAIN: f32 = 12.0;

/// Ancho de las bandas de pico: una octava.
const Q: f32 = 1.41;

/// Debajo de esto una banda se considera plana y no se calcula.
const FLAT: f32 = 0.05;

/// Ajuste del ecualizador.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Equalizer {
    pub enabled: bool,
    /// Ganancia de cada banda de [`BANDS`], en dB.
    pub gains: [f32; 10],
}

impl Default for Equalizer {
    fn default() -> Self {
        Self {
            enabled: false,
            gains: [0.0; 10],
        }
    }
}

impl Equalizer {
    /// Si cambia algo el sonido.
    fn active(&self) -> bool {
        self.enabled && self.gains.iter().any(|g| g.abs() >= FLAT)
    }

    /// Atenuación previa para que las bandas subidas no saturen: un poco
    /// menos que la banda más alta (bajar todo lo que se sube se oye flojo).
    fn preamp(&self) -> f32 {
        let max = self.gains.iter().copied().fold(0.0_f32, f32::max);
        db_to_gain(-max * 0.7)
    }
}

static SETTINGS: RwLock<Equalizer> = RwLock::new(Equalizer {
    enabled: false,
    gains: [0.0; 10],
});
static VERSION: AtomicU64 = AtomicU64::new(0);

/// Cambia el ecualizador. Se oye en unos 100 ms.
pub fn set(eq: Equalizer) {
    *SETTINGS.write().unwrap_or_else(PoisonError::into_inner) = eq;
    VERSION.fetch_add(1, Ordering::Release);
}

/// El ecualizador actual.
pub fn get() -> Equalizer {
    *SETTINGS.read().unwrap_or_else(PoisonError::into_inner)
}

fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Coeficientes de un biquad, ya normalizados (a0 = 1).
#[derive(Debug, Clone, Copy, Default)]
struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    LowShelf,
    Peak,
    HighShelf,
}

impl Coefs {
    fn new(shape: Shape, freq: f32, gain_db: f32, rate: f32) -> Self {
        let a = 10.0_f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Peak => {
                let alpha = sin / (2.0 * Q);
                (
                    1.0 + alpha * a,
                    -2.0 * cos,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos,
                    1.0 - alpha / a,
                )
            }
            Shape::LowShelf | Shape::HighShelf => {
                // Pendiente S = 1.
                let alpha = sin / 2.0 * 2.0_f32.sqrt();
                let k = 2.0 * a.sqrt() * alpha;
                if matches!(shape, Shape::LowShelf) {
                    (
                        a * ((a + 1.0) - (a - 1.0) * cos + k),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                        a * ((a + 1.0) - (a - 1.0) * cos - k),
                        (a + 1.0) + (a - 1.0) * cos + k,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                        (a + 1.0) + (a - 1.0) * cos - k,
                    )
                } else {
                    (
                        a * ((a + 1.0) + (a - 1.0) * cos + k),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                        a * ((a + 1.0) + (a - 1.0) * cos - k),
                        (a + 1.0) - (a - 1.0) * cos + k,
                        2.0 * ((a - 1.0) - (a + 1.0) * cos),
                        (a + 1.0) - (a - 1.0) * cos - k,
                    )
                }
            }
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Estado de un filtro en un canal (forma directa II transpuesta).
#[derive(Debug, Clone, Copy, Default)]
struct State {
    z1: f32,
    z2: f32,
}

/// Ecualizador de una fuente: coeficientes de las bandas activas y el
/// estado de cada una por canal.
#[derive(Debug, Default)]
pub(crate) struct Filters {
    version: u64,
    rate: u32,
    active: bool,
    preamp: f32,
    coefs: Vec<Coefs>,
    /// `coefs.len() * channels` estados, banda por banda.
    states: Vec<State>,
}

impl Filters {
    pub(crate) fn new() -> Self {
        Self {
            // Distinta de cualquier versión real: el primer lote configura.
            version: u64::MAX,
            ..Self::default()
        }
    }

    fn configure(&mut self, rate: u32, channels: usize) {
        let eq = get();
        self.rate = rate;
        self.active = eq.active() && rate > 0;
        self.preamp = eq.preamp();
        let old = self.coefs.len();
        self.coefs.clear();
        if self.active {
            let nyquist = rate as f32 / 2.0;
            for (i, (&freq, &gain)) in BANDS.iter().zip(&eq.gains).enumerate() {
                if gain.abs() < FLAT || freq >= nyquist * 0.95 {
                    continue;
                }
                let shape = match i {
                    0 => Shape::LowShelf,
                    9 => Shape::HighShelf,
                    _ => Shape::Peak,
                };
                self.coefs.push(Coefs::new(
                    shape,
                    freq,
                    gain.clamp(-MAX_GAIN, MAX_GAIN),
                    rate as f32,
                ));
            }
        }
        // Si cambió qué bandas se usan, el estado viejo no sirve.
        if old != self.coefs.len() || self.states.len() != self.coefs.len() * channels {
            self.states.clear();
            self.states
                .resize(self.coefs.len() * channels, State::default());
        }
    }

    /// Ecualiza `samples` (cuadros intercalados de `channels` canales).
    pub(crate) fn process(&mut self, samples: &mut [f32], channels: usize, rate: u32) {
        let version = VERSION.load(Ordering::Acquire);
        if version != self.version || rate != self.rate {
            self.version = version;
            self.configure(rate, channels);
        }
        if !self.active || channels == 0 {
            return;
        }
        let bands = self.coefs.len();
        for frame in samples.chunks_exact_mut(channels) {
            for (ch, sample) in frame.iter_mut().enumerate() {
                let mut x = *sample * self.preamp;
                let states = &mut self.states[ch * bands..(ch + 1) * bands];
                for (c, s) in self.coefs.iter().zip(states.iter_mut()) {
                    let y = c.b0 * x + s.z1;
                    s.z1 = c.b1 * x - c.a1 * y + s.z2;
                    s.z2 = c.b2 * x - c.a2 * y;
                    x = y;
                }
                *sample = x.clamp(-1.0, 1.0);
            }
        }
    }
}

/// Ajustes de fábrica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Flat,
    Bass,
    Treble,
    Vocal,
    Rock,
    Pop,
    Electronic,
    Acoustic,
}

impl Preset {
    pub const ALL: [Preset; 8] = [
        Preset::Flat,
        Preset::Bass,
        Preset::Treble,
        Preset::Vocal,
        Preset::Rock,
        Preset::Pop,
        Preset::Electronic,
        Preset::Acoustic,
    ];

    /// Ganancias de las 10 bandas, en dB.
    pub fn gains(self) -> [f32; 10] {
        match self {
            Preset::Flat => [0.0; 10],
            Preset::Bass => [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            Preset::Treble => [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0],
            Preset::Vocal => [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0],
            Preset::Rock => [5.0, 4.0, 2.0, -1.0, -2.0, -1.0, 2.0, 3.0, 4.0, 4.0],
            Preset::Pop => [-1.0, 1.0, 3.0, 4.0, 3.0, 0.0, -1.0, -1.0, 1.0, 2.0],
            Preset::Electronic => [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.0, 2.0, 4.0, 5.0],
            Preset::Acoustic => [3.0, 2.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 2.0, 1.0],
        }
    }

    /// El ajuste de fábrica con estas ganancias, si hay uno.
    pub fn matching(gains: &[f32; 10]) -> Option<Preset> {
        Self::ALL.into_iter().find(|p| {
            p.gains()
                .iter()
                .zip(gains)
                .all(|(a, b)| (a - b).abs() < FLAT)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amplitud de un seno de `freq` Hz después de pasar por `f`.
    fn response(f: &mut Filters, freq: f32) -> f32 {
        let rate = 48_000;
        let n = 48_000;
        let mut buf: Vec<f32> = (0..n)
            .map(|i| 0.25 * (2.0 * PI * freq * i as f32 / rate as f32).sin())
            .collect();
        f.process(&mut buf, 1, rate);
        // La segunda mitad, ya sin el transitorio.
        buf[n / 2..].iter().fold(0.0_f32, |m, v| m.max(v.abs())) / 0.25
    }

    // Un solo test toca el global, para que no se pisen en paralelo.
    #[test]
    fn bands_boost_cut_and_bypass() {
        set(Equalizer::default());
        let mut f = Filters::new();
        let mut buf = vec![0.3_f32, -0.2, 0.1];
        f.process(&mut buf, 1, 48_000);
        assert_eq!(buf, [0.3, -0.2, 0.1], "apagado no toca nada");

        let mut gains = [0.0; 10];
        gains[5] = 12.0; // 1 kHz
        set(Equalizer {
            enabled: true,
            gains,
        });
        let mut f = Filters::new();
        let at_1k = response(&mut f, 1000.0);
        let mut f = Filters::new();
        let at_100 = response(&mut f, 100.0);
        let pre = db_to_gain(-12.0 * 0.7);
        // +12 dB en la banda (×3,98) por la atenuación previa.
        assert!((at_1k / pre - 3.98).abs() < 0.3, "1 kHz: {at_1k}");
        assert!((at_100 / pre - 1.0).abs() < 0.1, "100 Hz: {at_100}");

        let mut gains = [0.0; 10];
        gains[0] = -12.0;
        set(Equalizer {
            enabled: true,
            gains,
        });
        let mut f = Filters::new();
        let low = response(&mut f, 30.0);
        assert!(low < 0.6, "graves bajados: {low}");
        set(Equalizer::default());
    }

    #[test]
    fn presets_are_found_back() {
        for p in Preset::ALL {
            assert_eq!(Preset::matching(&p.gains()), Some(p));
        }
        let mut odd = [0.0; 10];
        odd[3] = 7.0;
        assert_eq!(Preset::matching(&odd), None);
    }
}
