//! Actualizaciones: busca la última versión publicada en GitHub y la instala
//! con el mismo formato con el que se instaló Surco (instalador o portable
//! en Windows, AppImage, .deb o portable en Linux, .dmg en macOS).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::{Error, Result};

/// Última versión publicada (la API sigue al repo aunque se renombre).
const LATEST_URL: &str = "https://api.github.com/repos/devMathiuX/Surco/releases/latest";

/// Página de descargas, para abrir en el navegador si no se puede instalar sola.
pub const RELEASES_PAGE: &str = "https://github.com/devMathiuX/Surco/releases/latest";

/// Versión publicada más nueva que la que corre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Sin la `v` (por ejemplo `0.5.6`).
    pub version: String,
    pub page: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// Cómo está instalado Surco: define qué se baja y cómo se reemplaza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Install {
    /// Windows, con el instalador (hay un `unins000.exe` al lado).
    WindowsSetup { exe: PathBuf, all_users: bool },
    /// Windows, el `.zip` portable.
    WindowsPortable { exe: PathBuf },
    /// Linux, AppImage (ruta del `.AppImage`).
    AppImage { path: PathBuf },
    /// Linux, paquete `.deb` (o el repositorio APT).
    Deb,
    /// Linux, el `.tar.gz` portable.
    LinuxPortable { exe: PathBuf },
    /// macOS: se abre el `.dmg` y el usuario arrastra la app.
    MacOs,
}

/// Qué hacer después de instalar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// Ya quedó instalada: hay que cerrar Surco (el comando ya relanza la
    /// nueva, o se relanza `relaunch`).
    Restart { relaunch: Option<PathBuf> },
    /// Se abrió el instalador del sistema (macOS): el usuario sigue ahí.
    Opened,
    /// Linux sin pkexec: se abrió una terminal con `command`; el usuario
    /// escribe su contraseña ahí y después reinicia Surco.
    Terminal { command: String },
    /// Linux sin pkexec ni terminal: el usuario tiene que correr `command`.
    Manual { command: String },
}

/// Compara versiones `x.y.z` (sin la `v`). Lo que no es número cuenta como 0.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate), parts(current));
    for i in 0..3 {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Cliente para la API y las descargas de GitHub (pide un User-Agent).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Surco/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

/// Busca la última versión publicada; `None` si no hay una más nueva que
/// `current`.
pub async fn check(client: &reqwest::Client, current: &str) -> Result<Option<Release>> {
    let body = client
        .get(LATEST_URL)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let r: ApiRelease = serde_json::from_slice(&body)
        .map_err(|e| Error::Parse(format!("respuesta de GitHub inválida: {e}")))?;
    let version = r.tag_name.trim_start_matches('v').to_owned();
    if r.draft || r.prerelease || !is_newer(&version, current) {
        return Ok(None);
    }
    Ok(Some(Release {
        version,
        page: r.html_url,
        assets: r.assets,
    }))
}

/// Cómo está instalado el Surco que corre, si se sabe actualizarlo.
pub fn detect() -> Option<Install> {
    let exe = std::env::current_exe().ok()?;
    if cfg!(windows) {
        let dir = exe.parent()?;
        if dir.join("unins000.exe").exists() {
            let all_users = std::env::var_os("ProgramFiles")
                .is_some_and(|pf| exe.starts_with(PathBuf::from(pf)));
            return Some(Install::WindowsSetup { exe, all_users });
        }
        return Some(Install::WindowsPortable { exe });
    }
    if cfg!(target_os = "macos") {
        return Some(Install::MacOs);
    }
    if cfg!(target_os = "linux") {
        if let Some(path) = std::env::var_os("APPIMAGE") {
            return Some(Install::AppImage { path: path.into() });
        }
        if exe.starts_with("/usr") && Path::new("/var/lib/dpkg/info/surco.list").exists() {
            return Some(Install::Deb);
        }
        if exe.starts_with("/usr") {
            // Instalado por otro medio (por ejemplo, a mano en /usr): no tocar.
            return None;
        }
        return Some(Install::LinuxPortable { exe });
    }
    None
}

/// Archivo de la versión que corresponde a esta instalación.
pub fn asset_for<'a>(release: &'a Release, install: &Install) -> Option<&'a Asset> {
    let ends = |suffix: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name.to_ascii_lowercase().ends_with(suffix))
    };
    match install {
        Install::WindowsSetup { .. } => ends("-windows-x64-setup.exe"),
        Install::WindowsPortable { .. } => ends("-windows-x64-portable.zip"),
        Install::AppImage { .. } => ends("-x86_64.appimage"),
        Install::Deb => ends("_amd64.deb"),
        Install::LinuxPortable { .. } => ends("-linux-x86_64.tar.gz"),
        Install::MacOs => ends(".dmg"),
    }
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    tracing::info!(%url, "bajando la actualización");
    let data = client
        .get(url)
        .timeout(Duration::from_secs(600))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(data.to_vec())
}

/// Baja e instala la versión nueva.
pub async fn install(
    client: &reqwest::Client,
    release: &Release,
    install: &Install,
) -> Result<Installed> {
    let asset = asset_for(release, install).ok_or_else(|| {
        Error::Parse("esta versión no trae un archivo para tu instalación".into())
    })?;
    let data = download(client, &asset.url).await?;
    let tmp = std::env::temp_dir().join(&asset.name);
    match install {
        Install::WindowsSetup { exe, all_users } => {
            tokio::fs::write(&tmp, &data).await?;
            run_windows_setup(&tmp, exe, *all_users)?;
            Ok(Installed::Restart { relaunch: None })
        }
        Install::WindowsPortable { exe } => {
            let binary = crate::deno::extract(&data, "surco.exe")?;
            replace_exe(exe, &binary).await?;
            Ok(Installed::Restart {
                relaunch: Some(exe.clone()),
            })
        }
        Install::AppImage { path } => {
            replace_exe(path, &data).await?;
            Ok(Installed::Restart {
                relaunch: Some(path.clone()),
            })
        }
        Install::LinuxPortable { exe } => {
            let binary = untar_gz(&data, "surco")?;
            replace_exe(exe, &binary).await?;
            Ok(Installed::Restart {
                relaunch: Some(exe.clone()),
            })
        }
        Install::Deb => {
            if find_in_path("pkexec").is_none() {
                // Sin pkexec: el paquete queda en Descargas y se instala con
                // sudo desde una terminal (o a mano).
                let dest = downloads_dir().join(&asset.name);
                tokio::fs::write(&dest, &data).await?;
                let command = format!(
                    "sudo apt-get install -y --allow-downgrades {}",
                    shell_quote(&dest.to_string_lossy())
                );
                return Ok(if open_terminal(&command) {
                    Installed::Terminal { command }
                } else {
                    Installed::Manual { command }
                });
            }
            tokio::fs::write(&tmp, &data).await?;
            // pkexec pide la contraseña con una ventana del sistema.
            let status = tokio::process::Command::new("pkexec")
                .args(["apt-get", "install", "-y", "--allow-downgrades"])
                .arg(&tmp)
                .status()
                .await
                .map_err(|e| Error::Parse(format!("no se pudo pedir permisos (pkexec): {e}")))?;
            let _ = tokio::fs::remove_file(&tmp).await;
            if !status.success() {
                return Err(Error::Parse(
                    "no se pudo instalar el paquete (¿se canceló la contraseña?)".into(),
                ));
            }
            Ok(Installed::Restart {
                relaunch: Some(PathBuf::from("/usr/bin/surco")),
            })
        }
        Install::MacOs => {
            let dest = downloads_dir().join(&asset.name);
            tokio::fs::write(&dest, &data).await?;
            Command::new("open").arg(&dest).spawn()?;
            Ok(Installed::Opened)
        }
    }
}

/// Carpeta de descargas del usuario (o la temporal si no hay).
fn downloads_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Downloads"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Busca un programa en el `PATH`.
fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Pone `s` entre comillas simples para la shell.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Terminales conocidas y cómo se les pasa el comando a correr.
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("mate-terminal", &["-x"]),
    ("tilix", &["-e"]),
    ("lxterminal", &["-e"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("foot", &[]),
    ("xterm", &["-e"]),
];

/// Abre una terminal que corre `command` y espera un Enter antes de
/// cerrarse, para que se lea el resultado. `false` si no hay ninguna.
fn open_terminal(command: &str) -> bool {
    let script = format!(
        "echo 'Surco: instalando la versión nueva (te va a pedir tu contraseña)'; {command}; \
         echo; echo 'Listo. Cerrá Surco y abrilo de nuevo. Enter para cerrar.'; read _"
    );
    for (name, flag) in TERMINALS {
        let Some(path) = find_in_path(name) else {
            continue;
        };
        let spawned = Command::new(path)
            .args(*flag)
            .args(["sh", "-c", &script])
            .spawn();
        if spawned.is_ok() {
            return true;
        }
    }
    false
}

/// Lanza el instalador de Windows en silencio cuando Surco ya se cerró, y
/// después abre la versión nueva.
fn run_windows_setup(setup: &Path, exe: &Path, all_users: bool) -> Result<()> {
    let scope = if all_users {
        "/ALLUSERS"
    } else {
        "/CURRENTUSER"
    };
    // `ping` como pausa: `timeout` no anda sin consola.
    let script = format!(
        "ping -n 3 127.0.0.1 >nul & \"{}\" /SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS {scope} & start \"\" \"{}\"",
        setup.display(),
        exe.display()
    );
    let mut cmd = Command::new("cmd");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        cmd.creation_flags(0x0800_0000);
        cmd.arg("/C").raw_arg(format!("\"{script}\""));
    }
    #[cfg(not(windows))]
    cmd.arg("/C").arg(script);
    cmd.spawn()?;
    Ok(())
}

/// Reemplaza el ejecutable que corre por `data`. En Windows el que corre se
/// renombra a `.old` (se borra al abrir la próxima vez); en Unix se pisa
/// con un `rename`, que el proceso viejo no nota.
async fn replace_exe(exe: &Path, data: &[u8]) -> Result<()> {
    let new = exe.with_extension("new");
    tokio::fs::write(&new, data).await?;
    crate::ytdlp::make_executable(&new).await?;
    if cfg!(windows) {
        let old = old_exe(exe);
        let _ = tokio::fs::remove_file(&old).await;
        tokio::fs::rename(exe, &old).await?;
        if let Err(e) = tokio::fs::rename(&new, exe).await {
            let _ = tokio::fs::rename(&old, exe).await;
            return Err(e.into());
        }
    } else {
        tokio::fs::rename(&new, exe).await?;
    }
    Ok(())
}

fn old_exe(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Borra el ejecutable viejo que dejó una actualización portable en Windows.
pub fn cleanup() {
    if cfg!(windows) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::fs::remove_file(old_exe(&exe));
        }
    }
}

/// Abre la versión nueva (después de cerrar esta).
pub fn relaunch(path: &Path) {
    if let Err(e) = Command::new(path).spawn() {
        tracing::warn!(error = %e, path = %path.display(), "no se pudo abrir la versión nueva");
    }
}

/// Saca el archivo cuyo nombre termina en `/name` (o es `name`) de un
/// `.tar.gz`.
fn untar_gz(data: &[u8], name: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut tar = Vec::new();
    flate2::read::GzDecoder::new(data).read_to_end(&mut tar)?;
    let bad = |what: &str| Error::Parse(format!("tar inválido: {what}"));
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|&b| b == 0) {
            break;
        }
        let field = |r: std::ops::Range<usize>| {
            let f = &header[r];
            let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
            String::from_utf8_lossy(&f[..end]).into_owned()
        };
        let mut path = field(0..100);
        let prefix = field(345..500);
        if !prefix.is_empty() {
            path = format!("{prefix}/{path}");
        }
        let size_field = field(124..136);
        let size = usize::from_str_radix(size_field.trim(), 8).map_err(|_| bad("tamaño"))?;
        let kind = header[156];
        let start = at + 512;
        let end = start.checked_add(size).ok_or_else(|| bad("tamaño"))?;
        let body = tar.get(start..end).ok_or_else(|| bad("corto"))?;
        if (kind == b'0' || kind == 0) && (path == name || path.ends_with(&format!("/{name}"))) {
            return Ok(body.to_vec());
        }
        at = start + size.div_ceil(512) * 512;
    }
    Err(Error::Parse(format!("el tar no tiene {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_for_the_shell() {
        assert_eq!(shell_quote("/home/a b/x.deb"), "'/home/a b/x.deb'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.5.6", "0.5.5"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.5.5", "0.5.5"));
        assert!(!is_newer("0.5.4", "0.5.5"));
    }

    fn release() -> Release {
        let a = |n: &str| Asset {
            name: n.into(),
            url: format!("https://x/{n}"),
        };
        Release {
            version: "0.5.6".into(),
            page: RELEASES_PAGE.into(),
            assets: vec![
                a("Surco-0.5.6-windows-x64-setup.exe"),
                a("Surco-0.5.6-windows-x64-portable.zip"),
                a("Surco-0.5.6-x86_64.AppImage"),
                a("surco_0.5.6-1_amd64.deb"),
                a("surco-0.5.6-linux-x86_64.tar.gz"),
                a("Surco-0.5.6-macos-universal.dmg"),
            ],
        }
    }

    #[test]
    fn picks_the_asset_for_each_install() {
        let r = release();
        let name = |i: Install| asset_for(&r, &i).map(|a| a.name.clone()).unwrap();
        let exe = PathBuf::from("surco");
        assert!(name(Install::WindowsSetup {
            exe: exe.clone(),
            all_users: false
        })
        .ends_with("setup.exe"));
        assert!(name(Install::WindowsPortable { exe: exe.clone() }).ends_with(".zip"));
        assert!(name(Install::AppImage { path: exe.clone() }).ends_with(".AppImage"));
        assert!(name(Install::Deb).ends_with(".deb"));
        assert!(name(Install::LinuxPortable { exe }).ends_with(".tar.gz"));
        assert!(name(Install::MacOs).ends_with(".dmg"));
    }

    #[test]
    fn extracts_from_tar_gz() {
        use std::io::Write;
        // Un tar mínimo con `surco-0.5.6/README.md` y `surco-0.5.6/surco`.
        let entry = |path: &str, body: &[u8]| {
            let mut h = [0u8; 512];
            h[..path.len()].copy_from_slice(path.as_bytes());
            let size = format!("{:011o}\0", body.len());
            h[124..136].copy_from_slice(size.as_bytes());
            h[156] = b'0';
            let mut out = h.to_vec();
            out.extend_from_slice(body);
            out.resize(512 + body.len().div_ceil(512) * 512, 0);
            out
        };
        let mut tar = entry("surco-0.5.6/README.md", b"hola");
        tar.extend(entry("surco-0.5.6/surco", b"binario"));
        tar.extend([0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        let data = gz.finish().unwrap();
        assert_eq!(untar_gz(&data, "surco").unwrap(), b"binario");
        assert!(untar_gz(&data, "otro").is_err());
    }
}
