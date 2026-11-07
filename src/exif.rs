//! EXIF orientation 1–8 (PRD §5.2.3) and JFIF density (PRD §5.2.9).
//!
//! The orientation is read straight out of the JPEG APP1 segment: TIFF header
//! (`II`/`MM`), IFD0, tag 0x0112. No EXIF crate, because the only thing the product
//! needs is one `u16`, and a hand-rolled reader has no metadata surface to leak.

const TAG_ORIENTATION: u16 = 0x0112;

/// Read the orientation tag from a JPEG's APP1/Exif segment. Anything malformed,
/// missing or outside 1–8 is treated as 1 (upright), matching the C# codec.
pub(crate) fn orientation(jpeg: &[u8]) -> u8 {
    match find_exif_tiff(jpeg) {
        Some(tiff) => parse_orientation(tiff).unwrap_or(1),
        None => 1,
    }
}

/// Locate the TIFF block inside `APP1 "Exif\0\0"`, returning the slice that starts
/// at the TIFF header.
fn find_exif_tiff(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.len() < 4 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 3 < jpeg.len() {
        if jpeg[i] != 0xFF {
            return None; // left the marker stream
        }
        let mut marker_at = i;
        while marker_at < jpeg.len() && jpeg[marker_at] == 0xFF {
            marker_at += 1;
        }
        if marker_at >= jpeg.len() {
            return None;
        }
        let marker = jpeg[marker_at];
        i = marker_at + 1;

        // Standalone markers carry no length.
        if marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            return None; // start of scan / end of image: no metadata past here
        }
        if i + 1 >= jpeg.len() {
            return None;
        }
        let len = u16::from_be_bytes([jpeg[i], jpeg[i + 1]]) as usize;
        if len < 2 {
            return None;
        }
        let payload_start = i + 2;
        let payload_end = i + len;
        if payload_end > jpeg.len() {
            return None;
        }
        if marker == 0xE1 && payload_end - payload_start >= 6 {
            let payload = &jpeg[payload_start..payload_end];
            if payload.starts_with(b"Exif\0\0") {
                return Some(&payload[6..]);
            }
        }
        i = payload_end;
    }
    None
}

fn parse_orientation(tiff: &[u8]) -> Option<u8> {
    if tiff.len() < 8 {
        return None;
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let magic = read_u16(tiff, 2, little)?;
    if magic != 42 {
        return None;
    }
    let ifd0 = read_u32(tiff, 4, little)? as usize;
    if ifd0 < 8 || ifd0 + 2 > tiff.len() {
        return None;
    }
    let count = read_u16(tiff, ifd0, little)? as usize;
    // Each entry is 12 bytes; bound the loop by what actually fits.
    let available = (tiff.len().saturating_sub(ifd0 + 2)) / 12;
    for index in 0..count.min(available) {
        let entry = ifd0 + 2 + index * 12;
        let tag = read_u16(tiff, entry, little)?;
        if tag != TAG_ORIENTATION {
            continue;
        }
        let field_type = read_u16(tiff, entry + 2, little)?;
        let count = read_u32(tiff, entry + 4, little)?;
        if count == 0 || field_type != 3 {
            return None; // SHORT is the only type EXIF allows here
        }
        let value = read_u16(tiff, entry + 8, little)?;
        return match value {
            1..=8 => Some(value as u8),
            _ => None,
        };
    }
    None
}

fn read_u16(buf: &[u8], at: usize, little: bool) -> Option<u16> {
    let bytes = [*buf.get(at)?, *buf.get(at + 1)?];
    Some(if little {
        u16::from_le_bytes(bytes)
    } else {
        u16::from_be_bytes(bytes)
    })
}

fn read_u32(buf: &[u8], at: usize, little: bool) -> Option<u32> {
    let bytes = [
        *buf.get(at)?,
        *buf.get(at + 1)?,
        *buf.get(at + 2)?,
        *buf.get(at + 3)?,
    ];
    Some(if little {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    })
}

/// JFIF APP0 density in dots per inch (PRD §5.2.9). `None` when absent, unitless
/// (aspect-ratio only) or nonsensical.
pub(crate) fn jfif_density(jpeg: &[u8]) -> Option<(u16, u16)> {
    if jpeg.len() < 18 || jpeg[0] != 0xFF || jpeg[1] != 0xD8 {
        return None;
    }
    if jpeg[2] != 0xFF || jpeg[3] != 0xE0 || &jpeg[6..11] != b"JFIF\0" {
        return None;
    }
    let len = u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize;
    if len < 16 || 4 + len > jpeg.len() {
        return None;
    }
    let units = jpeg[13];
    let x = u16::from_be_bytes([jpeg[14], jpeg[15]]);
    let y = u16::from_be_bytes([jpeg[16], jpeg[17]]);
    match units {
        1 => Some((x, y)),
        2 => Some((to_inch(x), to_inch(y))),
        _ => None,
    }
}

fn to_inch(per_cm: u16) -> u16 {
    ((f64::from(per_cm) * 2.54).round() as u16).max(1)
}

/// Preserve the source density when it is in [1,1200], else 96 (PRD §5.2.9).
pub(crate) fn choose_dpi(density: Option<(u16, u16)>) -> u16 {
    let Some((x, y)) = density else { return 96 };
    if (1..=1200).contains(&x) {
        return x;
    }
    if (1..=1200).contains(&y) {
        return y;
    }
    96
}

/// Bake orientation 1–8 into the pixels so the encoded file needs no tag.
/// Mapping is identical to the C# reference (`ExifOrientation.Apply`):
///
/// | value | transform                    |
/// |-------|------------------------------|
/// | 2     | flip horizontal              |
/// | 3     | rotate 180                   |
/// | 4     | flip vertical                |
/// | 5     | transpose (rotate 90 + flipH)|
/// | 6     | rotate 90 clockwise          |
/// | 7     | rotate 90 + flipV            |
/// | 8     | rotate 270 (90 anticlockwise)|
pub(crate) fn apply(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    orientation: u8,
) -> (Vec<u8>, u32, u32) {
    use image::imageops;
    use image::{ImageBuffer, Rgb};

    if !(2..=8).contains(&orientation) {
        return (pixels, width, height);
    }
    let Some(img) = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width, height, pixels) else {
        return (Vec::new(), 0, 0);
    };

    let out = match orientation {
        2 => imageops::flip_horizontal(&img),
        3 => imageops::rotate180(&img),
        4 => imageops::flip_vertical(&img),
        5 => imageops::flip_horizontal(&imageops::rotate90(&img)),
        6 => imageops::rotate90(&img),
        7 => imageops::flip_vertical(&imageops::rotate90(&img)),
        _ => imageops::rotate270(&img),
    };

    let (w, h) = (out.width(), out.height());
    (out.into_raw(), w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{encode_jpeg, noise_rgb, with_exif_orientation};

    fn sample_jpeg() -> Vec<u8> {
        let px = noise_rgb(16, 12, 42);
        encode_jpeg(&px, 16, 12, 80)
    }

    #[test]
    fn plain_jpeg_has_no_orientation() {
        assert_eq!(orientation(&sample_jpeg()), 1);
    }

    #[test]
    fn reads_big_endian_orientation_6() {
        let jpeg = with_exif_orientation(&sample_jpeg(), 6, false);
        assert_eq!(orientation(&jpeg), 6);
    }

    #[test]
    fn reads_little_endian_orientation_3() {
        let jpeg = with_exif_orientation(&sample_jpeg(), 3, true);
        assert_eq!(orientation(&jpeg), 3);
    }

    #[test]
    fn reads_every_valid_value() {
        for value in 1..=8u16 {
            let jpeg = with_exif_orientation(&sample_jpeg(), value, true);
            assert_eq!(orientation(&jpeg), value as u8, "value {value}");
        }
    }

    #[test]
    fn out_of_range_value_is_upright() {
        let jpeg = with_exif_orientation(&sample_jpeg(), 9, true);
        assert_eq!(orientation(&jpeg), 1);
        let jpeg = with_exif_orientation(&sample_jpeg(), 0, false);
        assert_eq!(orientation(&jpeg), 1);
    }

    #[test]
    fn garbage_app1_is_upright() {
        let mut jpeg = sample_jpeg();
        // APP1 whose payload is not "Exif\0\0"
        jpeg.splice(2..2, [0xFF, 0xE1, 0x00, 0x08, b'X', b'X', b'X', b'X', 0, 0]);
        assert_eq!(orientation(&jpeg), 1);
    }

    #[test]
    fn truncated_segment_does_not_panic() {
        let jpeg = with_exif_orientation(&sample_jpeg(), 6, true);
        for cut in 0..jpeg.len().min(40) {
            let _ = orientation(&jpeg[..cut]);
        }
        assert_eq!(orientation(&[]), 1);
        assert_eq!(orientation(&[0xFF]), 1);
    }

    #[test]
    fn bad_tiff_magic_is_upright() {
        let mut jpeg = sample_jpeg();
        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(b"II");
        payload.extend_from_slice(&41u16.to_le_bytes()); // not 42
        payload.extend_from_slice(&8u32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]);
        out.push(0xFF);
        out.push(0xE1);
        out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[2..]);
        jpeg = out;
        assert_eq!(orientation(&jpeg), 1);
    }

    // --- pixel transforms -------------------------------------------------

    fn marker_image() -> (Vec<u8>, u32, u32) {
        // 3x2, each pixel a distinct grey so orientation is unambiguous.
        let px = vec![
            10u8, 10, 10, 20, 20, 20, 30, 30, 30, //
            40, 40, 40, 50, 50, 50, 60, 60, 60,
        ];
        (px, 3, 2)
    }

    #[test]
    fn identity_keeps_pixels() {
        let (px, w, h) = marker_image();
        let (out, ow, oh) = apply(px.clone(), w, h, 1);
        assert_eq!((ow, oh), (3, 2));
        assert_eq!(out, px);
    }

    #[test]
    fn orientation_3_is_rotate_180() {
        let (px, w, h) = marker_image();
        let (out, ow, oh) = apply(px, w, h, 3);
        assert_eq!((ow, oh), (3, 2));
        assert_eq!(out[..3], [60, 60, 60]);
        assert_eq!(out[15..], [10, 10, 10]);
    }

    #[test]
    fn orientation_6_swaps_dimensions_and_rotates_clockwise() {
        let (px, w, h) = marker_image();
        let (out, ow, oh) = apply(px, w, h, 6);
        assert_eq!((ow, oh), (2, 3));
        // rotate 90 CW: first output row is the first source column, bottom-up
        assert_eq!(out[..3], [40, 40, 40]);
        assert_eq!(out[3..6], [10, 10, 10]);
    }

    #[test]
    fn orientation_8_swaps_dimensions_and_rotates_anticlockwise() {
        let (px, w, h) = marker_image();
        let (out, ow, oh) = apply(px, w, h, 8);
        assert_eq!((ow, oh), (2, 3));
        assert_eq!(out[..3], [30, 30, 30]);
    }

    #[test]
    fn orientation_2_flips_horizontally() {
        let (px, w, h) = marker_image();
        let (out, _, _) = apply(px, w, h, 2);
        assert_eq!(out[..3], [30, 30, 30]);
        assert_eq!(out[15..], [40, 40, 40]);
    }

    #[test]
    fn orientation_5_and_7_are_transposes() {
        let (px, w, h) = marker_image();
        let (five, w5, h5) = apply(px.clone(), w, h, 5);
        let (seven, w7, h7) = apply(px, w, h, 7);
        assert_eq!((w5, h5), (2, 3));
        assert_eq!((w7, h7), (2, 3));
        // 5 = transpose: out(x,y) = src(y,x)
        assert_eq!(five[..3], [10, 10, 10]);
        assert_eq!(five[3..6], [40, 40, 40]);
        // 7 = anti-transpose: out(x,y) = src(w-1-y, h-1-x)
        assert_eq!(seven[..3], [60, 60, 60]);
    }

    // --- density ----------------------------------------------------------

    #[test]
    fn density_is_read_from_the_jfif_app0() {
        let mut jpeg = sample_jpeg();
        // image's encoder writes a JFIF APP0; stamp 300 dpi into it.
        assert_eq!(&jpeg[6..11], b"JFIF\0", "encoder must emit JFIF");
        jpeg[13] = 1;
        jpeg[14..16].copy_from_slice(&300u16.to_be_bytes());
        jpeg[16..18].copy_from_slice(&300u16.to_be_bytes());
        assert_eq!(jfif_density(&jpeg), Some((300, 300)));
        assert_eq!(choose_dpi(jfif_density(&jpeg)), 300);
    }

    #[test]
    fn missing_density_defaults_to_96() {
        assert_eq!(choose_dpi(None), 96);
        assert_eq!(choose_dpi(Some((0, 0))), 96);
        assert_eq!(choose_dpi(Some((5000, 5000))), 96);
        assert_eq!(choose_dpi(Some((0, 150))), 150);
    }
}
