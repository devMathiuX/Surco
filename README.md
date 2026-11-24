# Surco

Surco es un reproductor de música para la computadora, liviano y gratuito. Pegás el link de una playlist de Spotify o de YouTube y Surco la reproduce buscando cada canción en YouTube. No hace falta cuenta de Spotify Premium ni de ningún otro servicio.

Funciona en **Linux**, **Windows** y **macOS** (Intel y Apple Silicon).

![Surco con una playlist abierta](docs/biblioteca.png)

## Qué podés hacer

- **Importar playlists** de Spotify, YouTube o YouTube Music pegando el link, o desde un archivo CSV.
- **Buscar** canciones, artistas y playlists de YouTube sin salir de la app: los resultados aparecen mientras escribís.
- **Armar tus playlists**: creá las tuyas, agregá canciones desde el buscador o desde otras playlists, cambiales el nombre y ordenalas arrastrando.
- **Descubrir canciones**: al final de cada playlist hay recomendaciones parecidas para sumarle.
- **Escuchar sin cortes**: la próxima canción se prepara mientras suena la actual.
- **Música sin fin**: si escuchás desde el buscador, Surco va sumando canciones parecidas a la cola para que nunca se termine.
- **Radio**: armá una lista de canciones parecidas a la que está sonando.
- **Editar la cola**: agregá, quitá y reordená canciones arrastrándolas.
- **Elegir otro video** si Surco encontró una versión equivocada de una canción (en vivo, cover, etc.).
- **Usar las teclas multimedia** del teclado y los controles de música del sistema.
- **Cambiar el tema**: oscuro, verde, rosa, claro y claro rosa.

Todo lo que importás queda guardado en tu computadora: playlists, videos elegidos, historial y ajustes. La pantalla de **Inicio** te muestra tus playlists y lo último que escuchaste.

## Instalación

### 1. Descargar Surco

Entrá a [Releases](https://github.com/devMathiuX/Surco/releases) y bajá, de la última versión, el archivo que corresponde a tu sistema:

| Sistema | Archivo |
|---|---|
| Windows 10 u 11 | `Surco-<versión>-windows-x64-setup.exe` (instalador) o `…-portable.zip` (sin instalar) |
| macOS 11 o más nuevo | `Surco-<versión>-macos-universal.dmg` |
| Ubuntu, Debian, Linux Mint, Pop!_OS | `surco_<versión>-1_amd64.deb` |
| Cualquier otro Linux | `Surco-<versión>-x86_64.AppImage` |

### 2. Instalarlo

<details open>
<summary><b>Windows</b></summary>

1. Abrí el `setup.exe` que bajaste. No pide permisos de administrador.
2. Si aparece **"Windows protegió su PC"** (SmartScreen), hacé clic en **Más información** y después en **Ejecutar de todas formas**. El aviso sale porque Surco no está firmado con un certificado pago, no porque tenga algo malo.
3. Seguí los pasos del instalador. Surco queda en el menú Inicio.

Si preferís no instalar nada, descomprimí el `.zip` portable y abrí `surco.exe`.

</details>

<details>
<summary><b>macOS</b></summary>

1. Abrí el `.dmg` y arrastrá **Surco** a la carpeta **Aplicaciones**.
2. La primera vez, macOS no la deja abrir con doble clic porque Surco no está registrado en Apple. Hacé **clic derecho sobre Surco > Abrir**, y en el aviso, **Abrir** otra vez. Desde la segunda vez abre normal.
3. Si macOS dice que la app **"está dañada"**, abrí la app Terminal y pegá este comando:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Surco.app
   ```

   Después abrila de nuevo.

</details>

<details>
<summary><b>Linux</b></summary>

**Con el `.deb` (Ubuntu, Debian y derivadas):** abrí una terminal en la carpeta de descargas y corré:

```sh
sudo apt install ./surco_*.deb
```

Surco aparece en el menú de aplicaciones.

**Con el AppImage (cualquier distro):**

```sh
chmod +x Surco-*.AppImage
./Surco-*.AppImage
```

Hace falta una distro de 2022 o más nueva (por ejemplo Ubuntu 22.04 o Debian 12).

</details>

### 3. Instalar yt-dlp y deno

Surco usa dos programas gratuitos para sacar el audio de YouTube:

- **yt-dlp**: si no lo tenés, Surco te muestra el botón **Descargar yt-dlp** al abrir y lo baja solo. No hace falta hacer nada más.
- **deno**: yt-dlp lo necesita para que YouTube no bloquee la reproducción. Instalalo una sola vez:

| Sistema | Cómo instalar deno |
|---|---|
| Windows | En PowerShell: `irm https://deno.land/install.ps1 \| iex` |
| macOS | Con [Homebrew](https://brew.sh): `brew install deno`. Sin Homebrew, en la Terminal: `curl -fsSL https://deno.land/install.sh \| sh` |
| Linux | En una terminal: `curl -fsSL https://deno.land/install.sh \| sh` |

Después de instalar deno, cerrá y volvé a abrir Surco. Si ya tenés `node` o `bun` instalados, Surco los usa y deno no hace falta.

## Cómo se usa

![Inicio](docs/inicio.png)

### Importar una playlist

Entrá a **Importar** en la barra lateral (o apretá **Ctrl+L**), pegá el link y apretá **Enter** o **Importar**. Sirven:

- **Spotify:** `https://open.spotify.com/playlist/…` (el link de "Compartir > Copiar enlace").
- **YouTube y YouTube Music:** `https://www.youtube.com/playlist?list=…`, `https://music.youtube.com/playlist?list=…` o un video que esté dentro de una playlist.
- **Un archivo CSV:** con el botón **Elegir CSV…**. Por ejemplo, uno exportado desde [Exportify](https://exportify.net).

La playlist queda guardada en la barra lateral y sus portadas van apareciendo solas. Hacé clic en una canción para escucharla: Surco busca el video en YouTube y sigue sola con las demás.

**Playlists de Spotify grandes:** se importan enteras, aunque tengan miles de canciones, incluso las hechas por Spotify (como "All Out 80s"). Surco las lee igual que la web de Spotify cuando no iniciaste sesión, así que no hace falta cuenta, Premium ni claves. Si alguna vez solo trae las primeras 100, probá de nuevo más tarde o importá un CSV de Exportify.

### Buscar música

![Buscador](docs/buscador.png)

Escribí en el buscador del **Inicio** o entrá a **Buscar** en la barra lateral. Cuando dejás de escribir aparecen canciones, artistas y playlists:

- Clic en una **canción** para escucharla. Surco sigue con canciones parecidas, así que la música no se corta.
- Clic en un **artista** para ver sus canciones más escuchadas.
- Clic en una **playlist** para verla, escucharla o **Guardar en la biblioteca**.
- El **+** al lado de cada canción la agrega a la cola o a una de tus playlists.

### Tus playlists

- **Crear una:** el **+** al lado de "Tu biblioteca", en la barra lateral, o "+ Nueva playlist" en el menú del **+** de cualquier canción.
- **Agregar canciones:** el **+** de una canción (en el buscador, en otra playlist o en el inicio) y elegí la playlist.
- **Quitar una canción:** el **+** de la canción dentro de la playlist, y **Quitar de…**.
- **Cambiar el nombre:** **Renombrar**, arriba de la playlist. También sirve para guardar una radio: con nombre deja de reemplazarse por la próxima.
- **Cambiar el orden:** arrastrá la playlist en la barra lateral.
- **Recomendadas:** al final de cada playlist hay canciones parecidas. **Agregar** la suma a la playlist, un clic la escucha, y **Otras** trae más.

### La cola de reproducción

Abrila con **Cola** en la barra lateral o con el botón de la lista en la barra de abajo. Ahí ves la canción actual y las que siguen:

- **Arrastrá** una canción para cambiarla de lugar.
- La **×** la saca de la cola.
- Un **clic** la pone a sonar.

### Si suena la versión equivocada

Cuando Surco no está seguro de haber encontrado la canción correcta, la marca como **"dudosa"** en la lista. Hacé clic en "dudosa", o en **Cambiar video** en la barra de abajo, y elegí otro video de la lista. Surco recuerda tu elección.

### Radio

El botón de ondas de la barra de abajo arma la playlist "Radio" con unas 25 canciones parecidas a la que suena. En **Ajustes**, la opción **"Al terminar, seguir con canciones parecidas"** (prendida de entrada) arranca una radio sola cuando se termina la cola.

### Ajustes

![Ajustes](docs/ajustes.png)

El engranaje de arriba a la izquierda abre los ajustes:

- **Tema** de colores.
- **Portadas en la lista** de canciones.
- **Precargar canciones:** baja de antemano las próximas 5 canciones, o toda la playlist, para que no haya que esperar ni dependan de la conexión. Usa hasta 2 GB de disco. **Vaciar** borra todo lo descargado, y en la lista de abajo podés borrar solo lo de una playlist. También está **Borrar descargas** arriba de cada playlist.
- **Spotify:** claves propias de la Web API, opcionales (ver abajo).

### Atajos de teclado

| Atajo | Acción |
|---|---|
| Espacio | Reproducir / pausar |
| ← / → | Retroceder / adelantar 5 segundos |
| Ctrl+← / Ctrl+→ | Canción anterior / siguiente |
| Ctrl+L | Importar una playlist (pegar un link) |
| Ctrl+F | Buscar dentro de la playlist abierta |
| Esc | Cerrar la búsqueda, el selector de video o el menú de una canción |

En macOS, usá **Cmd** en lugar de Ctrl. Las teclas multimedia del teclado también funcionan, y Surco aparece en los controles de música del sistema (el panel de medios de Windows, "Está sonando" en macOS y el widget de música en Linux).

## Solución de problemas

**"No encontré yt-dlp"**
Hacé clic en **Descargar yt-dlp**. Si falla, instalalo a mano siguiendo las [instrucciones de yt-dlp](https://github.com/yt-dlp/yt-dlp/wiki/Installation) y volvé a abrir Surco.

**Las canciones no suenan o dan error una tras otra**
- Revisá que tengas internet.
- Instalá deno (paso 3 de la instalación) y reiniciá Surco.
- Actualizá yt-dlp: YouTube cambia seguido y las versiones viejas dejan de andar. Si lo bajó Surco, borralo de la carpeta de datos (ver abajo) y Surco va a ofrecer bajarlo de nuevo. Si lo instalaste vos, corré `yt-dlp -U`.
- Con VPN o desde algunas redes (trabajo, universidad), YouTube puede pedir "confirmar que no sos un bot". Probá sin VPN o desde otra red.

**Una canción suena en vivo, como cover o es otra canción**
Usá **Cambiar video** y elegí la correcta (ver [Si suena la versión equivocada](#si-suena-la-versión-equivocada)).

**De una playlist de Spotify solo se importaron 100 canciones**
Pasa si Spotify no respondió a la lectura completa. Esperá un rato y volvé a importarla (pegá el mismo link). Si sigue igual, importala como CSV desde [Exportify](https://exportify.net) o, si tenés Spotify Premium, cargá tus claves en **Ajustes > Spotify**.

**Algunas playlists de Spotify no se importan**
Las playlists privadas y los mixes personales (como "Daily Mix" o "Descubrimiento semanal") no se pueden leer sin tu cuenta. Copiá las canciones a una playlist pública tuya y probá con esa, o usá Exportify.

**En Windows o macOS aparece un aviso de seguridad al abrir**
Es normal, porque Surco no está firmado con un certificado pago. Ver los pasos de [instalación](#2-instalarlo).

### Dónde guarda Surco tus datos

| Sistema | Carpeta |
|---|---|
| Windows | `%APPDATA%\Surco\data\` |
| macOS | `~/Library/Application Support/Surco/` |
| Linux | `~/.local/share/surco/` |

Ahí están la biblioteca (`surco.db`), yt-dlp si lo bajó Surco, y la carpeta `logs` con los registros de los últimos 7 días. Si reportás un problema, adjuntá el log del día.

## Preguntas frecuentes

**¿Es gratis? ¿Necesito cuenta?**
Sí, es gratis, y no necesitás cuenta de Spotify, YouTube ni de nada.

**¿De dónde sale el audio?**
De YouTube. Spotify solo se usa para leer la lista de canciones de una playlist.

**¿Necesito claves de Spotify?**
No. Surco importa las playlists públicas enteras sin claves. Las claves de la Web API quedan como respaldo opcional, pero Spotify ahora pide **Premium** para crear una app de desarrollador. Si tenés Premium y querés usarlas: entrá al [panel de desarrolladores de Spotify](https://developer.spotify.com/dashboard), creá una app (cualquier nombre, y como Redirect URI `http://127.0.0.1:8888/callback`), y copiá el **Client ID** y el **Client secret** en **Ajustes > Spotify > Guardar**.

**¿Funciona sin internet?**
Solo con las canciones que ya precargaste (ver Ajustes).

## Para desarrolladores

Cómo compilar Surco, cómo está armado por dentro y las decisiones técnicas: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Aviso legal

El uso de contenido de YouTube y de Spotify queda bajo responsabilidad de cada usuario y de los términos de servicio de cada plataforma. Surco no aloja ni redistribuye contenido.
