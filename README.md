# Surco

Reproductor de música de escritorio, ultraliviano, escrito en Rust. Importa playlists de Spotify y reproduce cada canción usando YouTube como fuente de audio (vía `yt-dlp`), sin cuenta Premium. Corre en Linux y Windows (x86_64) con la misma base de código.

> **Estado:** Fase 3 de 7 (Spotify). La app importa playlists de Spotify (Web API, página pública o CSV) y las muestra. Reproducir cada canción desde YouTube llega con el resolver (Fase 4); por ahora el panel de prueba de audio sigue disponible en la pantalla de inicio.

## Requisitos

- Rust estable ≥ 1.88 (`rustup`).
- **Linux:** para compilar, `libasound2-dev` (ALSA; en Debian/Ubuntu `sudo apt install libasound2-dev`, en Fedora `alsa-lib-devel`). Para correr, X11 o Wayland con `libxkbcommon` (viene con cualquier escritorio).
- **Windows:** toolchain MSVC. Nada más.
- [`yt-dlp`](https://github.com/yt-dlp/yt-dlp): si no está en el `PATH`, la app ofrece descargarlo de los releases oficiales a su carpeta de datos (`~/.local/share/surco/` en Linux, `%APPDATA%\Surco\data\` en Windows). `ffmpeg` no hace falta por ahora.
- Un runtime de JavaScript para yt-dlp: desde fines de 2025 yt-dlp lo necesita para YouTube. Recomendado [`deno`](https://deno.com); si no está, Surco usa `node`, `bun` o QuickJS si los encuentra en el `PATH`.

## Cómo correrlo

```sh
cargo run                 # debug
cargo run --release       # release (opt-level 3)
cargo build --profile release-z   # variante optimizada para tamaño
```

## Probar el audio (Fase 2)

1. `cargo run --release`
2. El campo ya trae un link de prueba ("Big Buck Bunny", Blender Foundation, CC BY). Apretá **Reproducir** (o Enter).
3. Espacio pausa y reanuda; ←/→ mueven 5 s; la barra de progreso también hace seek; el volumen está a la derecha.

El campo acepta también un ID de video, una URL directa a un `.m4a` o una ruta local, útil para probar sin YouTube.

Variables de entorno útiles:

- `RUST_LOG=surco=debug` para más detalle en los logs.
- `SURCO_NULL_AUDIO=1` para correr sin placa de sonido (descarta el audio en tiempo real; lo usan los tests).

## Importar playlists (Fase 3)

En la barra lateral, pegá en el campo de abajo (o apretá **Ctrl+L**) cualquiera de estos y Enter / **+ Importar**:

- `https://open.spotify.com/playlist/{id}` (con o sin `?si=…`, también `/intl-es/…` y `/embed/…`)
- `spotify:playlist:{id}`
- la ruta a un `.csv` exportado con [Exportify](https://exportify.net)

Surco prueba tres estrategias en cascada (trait `PlaylistSource` en `crates/spotify`):

1. **Web API de Spotify** con Client Credentials, solo si pusiste `client_id`/`client_secret` en `config.toml` (`~/.config/surco/config.toml` en Linux, `%APPDATA%\Surco\config\config.toml` en Windows; se crea una plantilla en el primer arranque). Paginación completa y manejo de 429 con `Retry-After`. **Ojo:** desde febrero de 2026 las apps en *Development Mode* tienen restricciones: el endpoint pasó a `/playlists/{id}/items` y, para playlists ajenas, Spotify puede devolver solo los metadatos. En ese caso Surco sigue con la estrategia 2.
2. **Página embed pública** (`open.spotify.com/embed/playlist/{id}`), sin credenciales. Lee el JSON que trae embebido (`__NEXT_DATA__`). Trae título, artistas y duración, pero **no** álbum ni ISRC, y **solo las primeras 100 canciones** (la app lo avisa). No es una API pública: puede romperse si Spotify cambia el HTML.
3. **CSV** (Exportify, formato viejo y nuevo, o uno hecho a mano con `title,artist,album,duration`). Trae la playlist completa con álbum e ISRC. Es la vía recomendada para playlists de más de 100 canciones sin credenciales.

Las playlists importadas viven en memoria hasta la Fase 6 (persistencia).

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

### Audio

```
yt-dlp -f "bestaudio[ext=m4a]/bestaudio" --print ...  ->  URL de googlevideo (caché en memoria, 5 h)
HttpStream: bloques de 512 KiB con Range, en memoria, prioriza el bloque que pide el seek
hilo decodificador (symphonia, AAC) -> buffer de muestras (2 s antes de sonar, hasta 5 s)
RingSource (rodio/cpal): nunca bloquea; si la red no llegó, suena silencio en vez de colgarse
```

- Si la URL de stream vence (HTTP 403), se re-resuelve una vez con yt-dlp y se retoma desde la misma posición.
- yt-dlp corre con timeout (45 s) y, en Windows, sin abrir consola (`CREATE_NO_WINDOW`).
- **rodio** se usa solo como salida y mezclador (feature `playback`, sin decodificadores propios). La decodificación es nuestra, con symphonia 0.5 (la misma versión que usa rodio).
- Errores (sin yt-dlp, red caída, formato no soportado) quedan en el estado `Error` y se muestran en la UI; la app nunca se cae.

Medido en la Fase 2 (Linux, Xvfb, reproduciendo un m4a de 3 min por HTTP): **~23 MB de RAM** y ~1,5 % de CPU. Binario: **10,1 MB** (opt 3) / **7,8 MB** (opt z). El salto desde 6,4 MB viene de tokio, reqwest + rustls, symphonia y cpal.

## Calidad

- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Tests: parser de la salida de yt-dlp, caché con TTL, `HttpStream` contra un servidor HTTP local (seek antes de terminar la descarga, 403), y el motor de punta a punta con salida nula (sonar, pausar, seek, fin, errores). Hay un test con un yt-dlp falso que primero devuelve una URL vencida.
- Los archivos de prueba (`crates/player/tests/fixtures/*.m4a`) son tonos de 8 s generados con ffmpeg: `ffmpeg -f lavfi -i sine=frequency=440:duration=8 -ac 2 -c:a aac -b:a 64k tone.m4a` (y `-movflags +dash+global_sidx` para la versión fragmentada, como las de YouTube).
- `unwrap()` está prohibido por lint (`clippy::unwrap_used = "deny"`) en todo el workspace.
- Spotify: parser de links (todas las variantes), CSV (comillas, saltos de línea, BOM, formatos de Exportify), página embed, y la Web API contra un servidor local que imita a Spotify (token, 250 canciones en 3 páginas, 429 con `Retry-After`, formato viejo `tracks` y nuevo `items`, y la cascada cayendo a la embed cuando la API esconde las canciones). `crates/spotify/tests/fixtures/exportify_sample.csv` es un CSV de ejemplo de 120 canciones.
- Pruebas contra la red real, ignoradas por defecto:
  - `cargo test -p surco-spotify --test import -- --ignored --nocapture` (embed real; con `SPOTIFY_CLIENT_ID`/`SPOTIFY_CLIENT_SECRET` también la Web API)
  - `cargo test -p surco-player --test network -- --ignored --nocapture` (streaming HTTPS real con pausa y seek; con `SURCO_TEST_YOUTUBE=<id>` también YouTube)
- CI en GitHub Actions (Ubuntu y Windows): fmt, clippy, tests y build release.

## Limitaciones conocidas

- **Opus/WebM:** si YouTube solo ofrece el audio en WebM/Opus, por ahora se muestra un error. El fallback (crate `opus` o ffmpeg) queda pendiente; casi todos los videos tienen m4a.
- En un MP4 fragmentado sin duración en el contenedor (URLs directas) no hay barra de progreso; con YouTube la duración la da yt-dlp.
- **YouTube y redes "marcadas":** desde IPs de datacenter, VPN o redes con mucho tráfico, YouTube puede pedir "confirmar que no sos un bot" y yt-dlp falla. La app lo muestra como un error claro (no crashea). En una conexión hogareña normal no pasa.
- **Spotify sin credenciales:** máximo 100 canciones por playlist (ver arriba). Para importar un CSV hay que pegar la ruta; el selector de archivos llega con la UI completa (Fase 5).
- La canción entera queda en memoria mientras suena (~4 MB por cada 4 minutos). Para audios de horas habría que limitarlo.

## Aviso legal

El uso de contenido de YouTube y de Spotify queda bajo responsabilidad de cada usuario y de los términos de servicio de cada plataforma. Surco no aloja ni redistribuye contenido.
