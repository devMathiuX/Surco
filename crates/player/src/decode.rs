//! Decodificación con symphonia en un hilo propio.
//!
//! El hilo decodificador llena un buffer de muestras (`VecDeque<f32>`
//! entrelazadas). [`RingSource`] lo vacía desde el callback de audio de rodio
//! sin bloquear nunca: si el buffer está vacío devuelve silencio. Así una red
//! lenta produce un corte, no un cuelgue del audio.

use std::collections::VecDeque;
use std::io;
use std::num::NonZero;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Audio acumulado antes de empezar a sonar.
pub(crate) const PREBUFFER: Duration = Duration::from_secs(1);
/// Máximo de audio decodificado por adelantado.
const MAX_BUFFER: Duration = Duration::from_secs(5);
/// Muestras que el callback de audio toma por vez del buffer compartido.
const BATCH: usize = 4096;
/// Debajo de esto (≈ -50 dBFS) una muestra cuenta como silencio.
const SILENCE: f32 = 0.003;
/// Silencio inicial máximo que se saltea (más largo es parte de la canción).
const MAX_LEADING_SILENCE: Duration = Duration::from_secs(10);
/// Silencio retenido como máximo antes de dejar de decodificar.
const MAX_HELD_SILENCE: Duration = Duration::from_secs(20);
/// Con menos audio que esto en el buffer se suelta el silencio retenido.
const LOW_WATER: Duration = Duration::from_secs(1);

/// Estado compartido entre el hilo decodificador, el callback de audio y el motor.
#[derive(Default)]
pub(crate) struct Shared {
    buf: Mutex<VecDeque<f32>>,
    /// Cambia en cada seek (con `buf` tomado), para que el callback descarte su lote local.
    generation: AtomicU64,
    stop: AtomicBool,
    seek: Mutex<Option<Duration>>,
    /// El decodificador llegó al final del archivo.
    finished: AtomicBool,
    /// Muestras entregadas al callback (incluye el offset del último seek).
    played: AtomicU64,
}

impl Shared {
    fn buf(&self) -> MutexGuard<'_, VecDeque<f32>> {
        // Si otro hilo hizo panic con el lock tomado, seguimos con el dato igual:
        // en el peor caso suena un corte.
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn request_seek(&self, to: Duration) {
        *self.seek.lock().unwrap_or_else(|e| e.into_inner()) = Some(to);
    }

    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub(crate) fn played_samples(&self) -> u64 {
        self.played.load(Ordering::Relaxed)
    }

    /// El decodificador llegó al final (puede quedar audio en el buffer).
    pub(crate) fn finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// `true` cuando ya se decodificó y se reprodujo todo.
    pub(crate) fn drained(&self) -> bool {
        self.finished.load(Ordering::SeqCst) && self.buf().is_empty()
    }
}

/// Formato de la salida decodificada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AudioSpec {
    pub channels: u16,
    pub rate: u32,
}

impl AudioSpec {
    fn samples_for(self, d: Duration) -> usize {
        (d.as_secs_f64() * f64::from(self.rate) * f64::from(self.channels)) as usize
    }

    pub(crate) fn duration_of(self, samples: u64) -> Duration {
        let per_sec = u64::from(self.rate) * u64::from(self.channels);
        if per_sec == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(samples as f64 / per_sec as f64)
    }
}

/// Eventos que el hilo decodificador le manda al motor.
#[derive(Debug)]
pub(crate) enum DecodeEvent {
    /// Hay suficiente audio para arrancar.
    Ready {
        spec: AudioSpec,
        duration: Option<Duration>,
    },
    Failed(DecodeFailure),
}

#[derive(Debug, Clone)]
pub(crate) enum DecodeFailure {
    /// La URL del stream venció (403): hay que re-resolverla.
    Expired,
    Other(String),
}

impl std::fmt::Display for DecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => f.write_str("la URL del stream venció"),
            Self::Other(m) => f.write_str(m),
        }
    }
}

fn failure(e: SymError) -> DecodeFailure {
    match e {
        SymError::IoError(io) if io.kind() == io::ErrorKind::PermissionDenied => {
            DecodeFailure::Expired
        }
        SymError::Unsupported(what) => {
            DecodeFailure::Other(format!("formato no soportado: {what}"))
        }
        other => DecodeFailure::Other(other.to_string()),
    }
}

/// Lanza el hilo decodificador. `on_event` se llama desde ese hilo.
pub(crate) fn spawn<F>(
    source: Box<dyn MediaSource>,
    ext_hint: Option<String>,
    start_at: Option<Duration>,
    on_event: F,
) -> io::Result<Arc<Shared>>
where
    F: Fn(DecodeEvent) + Send + 'static,
{
    let shared = Arc::new(Shared::default());
    let thread_shared = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("surco-decode".into())
        .spawn(move || {
            if let Err(e) = run(source, ext_hint, start_at, &thread_shared, &on_event) {
                thread_shared.finished.store(true, Ordering::SeqCst);
                on_event(DecodeEvent::Failed(e));
            }
        })?;
    Ok(shared)
}

fn run(
    source: Box<dyn MediaSource>,
    ext_hint: Option<String>,
    start_at: Option<Duration>,
    shared: &Shared,
    on_event: &dyn Fn(DecodeEvent),
) -> Result<(), DecodeFailure> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = &ext_hint {
        hint.with_extension(ext);
    }
    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .map_err(failure)?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| DecodeFailure::Other("el archivo no tiene pistas de audio".into()))?;
    let track_id = track.id;
    let duration = match (track.codec_params.time_base, track.codec_params.n_frames) {
        (Some(tb), Some(n)) if n > 0 => {
            let t = tb.calc_time(n);
            Some(Duration::from_secs(t.seconds) + Duration::from_secs_f64(t.frac))
        }
        _ => None,
    };
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(failure)?;

    if let Some(at) = start_at {
        shared.request_seek(at);
    }

    let mut spec: Option<AudioSpec> = None;
    let mut ready_sent = false;
    // Sin cortes: se saltea el silencio del principio y del final del video.
    let mut trim = Trimmer::new(start_at.is_none());
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    // Tras un seek preciso, timestamp desde el que se vuelve a emitir audio.
    let mut skip_until_ts: u64 = 0;

    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return Ok(());
        }

        let seek_to = shared.seek.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(to) = seek_to {
            let time = Time::new(to.as_secs(), f64::from(to.subsec_nanos()) / 1e9);
            match format.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(track_id),
                },
            ) {
                Ok(seeked) => {
                    decoder.reset();
                    trim.reset();
                    skip_until_ts = seeked.required_ts;
                    shared.finished.store(false, Ordering::SeqCst);
                    let mut buf = shared.buf();
                    buf.clear();
                    shared.generation.fetch_add(1, Ordering::SeqCst);
                    let pos = spec.map(|s| s.samples_for(to) as u64).unwrap_or(0);
                    shared.played.store(pos, Ordering::SeqCst);
                }
                Err(e) => tracing::warn!(error = %e, "falló el seek"),
            }
        }

        // Silencio retenido: se suelta cuando hace falta para no cortar.
        // Antes de arrancar se suelta hasta el pre-buffer: si no, una canción
        // con una intro muy callada se quedaba esperando para siempre (buffer
        // "lleno" de silencio retenido, pero sin llegar a avisar que está lista).
        if let Some(s) = spec {
            let mut buf = shared.buf();
            let low = s.samples_for(if ready_sent { LOW_WATER } else { PREBUFFER });
            if trim.pending > 0 && buf.len() < low {
                let n = trim.release(low - buf.len(), usize::from(s.channels));
                buf.extend(std::iter::repeat_n(0.0, n));
            }
            let buffered = buf.len();
            drop(buf);
            if !ready_sent && buffered >= s.samples_for(PREBUFFER) {
                ready_sent = true;
                on_event(DecodeEvent::Ready { spec: s, duration });
            }
        }

        // Buffer lleno o archivo terminado: esperar (puede llegar un seek).
        let full = spec
            .map(|s| {
                shared.buf().len() >= s.samples_for(MAX_BUFFER)
                    || trim.pending >= s.samples_for(MAX_HELD_SILENCE)
            })
            .unwrap_or(false);
        if full || shared.finished.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // El silencio retenido era el final del video: se descarta.
                trim.pending = 0;
                shared.finished.store(true, Ordering::SeqCst);
                if !ready_sent {
                    if let Some(spec) = spec {
                        ready_sent = true;
                        on_event(DecodeEvent::Ready { spec, duration });
                    } else {
                        return Err(DecodeFailure::Other("el archivo no tiene audio".into()));
                    }
                }
                continue;
            }
            Err(SymError::ResetRequired) => {
                shared.finished.store(true, Ordering::SeqCst);
                continue;
            }
            Err(e) => return Err(failure(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        if packet.ts() + packet.dur() <= skip_until_ts {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(e)) => {
                tracing::debug!(error = e, "paquete corrupto, se descarta");
                continue;
            }
            Err(e) => return Err(failure(e)),
        };

        let s = *decoded.spec();
        let this_spec = AudioSpec {
            channels: s.channels.count() as u16,
            rate: s.rate,
        };
        if spec.is_none() {
            // Si hubo un seek antes del primer paquete, corregimos la posición.
            if let Some(at) = start_at {
                shared
                    .played
                    .store(this_spec.samples_for(at) as u64, Ordering::SeqCst);
            }
            spec = Some(this_spec);
        }
        let needed = decoded.capacity() as u64;
        let sb = match &mut sample_buf {
            Some(sb) if sb.capacity() as u64 >= needed => sb,
            slot => slot.insert(SampleBuffer::new(needed, s)),
        };
        sb.copy_interleaved_ref(decoded);

        let ch = usize::from(this_spec.channels);
        let was_leading = trim.leading;
        let (zeros, keep) = trim.feed(sb.samples(), ch, this_spec.samples_for(MAX_LEADING_SILENCE));
        if was_leading && !trim.leading && trim.dropped > 0 {
            // La posición sigue siendo la del video.
            shared
                .played
                .fetch_add(trim.dropped as u64, Ordering::SeqCst);
            tracing::debug!(
                secs = this_spec.duration_of(trim.dropped as u64).as_secs_f32(),
                "silencio inicial salteado"
            );
        }
        let buffered = {
            let mut buf = shared.buf();
            buf.extend(std::iter::repeat_n(0.0, zeros));
            buf.extend(&sb.samples()[keep]);
            buf.len()
        };

        if !ready_sent && buffered >= this_spec.samples_for(PREBUFFER) {
            ready_sent = true;
            on_event(DecodeEvent::Ready {
                spec: this_spec,
                duration,
            });
        }
    }
}

/// Saca el silencio del principio y del final de una canción.
///
/// Los videos de YouTube suelen tener segundos mudos al empezar y al terminar,
/// que suenan como un corte entre canciones. El del principio se descarta
/// (hasta un tope). El silencio que aparece después se retiene como un
/// contador: si vuelve a sonar algo se reinsertan los ceros, y si el archivo
/// termina antes, era el final y no se reproduce.
#[derive(Debug)]
struct Trimmer {
    /// Todavía no sonó nada.
    leading: bool,
    /// Muestras salteadas al principio.
    dropped: usize,
    /// Muestras de silencio retenidas (no encoladas todavía).
    pending: usize,
}

impl Trimmer {
    fn new(leading: bool) -> Self {
        Self {
            leading,
            dropped: 0,
            pending: 0,
        }
    }

    /// Después de un seek no hay silencio "inicial" ni retenido.
    fn reset(&mut self) {
        self.leading = false;
        self.pending = 0;
    }

    /// Procesa un bloque entrelazado. Devuelve cuántos ceros encolar antes y
    /// qué parte del bloque encolar después.
    fn feed(
        &mut self,
        samples: &[f32],
        ch: usize,
        max_leading: usize,
    ) -> (usize, std::ops::Range<usize>) {
        let ch = ch.max(1);
        let loud = |f: &[f32]| f.iter().any(|s| s.abs() >= SILENCE);
        let frames = samples.len() / ch;
        let first = (0..frames).find(|&i| loud(&samples[i * ch..(i + 1) * ch]));
        let Some(first) = first else {
            // Bloque mudo.
            if self.leading && self.dropped + samples.len() <= max_leading {
                self.dropped += samples.len();
            } else {
                self.leading = false;
                self.pending += samples.len();
            }
            return (0, 0..0);
        };
        let mut start = 0;
        if self.leading {
            self.leading = false;
            if self.dropped + first * ch <= max_leading {
                start = first * ch;
                self.dropped += start;
            }
        }
        let last = (first..frames)
            .rev()
            .find(|&i| loud(&samples[i * ch..(i + 1) * ch]))
            .unwrap_or(first);
        let end = (last + 1) * ch;
        let zeros = std::mem::replace(&mut self.pending, samples.len() - end);
        (zeros, start..end)
    }

    /// Suelta hasta `want` muestras de silencio retenido (cuadros completos).
    fn release(&mut self, want: usize, ch: usize) -> usize {
        let ch = ch.max(1);
        let n = (want.min(self.pending) / ch) * ch;
        self.pending -= n;
        n
    }
}

/// Fuente de rodio que lee del buffer compartido. Nunca bloquea.
pub(crate) struct RingSource {
    shared: Arc<Shared>,
    /// Saca esta fuente de la salida sin parar el decodificador.
    cancel: Arc<AtomicBool>,
    local: Vec<f32>,
    idx: usize,
    generation: u64,
    spec: AudioSpec,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    eq: crate::eq::Filters,
}

impl RingSource {
    pub(crate) fn new(shared: Arc<Shared>, spec: AudioSpec) -> Option<Self> {
        Some(Self {
            shared,
            cancel: Arc::default(),
            local: Vec::with_capacity(BATCH),
            idx: 0,
            generation: 0,
            spec,
            channels: NonZero::new(spec.channels)?,
            rate: NonZero::new(spec.rate)?,
            eq: crate::eq::Filters::new(),
        })
    }

    fn refill(&mut self) -> bool {
        let mut buf = self.shared.buf();
        self.generation = self.shared.generation.load(Ordering::SeqCst);
        // Lotes de cuadros completos, para no desalinear los canales.
        let ch = usize::from(self.spec.channels);
        let n = (buf.len().min(BATCH) / ch) * ch;
        self.local.clear();
        self.local.extend(buf.drain(..n));
        drop(buf);
        self.eq.process(&mut self.local, ch, self.spec.rate);
        self.idx = 0;
        self.shared.played.fetch_add(n as u64, Ordering::Relaxed);
        n > 0
    }
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.shared.stop.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
            return None;
        }
        // Un seek invalida lo que quedaba en el lote local.
        let need_refill = self.idx >= self.local.len() || self.shared_generation_changed();
        if need_refill && !self.refill() {
            if self.shared.finished.load(Ordering::SeqCst) {
                return None;
            }
            // Buffer vacío (red lenta): silencio en vez de bloquear.
            return Some(0.0);
        }
        let s = self.local[self.idx];
        self.idx += 1;
        Some(s)
    }
}

impl RingSource {
    /// Bandera que, al prenderse, termina esta fuente (la salida pasa a la
    /// siguiente) sin tocar el buffer ni el decodificador.
    pub(crate) fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    fn shared_generation_changed(&self) -> bool {
        self.shared.generation.load(Ordering::Relaxed) != self.generation
    }
}

impl rodio::Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(frames: &[f32]) -> Vec<f32> {
        // Estéreo: el mismo valor en los dos canales.
        frames.iter().flat_map(|&v| [v, v]).collect()
    }

    #[test]
    fn trims_leading_silence() {
        let mut t = Trimmer::new(true);
        let (z, r) = t.feed(&block(&[0.0, 0.0, 0.0]), 2, 100);
        assert_eq!((z, r), (0, 0..0));
        let (z, r) = t.feed(&block(&[0.001, 0.5, 0.0, 0.4]), 2, 100);
        assert_eq!(z, 0);
        assert_eq!(r, 2..8);
        assert_eq!(t.dropped, 8);
        assert!(!t.leading);
    }

    #[test]
    fn leading_cap_keeps_long_intros() {
        let mut t = Trimmer::new(true);
        t.feed(&block(&[0.0; 4]), 2, 10);
        // Pasa el tope: lo que sigue ya no se saltea.
        let (z, r) = t.feed(&block(&[0.0; 4]), 2, 10);
        assert_eq!((z, r), (0, 0..0));
        assert!(!t.leading);
        assert_eq!(t.pending, 8);
    }

    #[test]
    fn holds_trailing_silence_and_restores_it_if_music_returns() {
        let mut t = Trimmer::new(false);
        let (z, r) = t.feed(&block(&[0.5, 0.0, 0.0]), 2, 0);
        assert_eq!((z, r), (0, 0..2));
        assert_eq!(t.pending, 4);
        let (z, _) = t.feed(&block(&[0.0, 0.0]), 2, 0);
        assert_eq!(z, 0);
        assert_eq!(t.pending, 8);
        // Vuelve la música: los ceros retenidos van antes.
        let (z, r) = t.feed(&block(&[0.3]), 2, 0);
        assert_eq!((z, r), (8, 0..2));
        assert_eq!(t.pending, 0);
    }

    #[test]
    fn release_gives_whole_frames() {
        let mut t = Trimmer::new(false);
        t.pending = 9;
        assert_eq!(t.release(5, 2), 4);
        assert_eq!(t.pending, 5);
        assert_eq!(t.release(100, 2), 4);
    }
}
