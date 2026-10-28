use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};

use image::codecs::jpeg::JpegEncoder;
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use super::{MAX_DECODE_ALLOC, MAX_PIXELS_SIDE, MAX_SOURCE_BYTES, THUMB_SIZE};

const JPEG_QUALITY: u8 = 80;

/// A re-encoded preview, never the user's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("unreadable image: {0}")]
    Io(#[from] std::io::Error),
    #[error("unknown image format")]
    UnknownFormat,
    #[error("image too large to decode")]
    TooLarge,
    #[error(transparent)]
    Image(#[from] image::ImageError),
}

/// Decodes `file` within the limits, shrinks it to fit [`THUMB_SIZE`] and applies its EXIF
/// orientation. Images with an alpha channel become PNG, the others JPEG. At most
/// [`MAX_SOURCE_BYTES`] are read, even if the file grew since it was checked.
pub fn render(file: File) -> Result<Thumbnail, RenderError> {
    render_capped(file, MAX_SOURCE_BYTES)
}

fn render_capped(file: File, max_bytes: u64) -> Result<Thumbnail, RenderError> {
    let mut source = BufReader::new(Capped {
        inner: file,
        position: 0,
        limit: max_bytes,
    });
    let source_len = source.seek(SeekFrom::End(0))?;
    source.rewind()?;
    let reader = ImageReader::new(source).with_guessed_format()?;
    let Some(format) = reader.format().filter(|format| {
        matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Gif | ImageFormat::WebP
        )
    }) else {
        return Err(RenderError::UnknownFormat);
    };
    let mut source = reader.into_inner();
    let progressive = format == ImageFormat::Jpeg && is_progressive_jpeg(&mut source)?;
    source.rewind()?;
    let mut reader = ImageReader::with_format(source, format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PIXELS_SIDE);
    limits.max_image_height = Some(MAX_PIXELS_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);

    let mut decoder = reader.into_decoder()?;
    if decode_cost(format, progressive, decoder.total_bytes(), source_len) > MAX_DECODE_ALLOC {
        return Err(RenderError::TooLarge);
    }
    // A broken EXIF block only costs the orientation.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder)?;
    // Shrinking first keeps a rotation from copying the full-size image. The bounds are square,
    // so they fit both before and after a 90° turn.
    let mut image = if image.width() > THUMB_SIZE || image.height() > THUMB_SIZE {
        image.thumbnail(THUMB_SIZE, THUMB_SIZE)
    } else {
        image
    };
    image.apply_orientation(orientation);

    let mut bytes = Vec::new();
    if image.color().has_alpha() {
        DynamicImage::ImageRgba8(image.into_rgba8())
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
        Ok(Thumbnail {
            bytes,
            content_type: "image/png",
        })
    } else {
        let rgb = image.into_rgb8();
        JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY).encode_image(&rgb)?;
        Ok(Thumbnail {
            bytes,
            content_type: "image/jpeg",
        })
    }
}

/// The memory a decode takes at its peak, as measured: the source, read whole, plus the decoded
/// image times what the decoder keeps besides. A progressive JPEG keeps the coefficients of
/// every component until its last scan (up to twice the output), WebP its YUV planes or ARGB
/// words, GIF a frame besides the canvas. The image crate's `max_alloc` does not count these.
fn decode_cost(format: ImageFormat, progressive: bool, decoded: u64, source: u64) -> u64 {
    let factor = match format {
        ImageFormat::Jpeg if progressive => 3,
        ImageFormat::WebP => 3,
        ImageFormat::Gif => 2,
        _ => 1,
    };
    decoded.saturating_mul(factor).saturating_add(source)
}

/// Whether the first frame header (SOF) of a JPEG is progressive. Reaching the scan data first,
/// or a header that can't be followed, counts as progressive: the costlier guess.
fn is_progressive_jpeg(reader: &mut (impl Read + Seek)) -> io::Result<bool> {
    if (byte(reader)?, byte(reader)?) != (0xFF, 0xD8) {
        return Ok(true);
    }
    loop {
        if byte(reader)? != 0xFF {
            return Ok(true);
        }
        let mut marker = byte(reader)?;
        while marker == 0xFF {
            marker = byte(reader)?;
        }
        match marker {
            0xC2 | 0xC6 | 0xCA | 0xCE | 0xDA => return Ok(true),
            0xC0 | 0xC1 | 0xC3 | 0xC5 | 0xC7 | 0xC9 | 0xCB | 0xCD | 0xCF => return Ok(false),
            0x01 | 0xD0..=0xD7 => {}
            _ => {
                let length = u16::from_be_bytes([byte(reader)?, byte(reader)?]);
                if length < 2 {
                    return Ok(true);
                }
                reader.seek(SeekFrom::Current(i64::from(length) - 2))?;
            }
        }
    }
}

fn byte(reader: &mut impl Read) -> io::Result<u8> {
    let mut byte = [0];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
}

/// A seekable reader that ends at `limit`, as `Read::take` would if it could seek.
struct Capped<R> {
    inner: R,
    position: u64,
    limit: u64,
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.limit.saturating_sub(self.position);
        let len = usize::try_from(left).map_or(buf.len(), |left| left.min(buf.len()));
        let read = self.inner.read(&mut buf[..len])?;
        self.position += read as u64;
        Ok(read)
    }
}

/// The end is where reads stop: the cap, or the real end of a shorter source.
impl<R: Seek> Seek for Capped<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::End(offset) => {
                let end = self.inner.seek(SeekFrom::End(0))?.min(self.limit);
                let target = end.checked_add_signed(offset).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "seek before the start")
                })?;
                SeekFrom::Start(target)
            }
            pos => pos,
        };
        self.position = self.inner.seek(pos)?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};
    use std::time::{Duration, Instant};

    use image::{GenericImageView, ImageEncoder, Rgb, RgbImage, Rgba, RgbaImage};

    use super::*;

    fn file_with(bytes: &[u8]) -> File {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(bytes).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            Rgb([x.to_le_bytes()[0], y.to_le_bytes()[0], 128])
        });
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 90)
            .write_image(
                image.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        bytes
    }

    fn decoded(thumbnail: &Thumbnail) -> DynamicImage {
        image::load_from_memory(&thumbnail.bytes).unwrap()
    }

    #[test]
    fn renders_jpeg_within_bounds() {
        let thumbnail = render(file_with(&jpeg(2000, 1000))).unwrap();
        assert_eq!(thumbnail.content_type, "image/jpeg");
        assert_eq!(
            image::guess_format(&thumbnail.bytes).unwrap(),
            ImageFormat::Jpeg
        );
        assert_eq!(decoded(&thumbnail).dimensions(), (320, 160));
    }

    #[test]
    fn keeps_alpha_as_png() {
        let image = RgbaImage::from_pixel(64, 32, Rgba([10, 20, 30, 100]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let thumbnail = render(file_with(&bytes)).unwrap();
        assert_eq!(thumbnail.content_type, "image/png");
        let decoded = decoded(&thumbnail);
        assert!(decoded.color().has_alpha());
        assert_eq!(decoded.dimensions(), (64, 32));
    }

    #[test]
    fn animated_gif_gives_its_first_frame() {
        let frame = |color: [u8; 4]| image::Frame::new(RgbaImage::from_pixel(30, 10, Rgba(color)));
        let mut bytes = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut bytes)
            .encode_frames([frame([255, 0, 0, 255]), frame([0, 0, 255, 255])])
            .unwrap();
        let thumbnail = render(file_with(&bytes)).unwrap();
        let decoded = decoded(&thumbnail);
        assert_eq!(decoded.dimensions(), (30, 10));
        let pixel = decoded.to_rgba8().get_pixel(15, 5).0;
        assert!(pixel[0] > 200 && pixel[2] < 50, "{pixel:?}");
    }

    #[test]
    fn small_images_are_not_enlarged() {
        let thumbnail = render(file_with(&jpeg(40, 20))).unwrap();
        assert_eq!(decoded(&thumbnail).dimensions(), (40, 20));
    }

    /// An APP1 segment holding a big-endian TIFF with one IFD entry: Orientation = 6.
    fn exif_orientation_6() -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
        tiff.extend_from_slice(&[0, 1]);
        tiff.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        tiff.extend_from_slice(&[0, 0, 0, 0]);
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        let length = u16::try_from(payload.len() + 2).unwrap();
        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&length.to_be_bytes());
        segment.extend_from_slice(&payload);
        segment
    }

    #[test]
    fn thumbnail_applies_exif_orientation() {
        let source = jpeg(40, 20);
        let mut bytes = source[..2].to_vec();
        bytes.extend_from_slice(&exif_orientation_6());
        bytes.extend_from_slice(&source[2..]);
        let thumbnail = render(file_with(&bytes)).unwrap();
        assert_eq!(decoded(&thumbnail).dimensions(), (20, 40));
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn thumbnail_refuses_oversized_image() {
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend_from_slice(&30_000u32.to_be_bytes());
        ihdr.extend_from_slice(&30_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(&ihdr);
        bytes.extend_from_slice(&crc32(&ihdr).to_be_bytes());

        let started = Instant::now();
        let err = render(file_with(&bytes)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            matches!(&err, RenderError::Image(image::ImageError::Limits(_))),
            "{err:?}"
        );
    }

    /// `source` with its baseline frame header turned progressive and claiming `width` x
    /// `height`: the header is all a decoder reads before the cost is checked.
    fn as_progressive(source: &[u8], width: u16, height: u16) -> Vec<u8> {
        let mut bytes = source.to_vec();
        let sof = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        bytes[sof + 1] = 0xC2;
        bytes[sof + 5..sof + 7].copy_from_slice(&height.to_be_bytes());
        bytes[sof + 7..sof + 9].copy_from_slice(&width.to_be_bytes());
        bytes
    }

    #[test]
    fn progressive_jpeg_is_counted_at_its_real_cost() {
        // 536,838,144 decoded bytes, within 512 MiB, but a progressive decode keeps about twice
        // that again in coefficients.
        let bytes = as_progressive(&jpeg(16, 16), 16_384, 10_922);
        let started = Instant::now();
        let err = render(file_with(&bytes)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(matches!(err, RenderError::TooLarge), "{err:?}");
    }

    #[test]
    fn finds_whether_a_jpeg_is_progressive() {
        let progressive = |bytes: &[u8]| is_progressive_jpeg(&mut Cursor::new(bytes)).unwrap();
        let source = jpeg(40, 20);
        assert!(!progressive(&source));
        let mut with_exif = source[..2].to_vec();
        with_exif.extend_from_slice(&exif_orientation_6());
        with_exif.extend_from_slice(&source[2..]);
        assert!(!progressive(&with_exif));
        assert!(progressive(&as_progressive(&source, 40, 20)));
        assert!(progressive(b"\xFF\xD8\xFF\xE0\x00\x01"));
        assert!(progressive(b"\xFF\xD8\x00"));
        assert!(is_progressive_jpeg(&mut Cursor::new(b"\xFF\xD8\xFF\xE0\x00\x10")).is_err());
    }

    #[test]
    fn decode_cost_counts_what_the_decoders_keep() {
        let mib = 1024 * 1024;
        assert_eq!(decode_cost(ImageFormat::Png, false, 100 * mib, 0), 100 * mib);
        assert_eq!(decode_cost(ImageFormat::Jpeg, false, 100 * mib, mib), 101 * mib);
        assert_eq!(decode_cost(ImageFormat::Jpeg, true, 100 * mib, mib), 301 * mib);
        assert_eq!(decode_cost(ImageFormat::WebP, false, 100 * mib, 0), 300 * mib);
        assert_eq!(decode_cost(ImageFormat::Gif, false, 100 * mib, 0), 200 * mib);
        assert_eq!(decode_cost(ImageFormat::Png, false, u64::MAX, 1), u64::MAX);
    }

    #[test]
    fn reads_stop_at_the_cap() {
        let mut capped = Capped {
            inner: Cursor::new(vec![7u8; 100]),
            position: 0,
            limit: 10,
        };
        let mut all = Vec::new();
        capped.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 10);
        capped.seek(SeekFrom::Start(4)).unwrap();
        all.clear();
        capped.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 6);
        capped.seek(SeekFrom::Start(50)).unwrap();
        assert_eq!(capped.read(&mut [0; 8]).unwrap(), 0);
    }

    #[test]
    fn seeks_from_the_end_of_the_capped_source() {
        let mut capped = Capped {
            inner: Cursor::new((0..100u8).collect::<Vec<_>>()),
            position: 0,
            limit: 10,
        };
        assert_eq!(capped.seek(SeekFrom::End(-4)).unwrap(), 6);
        let mut tail = Vec::new();
        capped.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, [6, 7, 8, 9]);
        assert!(capped.seek(SeekFrom::End(-11)).is_err());

        let mut short = Capped {
            inner: Cursor::new(vec![1u8; 5]),
            position: 0,
            limit: 10,
        };
        assert_eq!(short.seek(SeekFrom::End(0)).unwrap(), 5);
    }

    #[test]
    fn file_grown_past_the_cap_is_cut() {
        let source = jpeg(400, 200);
        let scan = source.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert!(render_capped(file_with(&source), source.len() as u64).is_ok());
        assert!(render_capped(file_with(&source), scan as u64).is_err());
    }

    #[test]
    fn rotated_large_image_is_shrunk_then_turned() {
        let source = jpeg(2000, 1000);
        let mut bytes = source[..2].to_vec();
        bytes.extend_from_slice(&exif_orientation_6());
        bytes.extend_from_slice(&source[2..]);
        let thumbnail = render(file_with(&bytes)).unwrap();
        assert_eq!(decoded(&thumbnail).dimensions(), (160, 320));
    }

    #[test]
    fn thumbnail_of_corrupt_file() {
        // The lenient decoder renders a JPEG cut inside its scan data as a partial image, as
        // browsers do; one cut before the scan starts has nothing to show.
        let source = jpeg(400, 200);
        let scan = source.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        assert!(render(file_with(&source[..scan])).is_err());
        assert!(render(file_with(b"not an image, just text")).is_err());
        assert!(render(file_with(b"")).is_err());
    }
}
