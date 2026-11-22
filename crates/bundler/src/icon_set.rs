//! `tynd-bundler icon` — renders one square PNG or SVG source into every icon
//! the bundle targets consume: `icon.ico`, `icon.icns`, the Linux hicolor PNG
//! sizes and the MSIX visual assets. Paths of the written files go to stdout.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbaImage};
use rayon::prelude::*;

use crate::pack::icons::{assemble_ico_from_rgba, reject_svg_text_nodes, MAX_ICON_BYTES};

/// Every size in the Windows `.ico` (taskbar, Explorer, title bar, DPI steps).
const ICO_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
/// Freedesktop hicolor theme sizes (`share/icons/hicolor/<n>x<n>/apps`).
pub(crate) const HICOLOR_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512];
/// `.icns` element types holding PNG payloads, with their pixel size.
const ICNS_ENTRIES: &[(&[u8; 4], u32)] = &[
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];
/// MSIX visual assets referenced by the generated `AppxManifest.xml`.
pub(crate) const MSIX_ASSETS: &[(&str, u32, u32)] = &[
    ("StoreLogo.png", 50, 50),
    ("Square44x44Logo.png", 44, 44),
    ("Square71x71Logo.png", 71, 71),
    ("Square150x150Logo.png", 150, 150),
    ("Square310x310Logo.png", 310, 310),
    ("Wide310x150Logo.png", 310, 150),
];
/// A raster source below this size would be upscaled for the 1024 px entries.
const MIN_RASTER_SOURCE: u32 = 512;

/// A decoded icon source that renders at any size.
pub(crate) enum IconSource {
    Svg(Box<usvg::Tree>),
    Raster(DynamicImage),
}

impl IconSource {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let size = fs::metadata(path)
            .with_context(|| format!("icon source not found: {}", path.display()))?
            .len();
        ensure!(
            size <= MAX_ICON_BYTES,
            "icon source {} is larger than {MAX_ICON_BYTES} bytes",
            path.display()
        );
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "svg" => {
                let data = fs::read(path).with_context(|| format!("read {}", path.display()))?;
                reject_svg_text_nodes(&data)?;
                let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
                    .with_context(|| format!("parse SVG {}", path.display()))?;
                Ok(Self::Svg(Box::new(tree)))
            },
            "png" => {
                let image =
                    image::open(path).with_context(|| format!("decode PNG {}", path.display()))?;
                let (width, height) = image.dimensions();
                ensure!(
                    width == height,
                    "the icon source must be square (got {width}x{height}); re-export it at 1:1 \
                     or use an SVG"
                );
                ensure!(
                    width >= MIN_RASTER_SOURCE,
                    "the icon source is {width}x{height}; use at least \
                     {MIN_RASTER_SOURCE}x{MIN_RASTER_SOURCE} (1024x1024 recommended) or an SVG"
                );
                Ok(Self::Raster(image))
            },
            _ => bail!(
                "unsupported icon source {}: use a square .png (1024x1024 recommended) or an .svg",
                path.display()
            ),
        }
    }

    /// Render a `width`x`height` image with the icon fitted and centered
    /// (transparent margins on the long axis).
    pub(crate) fn render(&self, width: u32, height: u32) -> Result<RgbaImage> {
        let side = width.min(height);
        let square = match self {
            Self::Svg(tree) => {
                let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)
                    .with_context(|| format!("allocate a {side}x{side} pixmap"))?;
                let size = tree.size();
                let scale = side as f32 / size.width().max(size.height());
                let offset_x = (side as f32 - size.width() * scale) / 2.0;
                let offset_y = (side as f32 - size.height() * scale) / 2.0;
                let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
                    .post_translate(offset_x, offset_y);
                resvg::render(tree, transform, &mut pixmap.as_mut());
                // tiny-skia stores premultiplied RGBA; PNG wants straight alpha.
                let pixels = pixmap
                    .pixels()
                    .iter()
                    .flat_map(|pixel| {
                        let color = pixel.demultiply();
                        [color.red(), color.green(), color.blue(), color.alpha()]
                    })
                    .collect();
                RgbaImage::from_raw(side, side, pixels).context("pixmap dimensions")?
            },
            Self::Raster(image) => {
                if image.width() == side {
                    image.to_rgba8()
                } else {
                    image
                        .resize_exact(side, side, FilterType::Lanczos3)
                        .to_rgba8()
                }
            },
        };
        if width == height {
            return Ok(square);
        }
        let mut canvas = RgbaImage::new(width, height);
        image::imageops::overlay(
            &mut canvas,
            &square,
            i64::from((width - side) / 2),
            i64::from((height - side) / 2),
        );
        Ok(canvas)
    }

    fn png(&self, width: u32, height: u32) -> Result<Vec<u8>> {
        encode_png(&self.render(width, height)?)
    }
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut output),
            image::ImageFormat::Png,
        )
        .context("encode PNG")?;
    Ok(output)
}

/// Assemble an `.icns` container from PNG payloads (macOS 10.7+ reads them).
pub(crate) fn icns_bytes(entries: &[(&[u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    let total = entries
        .iter()
        .try_fold(8_usize, |total, (_, png)| total.checked_add(8 + png.len()))
        .context("icns too large")?;
    let total = u32::try_from(total).context("icns too large")?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&total.to_be_bytes());
    for (kind, png) in entries {
        let len = u32::try_from(png.len() + 8).context("icns entry too large")?;
        out.extend_from_slice(*kind);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(png);
    }
    Ok(out)
}

/// Write the whole icon set under `out_dir` and return the written paths.
pub(crate) fn generate(source: &Path, out_dir: &Path) -> Result<Vec<PathBuf>> {
    let source = IconSource::load(source)?;
    fs::create_dir_all(out_dir.join("png"))
        .and_then(|()| fs::create_dir_all(out_dir.join("msix")))
        .with_context(|| format!("create {}", out_dir.display()))?;

    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
    files.push((out_dir.join("icon.png"), source.png(1024, 1024)?));

    let ico = ICO_SIZES
        .par_iter()
        .map(|&size| Ok((size, source.render(size, size)?.into_raw())))
        .collect::<Result<Vec<_>>>()?;
    files.push((out_dir.join("icon.ico"), assemble_ico_from_rgba(&ico)?));

    let icns = ICNS_ENTRIES
        .par_iter()
        .map(|&(kind, size)| Ok((kind, source.png(size, size)?)))
        .collect::<Result<Vec<_>>>()?;
    files.push((out_dir.join("icon.icns"), icns_bytes(&icns)?));

    let hicolor = HICOLOR_SIZES
        .par_iter()
        .map(|&size| {
            Ok((
                out_dir.join("png").join(format!("{size}x{size}.png")),
                source.png(size, size)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    files.extend(hicolor);

    let msix = MSIX_ASSETS
        .par_iter()
        .map(|&(name, width, height)| {
            Ok((out_dir.join("msix").join(name), source.png(width, height)?))
        })
        .collect::<Result<Vec<_>>>()?;
    files.extend(msix);

    let mut written = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="#0af"/></svg>"##;

    #[test]
    fn icns_container_lists_png_entries_with_big_endian_lengths() {
        let bytes = icns_bytes(&[(b"ic07", vec![1, 2, 3]), (b"ic08", vec![4])]).unwrap();
        assert_eq!(&bytes[..4], b"icns");
        assert_eq!(
            u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            8 + 11 + 9
        );
        assert_eq!(bytes.len(), 28);
        assert_eq!(&bytes[8..12], b"ic07");
        assert_eq!(u32::from_be_bytes(bytes[12..16].try_into().unwrap()), 11);
        assert_eq!(&bytes[19..23], b"ic08");
    }

    #[test]
    fn generate_writes_every_target_format() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("logo.svg");
        fs::write(&source, SVG).unwrap();
        let out = dir.path().join("icons");
        let written = generate(&source, &out).unwrap();
        assert_eq!(written.len(), 3 + HICOLOR_SIZES.len() + MSIX_ASSETS.len());

        let ico = ico::IconDir::read(fs::File::open(out.join("icon.ico")).unwrap()).unwrap();
        let mut sizes: Vec<u32> = ico.entries().iter().map(ico::IconDirEntry::width).collect();
        sizes.sort_unstable();
        assert_eq!(sizes, ICO_SIZES);
        assert_eq!(&fs::read(out.join("icon.icns")).unwrap()[..4], b"icns");

        let wide = image::open(out.join("msix").join("Wide310x150Logo.png")).unwrap();
        assert_eq!(wide.dimensions(), (310, 150));
        // The 64x32 source is letterboxed: the top-left corner stays transparent.
        let master = image::open(out.join("icon.png")).unwrap().to_rgba8();
        assert_eq!(master.dimensions(), (1024, 1024));
        assert_eq!(master.get_pixel(0, 0)[3], 0);
        assert_eq!(master.get_pixel(512, 512)[3], 255);
        let small = image::open(out.join("png").join("16x16.png")).unwrap();
        assert_eq!(small.dimensions(), (16, 16));
    }

    #[test]
    fn small_or_non_square_raster_sources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small.png");
        RgbaImage::new(128, 128).save(&small).unwrap();
        let error = IconSource::load(&small).err().unwrap().to_string();
        assert!(error.contains("at least 512x512"), "{error}");
        let wide = dir.path().join("wide.png");
        RgbaImage::new(1024, 512).save(&wide).unwrap();
        let error = IconSource::load(&wide).err().unwrap().to_string();
        assert!(error.contains("must be square"), "{error}");
        let error = IconSource::load(&dir.path().join("logo.gif"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("not found"));
    }
}
