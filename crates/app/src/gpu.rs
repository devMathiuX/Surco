//! Dibujar con la placa de video (opcional).
//!
//! Surco trae los dos motores de dibujo de iced: tiny-skia (procesador) y
//! wgpu (placa de video). Por defecto usa el procesador, que gasta poca RAM.
//! Con la GPU el scroll y las animaciones van sincronizados con el monitor
//! (vsync) y el procesador casi no trabaja, a cambio de 50-100 MB más de RAM
//! por los drivers.
//!
//! La elección se guarda como un archivo vacío en la carpeta de datos (no en
//! la base): hay que leerla antes de abrir la ventana, y el motor no se puede
//! cambiar con la ventana abierta.

use std::path::PathBuf;

/// Archivo que, si existe, pide la GPU.
const MARKER: &str = "usar-gpu";

fn marker() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join(MARKER))
}

/// ¿Se pidió dibujar con la GPU?
pub fn enabled() -> bool {
    marker().is_some_and(|m| m.exists())
}

/// Guarda la elección (se aplica al volver a abrir Surco).
pub fn set(on: bool) {
    let Some(m) = marker() else {
        return;
    };
    let result = if on {
        m.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&m, b""))
    } else {
        match std::fs::remove_file(&m) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, "no se pudo guardar la elección de GPU");
    }
}

/// Elige el motor de dibujo. Va al principio de `main`, con un solo hilo
/// (cambiar variables de entorno con otros hilos andando no es seguro).
/// Si el usuario ya puso `ICED_BACKEND`, se respeta.
pub fn choose_backend() {
    if std::env::var_os("ICED_BACKEND").is_some() {
        return;
    }
    // Con GPU: si la placa no anda, iced sigue con el procesador.
    let backend = if enabled() {
        "wgpu,tiny-skia"
    } else {
        "tiny-skia"
    };
    std::env::set_var("ICED_BACKEND", backend);
}
