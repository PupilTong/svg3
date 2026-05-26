//! Decoding embedded PNG data URLs used by `<feImage>` filter primitives,
//! and uploading the result as a GPU texture.

use std::io::Cursor;

use base64::engine::general_purpose;
use base64::Engine as _;
use thiserror::Error;

/// A decoded CPU-side image ready for upload.
#[derive(Debug)]
pub(super) struct DecodedImage {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba: Vec<u8>,
}

/// A decoded `<feImage>` uploaded to GPU memory.
#[derive(Debug)]
pub(super) struct GpuImage {
    pub(super) _texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn decode_image_href(href: &str) -> Result<DecodedImage, ImageDecodeError> {
    let png_bytes = decode_png_data_url(href)?;
    decode_png(&png_bytes)
}

fn decode_png_data_url(href: &str) -> Result<Vec<u8>, ImageDecodeError> {
    let href = href.trim();
    let data_url = href
        .strip_prefix("data:")
        .ok_or(ImageDecodeError::UnsupportedHref)?;
    let (metadata, data) = data_url
        .split_once(',')
        .ok_or(ImageDecodeError::UnsupportedHref)?;
    let mut parts = metadata.split(';');
    let media_type = parts.next().unwrap_or_default();
    if !media_type.is_empty() && !media_type.eq_ignore_ascii_case("image/png") {
        return Err(ImageDecodeError::UnsupportedMediaType(
            media_type.to_owned(),
        ));
    }
    if !parts.any(|part| part.eq_ignore_ascii_case("base64")) {
        return Err(ImageDecodeError::DataUrlNotBase64);
    }
    Ok(general_purpose::STANDARD.decode(data.trim())?)
}

fn decode_png(bytes: &[u8]) -> Result<DecodedImage, ImageDecodeError> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let output_size = reader
        .output_buffer_size()
        .ok_or(ImageDecodeError::ImageTooLarge)?;
    let mut buffer = vec![0; output_size];
    let info = reader.next_frame(&mut buffer)?;
    if info.width == 0 || info.height == 0 {
        return Err(ImageDecodeError::EmptyImage);
    }
    let (color_type, bit_depth) = reader.output_color_type();
    let rgba = png_output_to_rgba(
        &buffer[..info.buffer_size()],
        info.width,
        info.height,
        color_type,
        bit_depth,
    )?;
    Ok(DecodedImage {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn png_output_to_rgba(
    bytes: &[u8],
    width: u32,
    height: u32,
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
) -> Result<Vec<u8>, ImageDecodeError> {
    if bit_depth != png::BitDepth::Eight {
        return Err(ImageDecodeError::UnsupportedPngColor {
            color_type,
            bit_depth,
        });
    }
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageDecodeError::ImageTooLarge)?;
    match color_type {
        png::ColorType::Rgba => {
            if bytes.len() != pixel_count * 4 {
                return Err(ImageDecodeError::ImageTooLarge);
            }
            Ok(bytes.to_vec())
        }
        png::ColorType::Rgb => {
            if bytes.len() != pixel_count * 3 {
                return Err(ImageDecodeError::ImageTooLarge);
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for rgb in bytes.chunks_exact(3) {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            Ok(rgba)
        }
        png::ColorType::GrayscaleAlpha => {
            if bytes.len() != pixel_count * 2 {
                return Err(ImageDecodeError::ImageTooLarge);
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for gray_alpha in bytes.chunks_exact(2) {
                rgba.extend_from_slice(&[
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[0],
                    gray_alpha[1],
                ]);
            }
            Ok(rgba)
        }
        png::ColorType::Grayscale => {
            if bytes.len() != pixel_count {
                return Err(ImageDecodeError::ImageTooLarge);
            }
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for gray in bytes {
                rgba.extend_from_slice(&[*gray, *gray, *gray, 255]);
            }
            Ok(rgba)
        }
        png::ColorType::Indexed => Err(ImageDecodeError::UnsupportedPngColor {
            color_type,
            bit_depth,
        }),
    }
}

/// Errors that can occur while decoding an embedded `<feImage>`.
#[derive(Debug, Error)]
pub(super) enum ImageDecodeError {
    /// The renderer currently supports embedded PNG data URLs only.
    #[error("unsupported href; only PNG data URLs are supported")]
    UnsupportedHref,
    /// The data URL's media type is not `image/png`.
    #[error("unsupported data URL media type `{0}`; only image/png is supported")]
    UnsupportedMediaType(String),
    /// The data URL is not base64-encoded.
    #[error("PNG data URL must be base64 encoded")]
    DataUrlNotBase64,
    /// Base64 payload decoding failed.
    #[error("base64 decoding failed: {0}")]
    Base64(#[from] base64::DecodeError),
    /// PNG decoding failed.
    #[error("PNG decoding failed: {0}")]
    Png(#[from] png::DecodingError),
    /// The decoded image dimensions or buffer length are too large.
    #[error("decoded PNG image is too large")]
    ImageTooLarge,
    /// The decoded image has no pixels.
    #[error("decoded PNG image is empty")]
    EmptyImage,
    /// The PNG output format is not one this renderer can upload as RGBA8.
    #[error("unsupported PNG output color type {color_type:?} at {bit_depth:?}")]
    UnsupportedPngColor {
        /// Output colour type after png decoder transformations.
        color_type: png::ColorType,
        /// Output bit depth after png decoder transformations.
        bit_depth: png::BitDepth,
    },
}
