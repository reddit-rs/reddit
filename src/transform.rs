//! Opt-in, lossy still-image transforms applied while saving downloads.
//!
//! `--convert` rewrites JPEG/PNG/BMP stills and `--max-size` downscales them
//! into a bounding box (never upscaling). GIF/WebP may be animated and are
//! never touched, nor are videos or subreddit art. Images that already match
//! are stored byte-for-byte.

use crate::models::{ConvertFormat, ManifestItem, MediaFormat, Size};
use anyhow::{Context, Result, bail};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::metadata::Orientation;
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageFormat, ImageReader, Rgb, RgbImage,
};
use std::io::Cursor;

/// Transform settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransformOptions {
    /// Downscale stills so neither side exceeds this box.
    pub max_size: Option<Size>,
    /// Re-encode JPEG/PNG/BMP stills in this format.
    pub convert: Option<ConvertFormat>,
    /// JPEG quality, 1-100.
    pub quality: u8,
}

impl Default for TransformOptions {
    fn default() -> Self {
        Self {
            max_size: None,
            convert: None,
            quality: 85,
        }
    }
}

impl TransformOptions {
    pub fn is_noop(&self) -> bool {
        self.max_size.is_none() && self.convert.is_none()
    }

    /// Whether `item` is a still these options may rewrite.
    pub(crate) fn applies_to(&self, item: &ManifestItem) -> bool {
        !self.is_noop()
            && item.kind.is_post_still()
            && item.format.is_some_and(MediaFormat::is_static_image)
    }
}

/// Result of [`apply`].
#[derive(Debug)]
pub(crate) enum Applied {
    /// Store the downloaded bytes.
    Unchanged,
    Transformed {
        bytes: Vec<u8>,
        converted: bool,
        resized: bool,
    },
}

/// Sniff the container format of downloaded bytes.
pub(crate) fn sniff(data: &[u8]) -> Option<MediaFormat> {
    match image::guess_format(data).ok()? {
        ImageFormat::Jpeg => Some(MediaFormat::Jpg),
        ImageFormat::Png => Some(MediaFormat::Png),
        ImageFormat::Gif => Some(MediaFormat::Gif),
        ImageFormat::WebP => Some(MediaFormat::Webp),
        ImageFormat::Bmp => Some(MediaFormat::Bmp),
        _ => None,
    }
}

/// Apply `opts` to downloaded `data`. CPU-bound: call from a blocking task.
pub(crate) fn apply(data: &[u8], item: &ManifestItem, opts: &TransformOptions) -> Result<Applied> {
    if !opts.applies_to(item) {
        return Ok(Applied::Unchanged);
    }
    let actual = sniff(data).context("downloaded data is not a supported image")?;
    if !actual.is_static_image() {
        bail!("server sent {actual}, which cannot be converted");
    }

    let reader = ImageReader::with_format(Cursor::new(data), image_format(actual));
    let (width, height) = reader.into_dimensions().context("cannot read image size")?;
    let target = opts.convert.map_or(actual, MediaFormat::from);
    let resize_to = opts.max_size.and_then(|max| scale_down(width, height, max));
    if target == actual && resize_to.is_none() {
        return Ok(Applied::Unchanged);
    }

    let mut image = decode(data, actual)?;
    if let Some((w, h)) = resize_to {
        image = image.resize_exact(w, h, FilterType::Lanczos3);
    }
    Ok(Applied::Transformed {
        bytes: encode(&image, target, opts.quality)?,
        converted: target != actual,
        resized: resize_to.is_some(),
    })
}

fn image_format(format: MediaFormat) -> ImageFormat {
    match format {
        MediaFormat::Png => ImageFormat::Png,
        MediaFormat::Bmp => ImageFormat::Bmp,
        _ => ImageFormat::Jpeg,
    }
}

/// Size after fitting `w`x`h` into `max`; `None` when it already fits.
fn scale_down(w: u32, h: u32, max: Size) -> Option<(u32, u32)> {
    let (max_w, max_h) = (max.width.get(), max.height.get());
    if w <= max_w && h <= max_h {
        return None;
    }
    let scale = f64::min(
        f64::from(max_w) / f64::from(w),
        f64::from(max_h) / f64::from(h),
    );
    let fit = |side: u32| ((f64::from(side) * scale).round() as u32).clamp(1, side);
    Some((fit(w), fit(h)))
}

/// Decode, applying EXIF orientation so re-encoded photos stay upright.
fn decode(data: &[u8], format: MediaFormat) -> Result<DynamicImage> {
    let mut decoder = ImageReader::with_format(Cursor::new(data), image_format(format))
        .into_decoder()
        .context("cannot decode image")?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).context("cannot decode image")?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Encode as `target`; JPEG has no alpha, so transparency becomes white.
fn encode(image: &DynamicImage, target: MediaFormat, quality: u8) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match target {
        MediaFormat::Jpg => {
            let rgb = flatten_on_white(image);
            JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100))
                .encode(&rgb, rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
                .context("cannot encode JPEG")?;
        }
        MediaFormat::Png => image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .context("cannot encode PNG")?,
        MediaFormat::Bmp => image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Bmp)
            .context("cannot encode BMP")?,
        other => bail!("cannot encode {other}"),
    }
    Ok(out)
}

fn flatten_on_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let alpha = u16::from(a);
        let blend = |c: u8| ((u16::from(c) * alpha + 255 * (255 - alpha)) / 255) as u8;
        Rgb([blend(r), blend(g), blend(b)])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AssetKind;
    use image::{Rgba, RgbaImage};

    fn item(kind: AssetKind, format: MediaFormat) -> ManifestItem {
        ManifestItem {
            post_id: "abc".into(),
            kind,
            index: None,
            url: format!("https://i.redd.it/abc.{format}"),
            fallback: None,
            format: Some(format),
            width: None,
            height: None,
        }
    }

    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let image = RgbaImage::from_pixel(w, h, Rgba([200, 0, 0, alpha]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(image)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    fn jpg(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        DynamicImage::new_rgb8(w, h)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    fn options(convert: Option<ConvertFormat>, max_size: Option<(u32, u32)>) -> TransformOptions {
        TransformOptions {
            convert,
            max_size: max_size.and_then(|(w, h)| Size::new(w, h)),
            quality: 85,
        }
    }

    #[test]
    fn png_to_jpeg_flattens_transparency_onto_white() {
        // the manifest already plans the converted extension
        let planned = item(AssetKind::Image, MediaFormat::Jpg);
        let Applied::Transformed {
            bytes,
            converted,
            resized,
        } = apply(
            &png(40, 30, 0),
            &planned,
            &options(Some(ConvertFormat::Jpg), None),
        )
        .unwrap()
        else {
            panic!("expected a conversion");
        };
        assert!(converted && !resized);
        assert_eq!(sniff(&bytes), Some(MediaFormat::Jpg));
        let image = image::load_from_memory(&bytes).unwrap().to_rgb8();
        assert_eq!(image.dimensions(), (40, 30));
        assert!(image.get_pixel(20, 15).0.iter().all(|&c| c >= 250));
    }

    #[test]
    fn resizes_into_the_bounding_box() {
        let Applied::Transformed {
            bytes,
            converted,
            resized,
        } = apply(
            &png(2000, 1000, 255),
            &item(AssetKind::Image, MediaFormat::Png),
            &options(None, Some((1344, 1792))),
        )
        .unwrap()
        else {
            panic!("expected a resize");
        };
        assert!(resized && !converted);
        let image = image::load_from_memory(&bytes).unwrap();
        assert_eq!((image.width(), image.height()), (1344, 672));
    }

    #[test]
    fn matching_images_stay_byte_identical() {
        let applied = apply(
            &jpg(800, 600),
            &item(AssetKind::Gallery, MediaFormat::Jpg),
            &options(Some(ConvertFormat::Jpg), Some((1344, 1792))),
        )
        .unwrap();
        assert!(matches!(applied, Applied::Unchanged));
    }

    #[test]
    fn never_touches_animated_formats_videos_or_art() {
        let data = png(200, 200, 255);
        for (kind, format) in [
            (AssetKind::Image, MediaFormat::Gif),
            (AssetKind::Image, MediaFormat::Webp),
            (AssetKind::Video, MediaFormat::Mp4),
            (AssetKind::Icon, MediaFormat::Png),
            (AssetKind::Banner, MediaFormat::Png),
        ] {
            let applied = apply(
                &data,
                &item(kind, format),
                &options(Some(ConvertFormat::Jpg), Some((10, 10))),
            )
            .unwrap();
            assert!(matches!(applied, Applied::Unchanged), "{kind:?} {format}");
        }
    }

    #[test]
    fn unsupported_data_is_an_error() {
        let planned = item(AssetKind::Image, MediaFormat::Jpg);
        let convert = options(Some(ConvertFormat::Jpg), None);
        assert!(apply(b"not an image", &planned, &convert).is_err());
        let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
        assert!(apply(gif, &planned, &convert).is_err());
    }

    #[test]
    fn scale_down_never_upscales() {
        let size = |w, h| Size::new(w, h).unwrap();
        assert_eq!(scale_down(100, 100, size(1000, 1000)), None);
        assert_eq!(scale_down(2688, 3584, size(1344, 1792)), Some((1344, 1792)));
        assert_eq!(scale_down(4000, 1000, size(1344, 1792)), Some((1344, 336)));
        assert_eq!(scale_down(10000, 1, size(100, 100)), Some((100, 1)));
    }
}
