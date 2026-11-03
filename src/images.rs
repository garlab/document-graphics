//! Bounded portable image decoding, with optional isolated region/native decoding.
//! Byte decoding does not access files, fonts, networking or OS codec services.
use crate::Raster;
use image::{ImageDecoder, ImageReader};
use std::{
    io::Cursor,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(all(feature = "image-worker", unix))]
mod backend;
#[cfg(all(feature = "image-native", target_os = "macos"))]
mod macos;
#[cfg(all(feature = "image-worker", unix))]
mod png_rows;
#[cfg(all(feature = "image-worker", unix))]
mod portable;
#[cfg(feature = "image-worker")]
mod protocol;
#[cfg(feature = "image-worker")]
mod regions;
#[cfg(all(feature = "image-worker", unix))]
mod svg;
#[cfg(feature = "image-worker")]
pub mod worker;
#[cfg(feature = "image-worker")]
pub use regions::*;
#[cfg(feature = "image-worker")]
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
#[cfg(all(feature = "image-worker", unix))]
const SVG_SOURCE_LIMIT: usize = 32 * 1024 * 1024;
#[cfg(all(feature = "image-worker", unix))]
const PORTABLE_PIXEL_LIMIT: u64 = 16 * 1024 * 1024;
pub const CANCELLED: &str = "Image request cancelled";
#[cfg(feature = "image-worker")]
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Png,
    Jpeg,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Invalid,
    Limit,
    Cancelled,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Invalid image",
            Self::Limit => "Image exceeds decoding limits",
            Self::Cancelled => CANCELLED,
        })
    }
}
impl std::error::Error for Error {}

/// Limits for in-process PNG/JPEG thumbnails. For large files or hard deadlines,
/// use the image worker instead. Larger configured values cannot exceed the hard
/// ceilings: 32 MiB encoded, 16 megapixels, 128 MiB decoded and 4 megapixels output.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub encoded_bytes: usize,
    pub source_edge: u32,
    pub source_pixels: u64,
    pub decoded_bytes: u64,
    pub output_pixels: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            encoded_bytes: 8 * 1024 * 1024,
            source_edge: 8192,
            source_pixels: 4_000_000,
            decoded_bytes: 32 * 1024 * 1024,
            output_pixels: 1_000_000,
        }
    }
}
impl Limits {
    fn valid(self) -> bool {
        self.encoded_bytes > 0
            && self.encoded_bytes <= 32 * 1024 * 1024
            && self.source_edge > 0
            && self.source_edge <= 32768
            && self.source_pixels > 0
            && self.source_pixels <= 16 * 1024 * 1024
            && self.decoded_bytes > 0
            && self.decoded_bytes <= 128 * 1024 * 1024
            && self.output_pixels > 0
            && self.output_pixels <= 4 * 1024 * 1024
    }
}
/// Decode a fitted, orientation-corrected still image to straight RGBA. No upscaling.
/// Run off the UI thread. Cancellation is checked around codec calls, not inside
/// them; allocation and source limits are enforced before full decoding.
pub fn decode(
    bytes: &[u8],
    format: Format,
    bounds: (u32, u32),
    limits: Limits,
    cancel: &AtomicBool,
) -> Result<Raster, Error> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    };
    check()?;
    if !limits.valid() || bytes.len() > limits.encoded_bytes || bounds.0 == 0 || bounds.1 == 0 {
        return Err(Error::Limit);
    }
    let format = match format {
        Format::Png => image::ImageFormat::Png,
        Format::Jpeg => image::ImageFormat::Jpeg,
    };
    let mut codec_limits = image::Limits::default();
    codec_limits.max_image_width = Some(limits.source_edge);
    codec_limits.max_image_height = Some(limits.source_edge);
    codec_limits.max_alloc = Some(limits.decoded_bytes);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(codec_limits);
    let mut decoder = reader.into_decoder().map_err(image_error)?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > limits.source_edge
        || height > limits.source_edge
        || u64::from(width) * u64::from(height) > limits.source_pixels
        || decoder.total_bytes() > limits.decoded_bytes
    {
        return Err(Error::Limit);
    }
    let orientation = decoder.orientation().map_err(image_error)?;
    check()?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(image_error)?;
    check()?;
    image.apply_orientation(orientation);
    let (original_width, original_height) = (image.width(), image.height());
    let scale = (bounds.0 as f64 / original_width as f64)
        .min(bounds.1 as f64 / original_height as f64)
        .min(1.)
        .min(
            (limits.output_pixels as f64 / (original_width as f64 * original_height as f64)).sqrt(),
        );
    let mut size = (
        (original_width as f64 * scale).floor().max(1.) as u32,
        (original_height as f64 * scale).floor().max(1.) as u32,
    );
    // Rounding a very narrow axis up to one pixel must not exceed the budget.
    size.0 = size.0.min(limits.output_pixels as u32);
    size.1 = size
        .1
        .min((limits.output_pixels / u64::from(size.0)) as u32);
    let image = if size != (original_width, original_height) {
        image.thumbnail_exact(size.0, size.1)
    } else {
        image
    };
    check()?;
    Ok(Raster {
        width: image.width(),
        height: image.height(),
        original_width,
        original_height,
        rgba: image.into_rgba8().into_raw(),
    })
}
fn image_error(error: image::ImageError) -> Error {
    if matches!(error, image::ImageError::Limits(_)) {
        Error::Limit
    } else {
        Error::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let image =
            image::RgbaImage::from_fn(37, 19, |x, y| image::Rgba([x as u8, y as u8, 92, 128]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
    #[test]
    fn preserves_pixels_and_applies_caller_budgets_before_decode() {
        let bytes = png();
        let cancel = AtomicBool::new(false);
        let limits = Limits::default();
        let result = decode(&bytes, Format::Png, (100, 100), limits, &cancel).unwrap();
        assert_eq!((result.width, result.height), (37, 19));
        assert_eq!(&result.rgba[..4], &[0, 0, 92, 128]);
        for limits in [
            Limits {
                encoded_bytes: 1,
                ..limits
            },
            Limits {
                source_pixels: 10,
                ..limits
            },
            Limits {
                decoded_bytes: 1,
                ..limits
            },
            Limits {
                output_pixels: 0,
                ..limits
            },
        ] {
            assert!(matches!(
                decode(&bytes, Format::Png, (100, 100), limits, &cancel),
                Err(Error::Limit)
            ));
        }
        for (width, height) in [(1, 200), (200, 1)] {
            let mut encoded = Cursor::new(Vec::new());
            image::RgbaImage::new(width, height)
                .write_to(&mut encoded, image::ImageFormat::Png)
                .unwrap();
            let result = decode(
                encoded.get_ref(),
                Format::Png,
                (1000, 1000),
                Limits {
                    output_pixels: 3,
                    ..limits
                },
                &cancel,
            )
            .unwrap();
            assert!(result.width > 0 && result.height > 0);
            assert!(u64::from(result.width) * u64::from(result.height) <= 3);
        }
        let result = decode(
            &bytes,
            Format::Png,
            (100, 100),
            Limits {
                output_pixels: 100,
                ..limits
            },
            &cancel,
        )
        .unwrap();
        assert!(result.width * result.height <= 100);
        assert!(matches!(
            decode(
                &bytes,
                Format::Png,
                (100, 100),
                limits,
                &AtomicBool::new(true)
            ),
            Err(Error::Cancelled)
        ));
    }
}
