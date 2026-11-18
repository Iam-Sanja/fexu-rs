//! Bounded PNG inflate. [`crate::inspect_image`] does not inflate pixels.

use crate::{ImageInfo, ImageLimits, StorageError};

/// Result of [`validate_image`].
///
/// PNG pixels are reconstructed and then discarded. JPEG markers are checked
/// and the scan is not decoded. Neither variant is a malware scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageValidation {
    PngDecoded(ImageInfo),
    JpegContainer(ImageInfo),
}

/// PNG bytes are inflated under `limits`. JPEG bytes stay a container check.
pub fn validate_image(bytes: &[u8], limits: ImageLimits) -> Result<ImageValidation, StorageError> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes, limits).map(ImageValidation::PngDecoded)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        crate::inspect_image(bytes, limits).map(ImageValidation::JpegContainer)
    } else {
        Err(StorageError::InvalidImage)
    }
}

/// Inflates one 8-bit non-interlaced PNG and checks the reconstructed scanlines.
///
/// Gray, RGB and RGBA are accepted. Palette, interlaced and other bit depths
/// are rejected. The pixel buffer is dropped before this function returns.
/// A matching signature without an `IDAT` chunk fails here and can still pass
/// [`crate::inspect_image`].
pub fn decode_png(bytes: &[u8], limits: ImageLimits) -> Result<ImageInfo, StorageError> {
    if bytes.len() as u64 > limits.max_bytes || bytes.len() < 8 + 8 + 13 + 4 {
        return Err(if (bytes.len() as u64) > limits.max_bytes {
            StorageError::TooLarge
        } else {
            StorageError::InvalidImage
        });
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(StorageError::InvalidImage);
    }
    let mut offset = 8usize;
    let mut info = None;
    let mut idat = Vec::new();
    let mut saw_idat = false;
    let mut ended = false;
    while offset + 8 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[offset + 4..offset + 8]
            .try_into()
            .map_err(|_| StorageError::InvalidImage)?;
        let data_at = offset + 8;
        let Some(next) = data_at
            .checked_add(length)
            .and_then(|value| value.checked_add(4))
        else {
            return Err(StorageError::InvalidImage);
        };
        if next > bytes.len() {
            return Err(StorageError::InvalidImage);
        }
        let data = &bytes[data_at..data_at + length];
        let stored = u32::from_be_bytes(
            bytes[data_at + length..next]
                .try_into()
                .map_err(|_| StorageError::InvalidImage)?,
        );
        let mut hashed = Vec::with_capacity(4 + data.len());
        hashed.extend_from_slice(&kind);
        hashed.extend_from_slice(data);
        if crc32(&hashed) != stored {
            return Err(StorageError::InvalidImage);
        }
        if ended {
            return Err(StorageError::InvalidImage);
        }
        match &kind {
            b"IHDR" => {
                if info.is_some() || saw_idat || length != 13 {
                    return Err(StorageError::InvalidImage);
                }
                let width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                let height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                let bit_depth = data[8];
                let color = data[9];
                if bit_depth != 8 || data[10] != 0 || data[11] != 0 || data[12] != 0 {
                    return Err(StorageError::InvalidImage);
                }
                let channels = match color {
                    0 => 1,
                    2 => 3,
                    6 => 4,
                    _ => return Err(StorageError::InvalidImage),
                };
                check_dimensions(width, height, limits)?;
                info = Some((
                    ImageInfo {
                        format: "image/png",
                        width,
                        height,
                    },
                    channels,
                ));
            }
            b"IDAT" => {
                if info.is_none() {
                    return Err(StorageError::InvalidImage);
                }
                saw_idat = true;
                idat.extend_from_slice(data);
            }
            b"IEND" => {
                if length != 0 || !saw_idat {
                    return Err(StorageError::InvalidImage);
                }
                ended = true;
            }
            _ if kind[0].is_ascii_lowercase() => {
                if saw_idat && !ended {
                    return Err(StorageError::InvalidImage);
                }
            }
            _ => return Err(StorageError::InvalidImage),
        }
        offset = next;
    }
    if !ended {
        return Err(StorageError::InvalidImage);
    }
    let Some((info, channels)) = info else {
        return Err(StorageError::InvalidImage);
    };
    let stride = (info.width as usize)
        .checked_mul(channels)
        .ok_or(StorageError::InvalidImage)?;
    let row = stride.checked_add(1).ok_or(StorageError::InvalidImage)?;
    let expected = row
        .checked_mul(info.height as usize)
        .ok_or(StorageError::InvalidImage)?;
    if expected > MAX_INFLATE {
        return Err(StorageError::InvalidImage);
    }
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, expected)
        .map_err(|_| StorageError::InvalidImage)?;
    if inflated.len() != expected {
        return Err(StorageError::InvalidImage);
    }
    let mut pixels = inflated;
    unfilter(&mut pixels, info.height as usize, stride, channels)?;
    drop(pixels);
    Ok(info)
}

const MAX_INFLATE: usize = 32 * 1024 * 1024;

fn check_dimensions(width: u32, height: u32, limits: ImageLimits) -> Result<(), StorageError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > limits.max_width
        || height > limits.max_height
        || pixels > limits.max_pixels
    {
        Err(StorageError::InvalidImage)
    } else {
        Ok(())
    }
}

fn unfilter(
    raw: &mut [u8],
    height: usize,
    stride: usize,
    channels: usize,
) -> Result<(), StorageError> {
    let row_len = stride + 1;
    if raw.len() != height * row_len {
        return Err(StorageError::InvalidImage);
    }
    for row in 0..height {
        let start = row * row_len;
        let filter = raw[start];
        if filter > 4 {
            return Err(StorageError::InvalidImage);
        }
        for col in 0..stride {
            let index = start + 1 + col;
            let left = if col >= channels {
                raw[index - channels]
            } else {
                0
            };
            let up = if row > 0 { raw[index - row_len] } else { 0 };
            let upper_left = if row > 0 && col >= channels {
                raw[index - row_len - channels]
            } else {
                0
            };
            let value = raw[index];
            raw[index] = match filter {
                0 => value,
                1 => value.wrapping_add(left),
                2 => value.wrapping_add(up),
                3 => value.wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8),
                4 => value.wrapping_add(paeth(left, up, upper_left)),
                _ => return Err(StorageError::InvalidImage),
            };
        }
    }
    Ok(())
}

fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let left = i16::from(left);
    let up = i16::from(up);
    let upper_left = i16::from(upper_left);
    let estimate = left + up - upper_left;
    let left_distance = (estimate - left).abs();
    let up_distance = (estimate - up).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left as u8
    } else if up_distance <= upper_left_distance {
        up as u8
    } else {
        upper_left as u8
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
pub(crate) fn sample_png(width: u32, height: u32) -> Vec<u8> {
    let mut row = vec![0];
    for _ in 0..width {
        row.extend_from_slice(&[12, 34, 56]);
    }
    let mut raw = Vec::with_capacity(row.len() * height as usize);
    for _ in 0..height {
        raw.extend_from_slice(&row);
    }
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &ihdr));
    bytes.extend(chunk(b"IDAT", &compressed));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

#[cfg(test)]
fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    let mut hashed = kind.to_vec();
    hashed.extend_from_slice(data);
    bytes.extend_from_slice(&crc32(&hashed).to_be_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::{decode_png, sample_png, validate_image, ImageValidation};
    use crate::{inspect_image, ImageLimits, StorageError};

    fn limits() -> ImageLimits {
        ImageLimits {
            max_bytes: 65_536,
            max_width: 8,
            max_height: 8,
            max_pixels: 64,
        }
    }

    #[test]
    fn png_pixels_are_inflated_and_a_header_prefix_is_not() {
        let image = sample_png(1, 1);
        let decoded = decode_png(&image, limits()).expect("png");
        assert_eq!(decoded.format, "image/png");
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert!(matches!(
            validate_image(&image, limits()),
            Ok(ImageValidation::PngDecoded(_))
        ));
        let header = {
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend_from_slice(&13u32.to_be_bytes());
            bytes.extend_from_slice(b"IHDR");
            bytes.extend_from_slice(&1u32.to_be_bytes());
            bytes.extend_from_slice(&1u32.to_be_bytes());
            bytes.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(b"IEND");
            bytes.extend_from_slice(&[0, 0, 0, 0]);
            bytes
        };
        assert!(inspect_image(&header, limits()).is_ok());
        assert_eq!(
            decode_png(&header, limits()),
            Err(StorageError::InvalidImage)
        );
        let mut truncated = image.clone();
        truncated.truncate(image.len().saturating_sub(8));
        assert_eq!(
            decode_png(&truncated, limits()),
            Err(StorageError::InvalidImage)
        );
        assert_eq!(
            decode_png(&sample_png(32, 1), limits()),
            Err(StorageError::InvalidImage)
        );
    }

    #[test]
    fn jpeg_validation_does_not_claim_a_pixel_decode() {
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x00,
            0x00, 0xFF, 0xD9,
        ];
        assert_eq!(
            validate_image(&jpeg, limits()),
            Ok(ImageValidation::JpegContainer(crate::ImageInfo {
                format: "image/jpeg",
                width: 1,
                height: 1,
            }))
        );
        assert_eq!(
            validate_image(&jpeg[..jpeg.len() - 2], limits()),
            Err(StorageError::InvalidImage)
        );
    }
}
