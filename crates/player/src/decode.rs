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
pub(crate) const PREBUFFER: Duration = Duration::from_secs(2);
/// Máximo de audio decodificado por adelantado.
const MAX_BUFFER: Duration = Duration::from_secs(5);
/// Muestras que el callback de audio toma por vez del buffer compartido.
const BATCH: usize = 4096;

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

        // Buffer lleno o archivo terminado: esperar (puede llegar un seek).
        let full = spec
            .map(|s| shared.buf().len() >= s.samples_for(MAX_BUFFER))
            .unwrap_or(false);
        if full || shared.finished.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
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

        let buffered = {
            let mut buf = shared.buf();
            buf.extend(sb.samples());
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

/// Fuente de rodio que lee del buffer compartido. Nunca bloquea.
pub(crate) struct RingSource {
    shared: Arc<Shared>,
    local: Vec<f32>,
    idx: usize,
    generation: u64,
    spec: AudioSpec,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl RingSource {
    pub(crate) fn new(shared: Arc<Shared>, spec: AudioSpec) -> Option<Self> {
        Some(Self {
            shared,
            local: Vec::with_capacity(BATCH),
            idx: 0,
            generation: 0,
            spec,
            channels: NonZero::new(spec.channels)?,
            rate: NonZero::new(spec.rate)?,
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
        self.idx = 0;
        self.shared.played.fetch_add(n as u64, Ordering::Relaxed);
        n > 0
    }
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.shared.stop.load(Ordering::Relaxed) {
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
