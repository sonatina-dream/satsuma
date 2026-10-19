//! Lossless metadata removal for JPEG, PNG, WebP and GIF. Pixels are never re-encoded.
//!
//! JPEG: drops APP1 (EXIF/XMP), APP13 (IPTC), comments and other vendor APP segments, keeping
//! JFIF, ICC (APP2) and Adobe (APP14). If the photo had a non-default EXIF orientation, a
//! minimal EXIF block holding only the orientation is written so the photo still shows upright.

use crate::{Error, Result};
use satsuma_core::Format;

pub fn strip(format: Format, data: &[u8]) -> Result<Vec<u8>> {
    match format {
        Format::Jpg => strip_jpeg(data),
        Format::Png => strip_png(data),
        Format::Webp => strip_webp(data),
        Format::Gif => strip_gif(data),
        other => Err(Error::msg(format!(
            "Metadata removal for {} is not supported.",
            other.label()
        ))),
    }
}

fn corrupt(what: &str) -> Error {
    Error::msg(format!("The {what} file looks damaged."))
}

pub fn strip_jpeg(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(corrupt("JPEG"));
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut orientation: Option<u16> = None;
    let mut wrote_orientation = false;
    let mut pos = 2;
    while pos < data.len() {
        if data[pos] != 0xFF {
            return Err(corrupt("JPEG"));
        }
        // Skip fill bytes.
        let mut m = pos + 1;
        while m < data.len() && data[m] == 0xFF {
            m += 1;
        }
        if m >= data.len() {
            break;
        }
        let marker = data[m];
        let seg_start = m - 1;
        if marker == 0xD9 {
            out.extend_from_slice(&data[seg_start..]);
            return Ok(out);
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&data[seg_start..m + 1]);
            pos = m + 1;
            continue;
        }
        if m + 3 > data.len() {
            return Err(corrupt("JPEG"));
        }
        let len = u16::from_be_bytes([data[m + 1], data[m + 2]]) as usize;
        let end = m + 1 + len;
        if len < 2 || end > data.len() {
            return Err(corrupt("JPEG"));
        }
        let payload = &data[m + 3..end];
        let keep = match marker {
            0xE0 | 0xE2 | 0xEE => true, // JFIF, ICC profile, Adobe
            0xE1 => {
                if payload.starts_with(b"Exif\0\0") {
                    orientation = orientation.or_else(|| exif_orientation(&payload[6..]));
                }
                false
            }
            0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        // Insert the minimal orientation block right before the first table/frame segment,
        // after the header APP segments (by then any EXIF block has been seen).
        let header_app = matches!(marker, 0xE0 | 0xE2 | 0xEE);
        if keep && !header_app && !wrote_orientation {
            if let Some(o) = orientation.filter(|o| (2..=8).contains(o)) {
                out.extend_from_slice(&orientation_app1(o));
            }
            wrote_orientation = true;
        }
        if keep {
            out.extend_from_slice(&data[seg_start..end]);
        }
        if marker == 0xDA {
            // Start of scan: the rest is entropy-coded data followed by EOI.
            out.extend_from_slice(&data[end..]);
            return Ok(out);
        }
        pos = end;
    }
    Ok(out)
}

/// Orientation tag (0x0112) from a TIFF-structured EXIF block.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    if tiff.len() < 8 {
        return None;
    }
    let le = match &tiff[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |i: usize| -> Option<u16> {
        let b = tiff.get(i..i + 2)?;
        Some(if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    };
    let u32_at = |i: usize| -> Option<u32> {
        let b = tiff.get(i..i + 4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Some(if le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    for k in 0..count {
        let e = ifd + 2 + k * 12;
        if u16_at(e)? == 0x0112 {
            return u16_at(e + 8);
        }
    }
    None
}

/// APP1 segment with an EXIF block containing only the orientation.
fn orientation_app1(o: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0\x2A\0\0\0\x08"); // big-endian header, IFD0 at 8
    tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&o.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
    let mut seg = vec![0xFF, 0xE1];
    let len = (2 + 6 + tiff.len()) as u16;
    seg.extend_from_slice(&len.to_be_bytes());
    seg.extend_from_slice(b"Exif\0\0");
    seg.extend_from_slice(&tiff);
    seg
}

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";

pub fn strip_png(data: &[u8]) -> Result<Vec<u8>> {
    if !data.starts_with(PNG_SIG) {
        return Err(corrupt("PNG"));
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(PNG_SIG);
    let mut pos = PNG_SIG.len();
    while pos + 12 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos + 12 + len;
        if end > data.len() {
            return Err(corrupt("PNG"));
        }
        let kind = &data[pos + 4..pos + 8];
        let drop = matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME");
        if !drop {
            out.extend_from_slice(&data[pos..end]);
        }
        pos = end;
        if kind == b"IEND" {
            break;
        }
    }
    Ok(out)
}

pub fn strip_webp(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err(corrupt("WebP"));
    }
    let mut body = Vec::with_capacity(data.len());
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let fourcc = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let padded = size + (size & 1);
        let end = (pos + 8 + padded).min(data.len());
        if pos + 8 + size > data.len() {
            return Err(corrupt("WebP"));
        }
        match fourcc {
            b"EXIF" | b"XMP " => {}
            b"VP8X" => {
                let mut chunk = data[pos..end].to_vec();
                if chunk.len() > 8 {
                    chunk[8] &= !(0x08 | 0x04);
                }
                body.extend_from_slice(&chunk);
            }
            _ => body.extend_from_slice(&data[pos..end]),
        }
        pos = end;
    }
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn strip_gif(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 13 || !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        return Err(corrupt("GIF"));
    }
    let mut out = Vec::with_capacity(data.len());
    let flags = data[10];
    let mut pos = 13;
    if flags & 0x80 != 0 {
        pos += 3 * (1usize << ((flags & 0x07) + 1));
    }
    if pos > data.len() {
        return Err(corrupt("GIF"));
    }
    out.extend_from_slice(&data[..pos]);

    // Returns the end of a sub-block chain starting at `p`.
    let skip_blocks = |mut p: usize| -> Result<usize> {
        loop {
            let len = *data.get(p).ok_or_else(|| corrupt("GIF"))? as usize;
            p += 1 + len;
            if len == 0 {
                return Ok(p);
            }
            if p > data.len() {
                return Err(corrupt("GIF"));
            }
        }
    };

    while pos < data.len() {
        match data[pos] {
            0x3B => {
                out.push(0x3B);
                return Ok(out);
            }
            0x21 => {
                let label = *data.get(pos + 1).ok_or_else(|| corrupt("GIF"))?;
                let end = skip_blocks(pos + 2)?;
                let keep = match label {
                    0xFE => false, // comment
                    0xFF => {
                        let id = data.get(pos + 3..pos + 14).unwrap_or(&[]);
                        id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0"
                    }
                    _ => true,
                };
                if keep {
                    out.extend_from_slice(&data[pos..end]);
                }
                pos = end;
            }
            0x2C => {
                let desc_end = pos + 10;
                let lflags = *data.get(pos + 9).ok_or_else(|| corrupt("GIF"))?;
                let mut p = desc_end;
                if lflags & 0x80 != 0 {
                    p += 3 * (1usize << ((lflags & 0x07) + 1));
                }
                p += 1; // LZW minimum code size
                let end = skip_blocks(p)?;
                out.extend_from_slice(&data[pos..end]);
                pos = end;
            }
            _ => return Err(corrupt("GIF")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with_exif(orientation: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        // APP0 JFIF
        v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        // APP1 EXIF with orientation + GPS-ish junk
        let mut app1 = orientation_app1(orientation);
        app1.extend_from_slice(b"secret-location");
        let len = (app1.len() - 2) as u16;
        app1[2..4].copy_from_slice(&len.to_be_bytes());
        v.extend_from_slice(&app1);
        // COM
        v.extend_from_slice(&[0xFF, 0xFE, 0x00, 0x07]);
        v.extend_from_slice(b"hello");
        // DQT (fake)
        v.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x04, 0xAA, 0xBB]);
        // SOS + data + EOI
        v.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x03, 0x01, 0x12, 0x34, 0xFF, 0xD9]);
        v
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn jpeg_drops_exif_and_comment_but_keeps_orientation() {
        let src = jpeg_with_exif(6);
        let out = strip_jpeg(&src).unwrap();
        assert!(!contains(&out, b"secret-location"));
        assert!(!contains(&out, b"hello"));
        assert!(contains(&out, b"JFIF"));
        assert!(contains(&out, &[0xFF, 0xDB, 0x00, 0x04, 0xAA, 0xBB]));
        assert!(out.ends_with(&[0x12, 0x34, 0xFF, 0xD9]));
        // Minimal orientation block survives.
        let pos = out.windows(6).position(|w| w == b"Exif\0\0").unwrap();
        assert_eq!(exif_orientation(&out[pos + 6..]), Some(6));
    }

    #[test]
    fn jpeg_with_default_orientation_gets_no_exif() {
        let out = strip_jpeg(&jpeg_with_exif(1)).unwrap();
        assert!(!contains(&out, b"Exif"));
    }

    #[test]
    fn png_text_chunks_are_removed() {
        let mut v = PNG_SIG.to_vec();
        let chunk = |kind: &[u8], data: &[u8]| {
            let mut c = (data.len() as u32).to_be_bytes().to_vec();
            c.extend_from_slice(kind);
            c.extend_from_slice(data);
            c.extend_from_slice(&[0, 0, 0, 0]);
            c
        };
        v.extend(chunk(b"IHDR", &[0; 13]));
        v.extend(chunk(b"tEXt", b"Author\0me"));
        v.extend(chunk(b"eXIf", b"MM\0*"));
        v.extend(chunk(b"IDAT", &[1, 2, 3]));
        v.extend(chunk(b"IEND", &[]));
        let out = strip_png(&v).unwrap();
        assert!(!contains(&out, b"Author"));
        assert!(!contains(&out, b"eXIf"));
        assert!(contains(&out, b"IDAT"));
        assert!(contains(&out, b"IEND"));
    }

    #[test]
    fn webp_exif_removed_and_flags_cleared() {
        let mut body = Vec::new();
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x0C, 0, 0, 0, 1, 0, 0, 1, 0, 0]);
        body.extend_from_slice(b"VP8L");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[9, 9, 9, 0]); // padded
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(b"gps!");
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(&body);
        let out = strip_webp(&v).unwrap();
        assert!(!contains(&out, b"gps!"));
        assert_eq!(out[20] & 0x0C, 0);
        let riff = u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff, out.len() - 8);
    }

    #[test]
    fn gif_comments_removed_loop_kept() {
        let mut v = b"GIF89a".to_vec();
        v.extend_from_slice(&[1, 0, 1, 0, 0x80, 0, 0]); // 1x1, global table of 2 colours
        v.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        v.extend_from_slice(&[0x21, 0xFF, 11]);
        v.extend_from_slice(b"NETSCAPE2.0");
        v.extend_from_slice(&[3, 1, 0, 0, 0]);
        v.extend_from_slice(&[0x21, 0xFE, 6]);
        v.extend_from_slice(b"secret");
        v.push(0);
        v.extend_from_slice(&[0x21, 0xFF, 11]);
        v.extend_from_slice(b"XMP DataXMP");
        v.extend_from_slice(&[2, b'x', b'y', 0]);
        v.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4C, 0x01, 0]);
        v.push(0x3B);
        let out = strip_gif(&v).unwrap();
        assert!(!contains(&out, b"secret"));
        assert!(!contains(&out, b"XMP DataXMP"));
        assert!(contains(&out, b"NETSCAPE2.0"));
        assert_eq!(*out.last().unwrap(), 0x3B);
    }

    #[test]
    fn rejects_garbage() {
        assert!(strip(Format::Jpg, b"nope").is_err());
        assert!(strip(Format::Png, b"nope").is_err());
        assert!(strip(Format::Heic, b"nope").is_err());
    }
}
