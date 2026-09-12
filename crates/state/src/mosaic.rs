use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use futures::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, HttpClient};
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

pub(crate) const TILES: usize = 4;

const TILE: u32 = 256;
const SIDE: u32 = TILE * 2;
const COLUMNS: u32 = 2;

fn path(id: &str, stamp: u32) -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sonora")
        .join("mosaics")
        .join(format!("{}-{stamp}.png", sanitised(id)))
}

fn sanitised(id: &str) -> String {
    id.chars()
        .map(|letter| match letter.is_ascii_alphanumeric() {
            true => letter,
            false => '_',
        })
        .collect()
}

fn located(path: &Path) -> String {
    format!("file://{}", path.display())
}

pub(crate) fn cached(id: &str, stamp: u32) -> Option<String> {
    let path = path(id, stamp);
    path.is_file().then(|| located(&path))
}

pub(crate) async fn build(
    http: Arc<dyn HttpClient>,
    id: &str,
    stamp: u32,
    covers: Vec<String>,
) -> Result<String> {
    if covers.is_empty() {
        bail!("cannot build a mosaic from no covers");
    }

    let mut tiles = Vec::with_capacity(TILES);
    for url in covers.iter().take(TILES) {
        let bytes = fetch(&http, url).await?;
        tiles.push(image::load_from_memory(&bytes).context("cannot decode a cover")?);
    }

    let canvas = compose(&tiles);
    let path = path(id, stamp);
    let parent = path.parent().context("cannot place the mosaic")?;
    fs::create_dir_all(parent).context("cannot create the mosaic cache")?;
    canvas
        .save_with_format(&path, ImageFormat::Png)
        .context("cannot write the mosaic")?;

    Ok(located(&path))
}

/// Lays the tiles into a two by two grid, top left first. Fewer than four leaves the rest of the
/// canvas clear rather than repeating one, so a folder holding a single playlist reads as one
/// cover in a grid of empty slots.
fn compose(tiles: &[DynamicImage]) -> RgbaImage {
    let mut canvas = RgbaImage::new(SIDE, SIDE);
    for (index, tile) in tiles.iter().take(TILES).enumerate() {
        let slot = index as u32;
        let tile = tile
            .resize_to_fill(TILE, TILE, FilterType::Triangle)
            .to_rgba8();
        image::imageops::replace(
            &mut canvas,
            &tile,
            i64::from(slot % COLUMNS * TILE),
            i64::from(slot / COLUMNS * TILE),
        );
    }

    canvas
}

async fn fetch(http: &Arc<dyn HttpClient>, url: &str) -> Result<Vec<u8>> {
    let mut response = http
        .get(url, AsyncBody::empty(), true)
        .await
        .context("cannot fetch a cover")?;
    if !response.status().is_success() {
        bail!("a cover request answered {}", response.status());
    }

    let mut bytes = Vec::new();
    response
        .body_mut()
        .read_to_end(&mut bytes)
        .await
        .context("cannot read a cover")?;

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_never_escapes_the_cache_directory() {
        let built = path("../../etc/passwd", 4);

        assert_eq!(
            built.file_name().and_then(|name| name.to_str()),
            Some("______etc_passwd-4.png")
        );
    }

    fn solid(red: u8, green: u8, blue: u8) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            600,
            600,
            image::Rgba([red, green, blue, 255]),
        ))
    }

    fn quarters() -> RgbaImage {
        compose(&[
            solid(255, 0, 0),
            solid(0, 255, 0),
            solid(0, 0, 255),
            solid(255, 255, 0),
        ])
    }

    #[test]
    fn every_pixel_of_a_mosaic_is_opaque() {
        let canvas = quarters();

        assert!(canvas.pixels().all(|pixel| pixel.0[3] == 255));
    }

    #[test]
    fn the_seam_leaves_no_gap_between_tiles() {
        let canvas = quarters();
        let last = TILE - 1;

        assert_eq!(canvas.get_pixel(last, 0).0, [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(TILE, 0).0, [0, 255, 0, 255]);
        assert_eq!(canvas.get_pixel(0, last).0, [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(0, TILE).0, [0, 0, 255, 255]);
    }

    #[test]
    fn each_tile_fills_its_own_quarter() {
        let canvas = quarters();
        let mid = TILE / 2;

        assert_eq!(canvas.get_pixel(mid, mid).0, [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(TILE + mid, mid).0, [0, 255, 0, 255]);
        assert_eq!(canvas.get_pixel(mid, TILE + mid).0, [0, 0, 255, 255]);
        assert_eq!(
            canvas.get_pixel(TILE + mid, TILE + mid).0,
            [255, 255, 0, 255]
        );
    }
}
