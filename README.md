# Surco

Reproductor de música de escritorio, ultraliviano, escrito en Rust. Importa playlists de Spotify y reproduce cada canción usando YouTube como fuente de audio (vía `yt-dlp`), sin cuenta Premium. Corre en Linux y Windows (x86_64) con la misma base de código.

> **Estado:** Fase 1 de 7 (esqueleto). La app abre una ventana vacía con el layout base; todavía no reproduce nada.

## Requisitos

- Rust estable ≥ 1.88 (`rustup`).
- **Linux:** un servidor X11 o Wayland con `libxkbcommon` (viene con cualquier escritorio). No hace falta nada más para compilar.
- **Windows:** toolchain MSVC.
- Desde la Fase 2: [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) (la app ofrecerá descargarlo). `ffmpeg` opcional.

## Cómo correrlo

```sh
cargo run                 # debug
cargo run --release       # release (opt-level 3)
cargo build --profile release-z   # variante optimizada para tamaño
```

## Estructura

```
crates/
├─ core/      surco-core      modelos (Track, Playlist, QueueItem, PlayerState…)
├─ spotify/   surco-spotify   importación de playlists (Fase 3)
├─ resolver/  surco-resolver  Spotify -> YouTube, búsqueda + scoring (Fase 4)
├─ provider/  surco-provider  wrapper de yt-dlp, URLs de stream (Fase 2)
├─ player/    surco-player    motor de audio, cola, estados (Fases 2 y 5)
├─ storage/   surco-storage   SQLite: caché, biblioteca, historial (Fase 6)
└─ app/       surco           GUI (iced) + binario
```

Los paquetes llevan el prefijo `surco-` porque `core` choca con el crate `core` de la librería estándar.

## Decisiones técnicas

### GUI: iced 0.14

Elegimos `iced` sobre `egui/eframe` porque:

- Es retenido y basado en mensajes (arquitectura Elm), que encaja con la comunicación GUI ↔ motor por canales: la GUI solo recibe mensajes y nunca bloquea.
- No redibuja cada frame: con la ventana quieta no consume CPU, a diferencia del modo inmediato de egui.
- Compila en Windows sin dependencias nativas extra (sin OpenSSL, sin webview), así que el empaquetado no se complica.

### Render: CPU (tiny-skia) por defecto, GPU opcional

Medido en Linux/X11 (Xvfb), ventana vacía, 4 s después de abrir:

| Backend | Binario (opt 3) | Binario (opt z) | RAM (RSS) |
|---|---|---|---|
| wgpu + tiny-skia (default de iced) | 10,5 MB | 7,6 MB | ~115 MB |
| **solo tiny-skia (elegido)** | **6,4 MB** | **4,9 MB** | **~14 MB** |

Con wgpu, la mayor parte de la RAM son los drivers gráficos mapeados en memoria (en la medición, llvmpipe/Vulkan por software; con GPU real también son decenas de MB). Eso solo ya rompe el límite de 80 MB, así que el render por defecto es por CPU. Para una UI plana como esta (listas, texto, rectángulos) alcanza de sobra.

Quien quiera GPU puede compilar con `cargo build --release --features gpu`.

### Perfil release

`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`. Medimos `opt-level = 3` y `"z"` (perfil `release-z`):

- `z` ahorra ~1,5 MB (4,9 vs 6,4 MB) sin diferencia de RAM.
- Ambos quedan muy por debajo de 15 MB.

Por ahora el perfil `release` usa `opt-level = 3`, porque desde la Fase 2 vamos a decodificar audio y ahí la velocidad importa más que 1,5 MB. Lo volvemos a medir en la Fase 7 con todo el código adentro.

## Calidad

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `unwrap()` está prohibido por lint (`clippy::unwrap_used = "deny"`) en todo el workspace.
- CI en GitHub Actions (Ubuntu y Windows): fmt, clippy, tests y build release.

## Aviso legal

El uso de contenido de YouTube y de Spotify queda bajo responsabilidad de cada usuario y de los términos de servicio de cada plataforma. Surco no aloja ni redistribuye contenido.
