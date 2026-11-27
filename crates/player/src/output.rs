//! Apertura de la salida de audio del sistema, con varios intentos.
//!
//! `rodio::DeviceSinkBuilder::open_default_sink` pide siempre un buffer fijo
//! de ~50 ms sin fijarse si el dispositivo lo acepta. En macOS hay salidas
//! (auriculares Bluetooth, monitores, placas externas, dispositivos
//! virtuales) que lo rechazan, y el plan B de rodio (probar otras
//! configuraciones y otros dispositivos) nunca corre ahí: filtra los
//! dispositivos por el nombre del driver, que CoreAudio no informa. Por eso
//! abrimos la salida nosotros, probando de lo más común a lo más raro.

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::cpal::{self, BufferSize, SupportedBufferSize, SupportedStreamConfig};
use rodio::{DeviceSinkBuilder, MixerDeviceSink};

/// Latencia buscada para el buffer del dispositivo (la misma que usa rodio).
const TARGET_MS: u32 = 50;

/// Cuántas configuraciones alternativas se prueban por dispositivo.
const MAX_ALTERNATIVES: usize = 12;

/// Abre la salida predeterminada; si no se puede, cualquier otra que ande.
///
/// `on_error` recibe los errores de la salida mientras suena (por ejemplo,
/// si se desconectan los auriculares).
pub(crate) fn open<F>(on_error: F) -> Result<MixerDeviceSink, String>
where
    F: FnMut(cpal::Error) + Clone + Send + 'static,
{
    let host = cpal::default_host();
    let default = host.default_output_device();
    let default_id = default.as_ref().and_then(|d| d.id().ok());
    let first_error = match &default {
        Some(device) => match open_device(device, &on_error) {
            Ok(sink) => return Ok(sink),
            Err(e) => e,
        },
        None => "no hay ningún dispositivo de salida de audio".to_owned(),
    };
    tracing::warn!(
        device = default.as_ref().map(device_name).unwrap_or_default(),
        error = %first_error,
        "no se pudo abrir la salida predeterminada, probando otras"
    );

    let devices = match host.output_devices() {
        Ok(devices) => devices,
        Err(e) => {
            tracing::warn!(error = %chain(&e), "no se pudo listar las salidas de audio");
            return Err(first_error);
        }
    };
    for device in devices {
        if default_id.is_some() && device.id().ok() == default_id {
            continue;
        }
        match open_device(&device, &on_error) {
            Ok(sink) => {
                tracing::warn!(device = device_name(&device), "usando otra salida de audio");
                return Ok(sink);
            }
            Err(e) => {
                tracing::debug!(device = device_name(&device), error = %e, "salida descartada")
            }
        }
    }
    Err(first_error)
}

/// Prueba un dispositivo: primero su configuración predeterminada (con un
/// buffer que el dispositivo acepte y, si no, con el que elija el sistema) y
/// después las demás configuraciones que dice soportar.
fn open_device<F>(device: &cpal::Device, on_error: &F) -> Result<MixerDeviceSink, String>
where
    F: FnMut(cpal::Error) + Clone + Send + 'static,
{
    let mut first_error = None;
    let mut remember = |e: String| {
        first_error.get_or_insert(e);
    };

    match device.default_output_config() {
        Ok(config) => {
            for buffer in [fitting_buffer(&config), BufferSize::Default] {
                match try_open(device, &config, buffer, on_error) {
                    Ok(sink) => {
                        log_opened(device, &config, buffer);
                        return Ok(sink);
                    }
                    Err(e) => {
                        tracing::debug!(?buffer, error = %e, "configuración rechazada");
                        remember(e);
                    }
                }
            }
        }
        Err(e) => remember(format!(
            "no se pudo leer la configuración del dispositivo: {}",
            chain(&e)
        )),
    }

    match rodio::stream::supported_output_configs(device) {
        Ok(configs) => {
            for config in configs.take(MAX_ALTERNATIVES) {
                for buffer in [BufferSize::Default, fitting_buffer(&config)] {
                    if let Ok(sink) = try_open(device, &config, buffer, on_error) {
                        log_opened(device, &config, buffer);
                        return Ok(sink);
                    }
                }
            }
        }
        Err(e) => remember(chain(&e)),
    }
    Err(first_error.unwrap_or_else(|| "el dispositivo no ofrece ninguna configuración".into()))
}

fn try_open<F>(
    device: &cpal::Device,
    config: &SupportedStreamConfig,
    buffer: BufferSize,
    on_error: &F,
) -> Result<MixerDeviceSink, String>
where
    F: FnMut(cpal::Error) + Clone + Send + 'static,
{
    DeviceSinkBuilder::default()
        .with_device(device.clone())
        .with_supported_config(config)
        .with_buffer_size(buffer)
        .with_error_callback(on_error.clone())
        .open_stream()
        .map_err(|e| chain(&e))
}

/// ~50 ms de buffer, ajustado al rango que acepta el dispositivo.
fn fitting_buffer(config: &SupportedStreamConfig) -> BufferSize {
    let frames = nearest_power_of_two(config.sample_rate() * TARGET_MS / 1000);
    match *config.buffer_size() {
        SupportedBufferSize::Range { min, max } if min <= max && max > 0 => {
            BufferSize::Fixed(frames.clamp(min.max(1), max))
        }
        _ => BufferSize::Fixed(frames),
    }
}

fn nearest_power_of_two(n: u32) -> u32 {
    if n <= 1 {
        return 1;
    }
    let next = n.next_power_of_two();
    let prev = next >> 1;
    if n - prev <= next - n {
        prev
    } else {
        next
    }
}

fn log_opened(device: &cpal::Device, config: &SupportedStreamConfig, buffer: BufferSize) {
    tracing::info!(
        device = device_name(device),
        rate = config.sample_rate(),
        channels = config.channels(),
        format = %config.sample_format(),
        ?buffer,
        "salida de audio abierta"
    );
}

fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "?".into())
}

/// El error con todas sus causas: rodio solo dice "Error opening the stream
/// with the OS" y el motivo real queda en `source()`.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let line = cause.to_string();
        if !text.ends_with(&line) {
            text.push_str(": ");
            text.push_str(&line);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_redondeado_a_potencia_de_dos() {
        assert_eq!(nearest_power_of_two(2400), 2048);
        assert_eq!(nearest_power_of_two(2205), 2048);
        assert_eq!(nearest_power_of_two(4800), 4096);
        assert_eq!(nearest_power_of_two(800), 1024);
        assert_eq!(nearest_power_of_two(1), 1);
    }

    #[test]
    fn buffer_dentro_del_rango_del_dispositivo() {
        let config = |min, max| {
            SupportedStreamConfig::new(
                2,
                48_000,
                SupportedBufferSize::Range { min, max },
                cpal::SampleFormat::F32,
            )
        };
        // Lo de siempre: entra en el rango.
        assert_eq!(fitting_buffer(&config(15, 4096)), BufferSize::Fixed(2048));
        // Auriculares Bluetooth con tope bajo: se achica.
        assert_eq!(fitting_buffer(&config(32, 1024)), BufferSize::Fixed(1024));
        // Mínimo alto: se agranda.
        assert_eq!(fitting_buffer(&config(4096, 8192)), BufferSize::Fixed(4096));
        let unknown = SupportedStreamConfig::new(
            2,
            44_100,
            SupportedBufferSize::Unknown,
            cpal::SampleFormat::F32,
        );
        assert_eq!(fitting_buffer(&unknown), BufferSize::Fixed(2048));
    }

    #[test]
    fn cadena_de_errores() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("Error opening the stream with the OS")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::other(
            "Buffer size 2048 is not in the supported range 32..=1024",
        ));
        assert_eq!(
            chain(&e),
            "Error opening the stream with the OS: Buffer size 2048 is not in the supported range 32..=1024"
        );
    }
}
