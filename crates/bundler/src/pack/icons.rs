use std::{fs, path::Path};

use anyhow::{Context, Result};
use error_stack::{Report, ResultExt as _};
use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use image::{imageops::FilterType, GenericImageView};
use quick_xml::{events::Event, Reader};
use rayon::prelude::*;

use super::{read_file_limited, PackContext, PackError};

/// Windows picks per DPI: 16 for title bar, 32/48 for taskbar/Alt-Tab,
/// 256 for Explorer.
const ICO_SIZES: &[u32] = &[16, 32, 48, 256];
pub(crate) const MAX_ICON_BYTES: u64 = 32 * 1024 * 1024;
// Icon rendering

/// Pass-through when the source already ends in `.ico`.
pub(crate) fn build_ico_bytes(icon: &Path) -> Result<Vec<u8>> {
    ensure_file_size(icon, MAX_ICON_BYTES)?;
    let ext = icon
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "ico" => read_file_limited(icon, MAX_ICON_BYTES),
        "svg" => render_svg_to_ico(icon),
        "png" => render_png_to_ico(icon),
        _ => Err(anyhow::Error::new(PackError::BadIcon {
            path: icon.display().to_string(),
        })),
    }
}

/// Neutral fallback used by installer formats that require an ICO even when
/// the application intentionally keeps the host's embedded default icon.
pub(crate) fn default_ico_bytes() -> Result<Vec<u8>> {
    let raster = ICO_SIZES
        .iter()
        .map(|&size| (size, default_icon_rgba(size)))
        .collect::<Vec<_>>();
    assemble_ico_from_rgba(&raster)
}

/// Deterministic square PNG used by bundle formats that require an icon when
/// the application intentionally keeps the embedded host default.
pub(crate) fn default_png_bytes() -> Result<Vec<u8>> {
    const SIZE: u32 = 256;
    let image = image::RgbaImage::from_raw(SIZE, SIZE, default_icon_rgba(SIZE))
        .ok_or_else(|| anyhow::anyhow!("default icon dimensions are invalid"))?;
    let mut output = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut output),
            image::ImageFormat::Png,
        )
        .context("encode default PNG icon")?;
    Ok(output)
}

fn default_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    let inset = (size / 8).max(1);
    let bar = (size / 7).max(1);
    for y in inset..(size - inset) {
        for x in inset..(size - inset) {
            let offset = ((y * size + x) * 4) as usize;
            rgba[offset] = 34;
            rgba[offset + 1] = 211;
            rgba[offset + 2] = 238;
            rgba[offset + 3] = 255;
        }
    }
    let top = size / 3;
    for y in top..(top + bar).min(size - inset) {
        for x in (size / 4)..(size * 3 / 4) {
            let offset = ((y * size + x) * 4) as usize;
            rgba[offset] = 15;
            rgba[offset + 1] = 23;
            rgba[offset + 2] = 42;
        }
    }
    for y in top..(size * 3 / 4) {
        for x in (size / 2 - bar / 2)..(size / 2 + bar - bar / 2) {
            let offset = ((y * size + x) * 4) as usize;
            rgba[offset] = 15;
            rgba[offset + 1] = 23;
            rgba[offset + 2] = 42;
        }
    }
    rgba
}

fn render_svg_to_ico(svg_path: &Path) -> Result<Vec<u8>> {
    let svg_data = read_file_limited(svg_path, MAX_ICON_BYTES).context("read svg")?;
    reject_svg_text_nodes(&svg_data)?;
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_data, &opt).context("parse svg")?;

    let raster: Vec<(u32, Vec<u8>)> = ICO_SIZES
        .par_iter()
        .map(|&size| -> Result<(u32, Vec<u8>)> {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
                .ok_or_else(|| anyhow::anyhow!("pixmap alloc {size}x{size}"))?;
            // Fit the longest edge to preserve aspect ratio in the square pixmap.
            let svg_size = tree.size();
            let scale = (size as f32) / svg_size.width().max(svg_size.height());
            let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            Ok((size, pixmap.data().to_vec()))
        })
        .collect::<Result<Vec<_>>>()?;

    assemble_ico_from_rgba(&raster)
}

pub(crate) fn reject_svg_text_nodes(svg_data: &[u8]) -> Result<()> {
    let mut reader = Reader::from_reader(svg_data);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element) | Event::Empty(element))
                if element.local_name().as_ref().eq_ignore_ascii_case("text") =>
            {
                return Err(anyhow::Error::new(PackError::IconRender {
                    cause: "SVG icons cannot contain <text> nodes because host fonts make release artifacts non-reproducible. Convert text to vector paths before packaging.".into(),
                }));
            },
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {},
            Err(error) => {
                return Err(anyhow::Error::new(PackError::IconRender {
                    cause: format!("invalid SVG XML: {error}"),
                }));
            },
        }
        buffer.clear();
    }
}

fn render_png_to_ico(png_path: &Path) -> Result<Vec<u8>> {
    let img = image::open(png_path).context("open png")?;
    let (w, h) = img.dimensions();
    if w != h {
        return Err(anyhow::Error::new(PackError::IconRender {
            cause: format!(
                "PNG must be square (got {w}x{h}). Re-export at 1:1 or use an SVG source."
            ),
        }));
    }
    let raster: Vec<(u32, Vec<u8>)> = ICO_SIZES
        .par_iter()
        .map(|&size| -> Result<(u32, Vec<u8>)> {
            let resized = if size == w {
                img.to_rgba8()
            } else {
                img.resize(size, size, FilterType::Lanczos3).to_rgba8()
            };
            Ok((size, resized.into_raw()))
        })
        .collect::<Result<Vec<_>>>()?;
    assemble_ico_from_rgba(&raster)
}

/// `IconDirEntry::encode` auto-picks PNG payloads for entries 256+
/// (natively supported on Vista+).
pub(crate) fn assemble_ico_from_rgba(entries: &[(u32, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut dir = IconDir::new(ResourceType::Icon);
    for (size, rgba) in entries {
        let img = IconImage::from_rgba_data(*size, *size, rgba.clone());
        let size = *size;
        let entry = IconDirEntry::encode(&img)
            .map_err(Report::from)
            .change_context(PackContext::AssembleIco)
            .attach_with(move || format!("entry_size = {size}x{size}"))
            .map_err(|report| anyhow::anyhow!("encode ICO entry: {report:?}"))?;
        dir.add_entry(entry);
    }
    let mut out = Vec::new();
    let entry_count = entries.len();
    dir.write(&mut out)
        .map_err(Report::from)
        .change_context(PackContext::AssembleIco)
        .attach_with(move || format!("entry_count = {entry_count}"))
        .map_err(|report| anyhow::anyhow!("write ICO: {report:?}"))?;
    Ok(out)
}

fn ensure_file_size(path: &Path, max_bytes: u64) -> Result<u64> {
    let size = fs::metadata(path)
        .with_context(|| format!("read metadata {}", path.display()))?
        .len();
    if size > max_bytes {
        return Err(anyhow::Error::new(PackError::AssetTooLarge {
            len: size,
            path: path.display().to_string(),
        }));
    }
    Ok(size)
}
