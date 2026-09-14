use crate::node::RasterImageData;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::{io::Cursor, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct ImageDecodeLimits {
    pub max_source_dimension: u32,
    pub max_source_pixels: u64,
    /// JPEGs can be admitted at a larger source size because the decoder uses
    /// native IDCT scaling before allocating the retained pixel buffer.
    pub max_jpeg_source_dimension: u32,
    pub max_jpeg_source_pixels: u64,
    pub max_alloc: u64,
    pub target_dimension: u32,
}

impl ImageDecodeLimits {
    /// Bound decoding and retain intrinsic dimensions independently of pixels.
    /// JPEG uses native IDCT scaling; other formats are resized after their
    /// bounded decode. The first frame is used for animated formats.
    pub fn decode(self, bytes: &[u8]) -> Result<RasterImageData, String> {
        let reader = ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let format = reader.format();
        let (width, height) = reader.into_dimensions().map_err(|e| e.to_string())?;
        let (max_source_dimension, max_source_pixels) =
            if format == Some(ImageFormat::Jpeg) {
                (
                    self.max_jpeg_source_dimension,
                    self.max_jpeg_source_pixels,
                )
            } else {
                (self.max_source_dimension, self.max_source_pixels)
            };
        if width == 0
            || height == 0
            || width > max_source_dimension
            || height > max_source_dimension
            || u64::from(width) * u64::from(height) > max_source_pixels
            || self.target_dimension == 0
        {
            return Err("Image dimensions exceed the document decoder limits".into());
        }

        let target = self.target_dimension;
        let scaled_jpeg = if format == Some(ImageFormat::Jpeg) && width.max(height) > target {
            let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
            decoder.set_max_decoding_buffer_size(self.max_alloc.min(usize::MAX as u64) as usize);
            decoder.read_info().map_err(|e| e.to_string())?;
            let info = decoder.info().ok_or("Missing JPEG dimensions")?;
            if matches!(
                info.pixel_format,
                jpeg_decoder::PixelFormat::RGB24 | jpeg_decoder::PixelFormat::L8
            ) {
                let divisor = width
                    .max(height)
                    .div_ceil(target)
                    .next_power_of_two()
                    .min(8);
                let (w, h) = decoder
                    .scale(
                        width.div_ceil(divisor) as u16,
                        height.div_ceil(divisor) as u16,
                    )
                    .map_err(|e| e.to_string())?;
                let pixels = decoder.decode().map_err(|e| e.to_string())?;
                Some(match info.pixel_format {
                    jpeg_decoder::PixelFormat::RGB24 => DynamicImage::ImageRgb8(
                        image::RgbImage::from_raw(w.into(), h.into(), pixels)
                            .ok_or("Invalid JPEG pixels")?,
                    ),
                    _ => DynamicImage::ImageLuma8(
                        image::GrayImage::from_raw(w.into(), h.into(), pixels)
                            .ok_or("Invalid JPEG pixels")?,
                    ),
                })
            } else {
                None
            }
        } else {
            None
        };
        let decoded = match scaled_jpeg {
            Some(image) => image,
            None => {
                let mut reader = ImageReader::new(Cursor::new(bytes))
                    .with_guessed_format()
                    .map_err(|e| e.to_string())?;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(self.max_source_dimension);
                limits.max_image_height = Some(self.max_source_dimension);
                limits.max_alloc = Some(self.max_alloc);
                reader.limits(limits);
                reader.decode().map_err(|e| e.to_string())?
            }
        };
        let decoded = if decoded.width().max(decoded.height()) > target {
            decoded.thumbnail(target, target)
        } else {
            decoded
        };
        let pixel_width = decoded.width();
        let pixel_height = decoded.height();
        let mut image =
            RasterImageData::new(width, height, Arc::new(decoded.into_rgba8().into_raw()));
        image.pixel_width = pixel_width;
        image.pixel_height = pixel_height;
        Ok(image)
    }
}
