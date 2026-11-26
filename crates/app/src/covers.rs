//! Portadas: la miniatura del video de YouTube, recortada al cuadrado,
//! redimensionada a 64 y 360 px y cacheada en disco.
//!
//! Las playlists importadas sin credenciales no traen la portada del álbum,
//! así que usamos la miniatura del video, que siempre existe.

use std::path::{Path, PathBuf};

use iced::widget::image::Handle;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat};

/// Tamaños que se guardan.
pub const SMALL: u32 = 64;
pub const LARGE: u32 = 360;

/// Carpeta de caché de portadas.
pub fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "Surco").map(|d| d.cache_dir().join("covers"))
}

fn path_for(dir: &Path, video: &str, size: u32) -> PathBuf {
    dir.join(format!("{video}-{size}.jpg"))
}

/// Ruta de la portada grande en caché (para MPRIS/SMTC), si ya existe.
pub fn large_path(video: &str) -> Option<PathBuf> {
    let p = path_for(&cache_dir()?, video, LARGE);
    p.is_file().then_some(p)
}

/// Portada chica de `video`: desde la caché, o la baja y la guarda.
pub async fn load(client: surco_provider::HttpClient, video: String) -> Result<Handle, String> {
    load_size(client, video, SMALL).await
}

/// Portada grande (360 px) de `video`, para mostrarla en grande (lo que
/// suena, el mejor resultado).
pub async fn load_large(
    client: surco_provider::HttpClient,
    video: String,
) -> Result<Handle, String> {
    load_size(client, video, LARGE).await
}

/// Portada de `size` px desde la caché. Si ese tamaño no está (por ejemplo,
/// la caché es de una versión con otro tamaño grande), baja la miniatura y
/// guarda los dos tamaños.
async fn load_size(
    client: surco_provider::HttpClient,
    video: String,
    size: u32,
) -> Result<Handle, String> {
    let dir = cache_dir().ok_or("no hay carpeta de caché")?;
    let path = path_for(&dir, &video, size);
    if !path.is_file() {
        download(&client, &dir, &video).await?;
    }
    tokio::task::spawn_blocking(move || {
        decode_rgba(&std::fs::read(path).map_err(|e| e.to_string())?)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Baja la miniatura del video, la recorta al cuadrado y guarda los dos
/// tamaños.
async fn download(
    client: &surco_provider::HttpClient,
    dir: &Path,
    video: &str,
) -> Result<(), String> {
    // hqdefault (480×360) siempre existe; mqdefault a veces es más chica.
    let url = format!("https://i.ytimg.com/vi/{video}/hqdefault.jpg");
    let bytes = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    let dir = dir.to_owned();
    let video = video.to_owned();
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
            .map_err(|e| e.to_string())?;
        let square = crop_square(img);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for size in [SMALL, LARGE] {
            square
                .resize_exact(size, size, FilterType::CatmullRom)
                .to_rgb8()
                .save_with_format(path_for(&dir, &video, size), ImageFormat::Jpeg)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn decode_rgba(bytes: &[u8]) -> Result<Handle, String> {
    let img = image::load_from_memory_with_format(bytes, ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok(Handle::from_rgba(w, h, img.into_raw()))
}

/// Recorta el centro cuadrado. Las miniaturas 4:3 de YouTube traen el video
/// 16:9 con franjas negras arriba y abajo: se recorta dentro de la imagen real.
pub fn crop_square(img: DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    // Alto útil: si es 4:3, el contenido 16:9 ocupa w*9/16.
    let content_h = if w * 3 == h * 4 { w * 9 / 16 } else { h };
    let side = content_h.min(w);
    let x = (w - side) / 2;
    let y = (h - side) / 2;
    img.crop_imm(x, y, side, side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn crops_inside_letterbox() {
        // 480×360 con franjas negras de 45 px arriba y abajo.
        let mut img = RgbImage::from_pixel(480, 360, Rgb([0, 0, 0]));
        for y in 45..315 {
            for x in 0..480 {
                img.put_pixel(x, y, Rgb([200, 100, 50]));
            }
        }
        let sq = crop_square(DynamicImage::ImageRgb8(img)).to_rgb8();
        assert_eq!(sq.dimensions(), (270, 270));
        // Sin negro en los bordes.
        assert_eq!(*sq.get_pixel(0, 0), Rgb([200, 100, 50]));
        assert_eq!(*sq.get_pixel(269, 269), Rgb([200, 100, 50]));
    }

    #[test]
    fn crops_plain_16_9() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(320, 180));
        assert_eq!(crop_square(img).width(), 180);
    }
}
