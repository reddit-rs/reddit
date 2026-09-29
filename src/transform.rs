//! Optional lossy image transforms applied while saving media.
//!
//! `--convert` rewrites JPEG/PNG/BMP stills to a single format and
//! `--max-size` downscales anything exceeding a bounding box (never upscaling).
//! GIF and WebP can be animated and are left untouched, as are videos and
//! subreddit art. Images that already have the requested format and size are
//! written unchanged, so no re-encoding happens without a reason.

use crate::clean::{is_post_still, item_format};
use crate::download::TransformOptions;
use crate::models::{ManifestItem, MediaFormat};
use anyhow::{Context, Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader, Rgb, RgbImage,
};
use std::io::Cursor;

/// Outcome of [`apply`].
pub(crate) enum Applied {
    /// Write the original bytes.
    Unchanged,
    /// Write these bytes instead.
    Transformed {
        bytes: Vec<u8>,
        /// The container format changed (e.g. PNG → JPEG).
        converted: bool,
        /// The pixel dimensions changed.
        resized: bool,
    },
}

/// Transform `data` when the item and options call for it. GIF/WebP stills are
/// returned unchanged so animation can never be lost.
pub(crate) fn apply(data: &[u8], item: &ManifestItem, opts: &TransformOptions) -> Result<Applied> {
    if opts.convert.is_none() && opts.max_size.is_none() {
        return Ok(Applied::Unchanged);
    }
    if !is_post_still(item) {
        return Ok(Applied::Unchanged);
    }
    // the format we planned for the file name must be one we can decode/encode
    if !item_format(item).is_some_and(MediaFormat::is_static_image) {
        return Ok(Applied::Unchanged);
    }
    let actual = image::guess_format(data)
        .ok()
        .and_then(format_of)
        .with_context(|| format!("unsupported image data for '{}'", item.id))?;
    if !actual.is_static_image() {
        return Ok(Applied::Unchanged); // GIF/WebP: keep as-is
    }

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .context("cannot read image header")?;
    let (width, height) = reader.into_dimensions().context("cannot read image size")?;

    let target = opts
        .convert
        .filter(|f| f.is_static_image())
        .unwrap_or(actual);
    let resized_to = opts
        .max_size
        .and_then(|(max_w, max_h)| scale_down(width, height, max_w, max_h));
    if target == actual && resized_to.is_none() {
        return Ok(Applied::Unchanged);
    }

    let mut img = decode(data)?;
    if let Some((new_w, new_h)) = resized_to {
        img = img.resize_exact(new_w, new_h, FilterType::Lanczos3);
    }
    Ok(Applied::Transformed {
        bytes: encode(&img, target, opts.quality)?,
        converted: target != actual,
        resized: resized_to.is_some(),
    })
}

/// Size after fitting `w`x`h` into the `max_w`x`max_h` box; `None` when the
/// image already fits (never upscales).
fn scale_down(w: u32, h: u32, max_w: u32, max_h: u32) -> Option<(u32, u32)> {
    if max_w == 0 || max_h == 0 || (w <= max_w && h <= max_h) {
        return None;
    }
    let scale = f64::min(max_w as f64 / w as f64, max_h as f64 / h as f64);
    let new_w = ((w as f64) * scale).round().max(1.0) as u32;
    let new_h = ((h as f64) * scale).round().max(1.0) as u32;
    Some((new_w.min(w), new_h.min(h)))
}

fn format_of(format: ImageFormat) -> Option<MediaFormat> {
    match format {
        ImageFormat::Jpeg => Some(MediaFormat::Jpg),
        ImageFormat::Png => Some(MediaFormat::Png),
        ImageFormat::Gif => Some(MediaFormat::Gif),
        ImageFormat::WebP => Some(MediaFormat::Webp),
        ImageFormat::Bmp => Some(MediaFormat::Bmp),
        _ => None,
    }
}

/// Decode the image, applying its EXIF orientation so photos are not rotated
/// when they are re-encoded.
fn decode(data: &[u8]) -> Result<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .context("cannot read image header")?;
    let mut decoder = reader.into_decoder().context("cannot decode image")?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).context("cannot decode image")?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Re-encode an image in `target` format. JPEG loses alpha, which is composited
/// onto white.
fn encode(img: &DynamicImage, target: MediaFormat, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match target {
        MediaFormat::Jpg => {
            let rgb = flatten_on_white(img);
            JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .encode(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
                .context("cannot encode JPEG")?;
        }
        MediaFormat::Png => img
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .context("cannot encode PNG")?,
        other => bail!("cannot encode {other:?}"),
    }
    Ok(out)
}

fn flatten_on_white(img: &DynamicImage) -> RgbImage {
    let rgba = img.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let alpha = u16::from(p[3]);
        let blend = |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
        Rgb([blend(p[0]), blend(p[1]), blend(p[2])])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn item(kind: &str, ext: &str) -> ManifestItem {
        ManifestItem {
            folder: "posts".into(),
            id: "abc".into(),
            kind: kind.into(),
            url: format!("https://i.redd.it/abc.{ext}"),
            fallback: None,
            ext: Some(ext.into()),
            index: None,
            width: None,
            height: None,
        }
    }

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbaImage::new(w, h);
        for (_, _, p) in img.enumerate_pixels_mut() {
            *p = Rgba([200, 50, 50, 255]);
        }
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn jpg_bytes(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::new_rgb8(w, h)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    fn options(convert: Option<MediaFormat>, max_size: Option<(u32, u32)>) -> TransformOptions {
        TransformOptions {
            convert,
            max_size,
            quality: 85,
        }
    }

    #[test]
    fn converts_png_to_jpeg_and_flattens_alpha() {
        // fully transparent PNG: everything must composite onto white
        let mut img = RgbaImage::new(40, 30);
        for (_, _, p) in img.enumerate_pixels_mut() {
            *p = Rgba([255, 0, 0, 0]);
        }
        let mut data = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut data), ImageFormat::Png)
            .unwrap();

        let applied = apply(
            &data,
            &item("image", "png"),
            &options(Some(MediaFormat::Jpg), None),
        )
        .unwrap();
        let Applied::Transformed {
            bytes,
            converted,
            resized,
        } = applied
        else {
            panic!("expected a conversion");
        };
        assert!(converted && !resized);
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Jpeg);
        let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (40, 30));
        let px = img.get_pixel(20, 15).0;
        assert!(
            px.iter().all(|&c| c >= 250),
            "transparency must flatten onto white, got {px:?}"
        );
    }

    #[test]
    fn resizes_into_the_bounding_box() {
        let data = png_bytes(2000, 1000);
        let applied = apply(
            &data,
            &item("image", "png"),
            &options(None, Some((1344, 1792))),
        )
        .unwrap();
        let Applied::Transformed {
            bytes,
            converted,
            resized,
        } = applied
        else {
            panic!("expected a resize");
        };
        assert!(resized && !converted);
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Png);
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (1344, 672));
    }

    #[test]
    fn leaves_matching_images_byte_identical() {
        let data = jpg_bytes(800, 600);
        let applied = apply(
            &data,
            &item("image", "jpg"),
            &options(Some(MediaFormat::Jpg), Some((1344, 1792))),
        )
        .unwrap();
        assert!(matches!(applied, Applied::Unchanged));
    }

    #[test]
    fn never_touches_gif_webp_videos_or_art() {
        let data = png_bytes(2000, 2000);
        for (kind, ext) in [
            ("image", "gif"),
            ("image", "webp"),
            ("video", "mp4"),
            ("icon", "png"),
            ("banner", "png"),
        ] {
            let mut it = item(kind, ext);
            if kind == "icon" || kind == "banner" {
                it.folder = String::new();
            }
            let applied = apply(
                &data,
                &it,
                &options(Some(MediaFormat::Jpg), Some((100, 100))),
            )
            .unwrap();
            assert!(
                matches!(applied, Applied::Unchanged),
                "{kind}.{ext} must stay untouched"
            );
        }
    }

    #[test]
    fn scale_down_never_upscales() {
        assert_eq!(scale_down(100, 100, 1000, 1000), None);
        assert_eq!(scale_down(2688, 3584, 1344, 1792), Some((1344, 1792)));
        assert_eq!(scale_down(4000, 1000, 1344, 1792), Some((1344, 336)));
    }
}
