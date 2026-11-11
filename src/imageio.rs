//! Décodage des photos : limites mémoire, orientation EXIF, réduction.

use crate::Result;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbImage, imageops};
use std::io::Cursor;

/// Grand côté maximal de l'image analysée (H3).
pub const MAX_SIDE: u32 = 1600;
/// Garde-fou contre les fichiers piégés : 100 mégapixels au plus.
const MAX_PIXELS_SIDE: u32 = 20_000;
const MAX_ALLOC: u64 = 1 << 30;

pub const EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

pub fn is_supported(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Vrai si un JPEG va jusqu'à son marqueur de fin (EOI) après les données
/// compressées. Le décodeur tolère les fichiers tronqués (il complète en gris),
/// d'où cette vérification. Les données ajoutées après EOI (photos animées)
/// sont acceptées.
pub fn jpeg_is_complete(b: &[u8]) -> bool {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return false;
    }
    let mut i = 2;
    loop {
        // Segment : FF xx [longueur sur 2 octets]
        while i < b.len() && b[i] == 0xFF {
            i += 1;
        }
        let Some(&marker) = b.get(i) else { return false };
        i += 1;
        match marker {
            0xD9 => return true,
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let Some(len) = b.get(i..i + 2).map(|l| u16::from_be_bytes([l[0], l[1]]) as usize) else { return false };
        i += len;
        if marker == 0xDA {
            // Données compressées jusqu'au prochain marqueur hors bourrage (FF00) et RSTn.
            loop {
                let Some(pos) = b.get(i..).and_then(|s| s.iter().position(|&x| x == 0xFF)) else { return false };
                i += pos;
                match b.get(i + 1) {
                    None => return false,
                    Some(0x00) | Some(0xD0..=0xD7) => i += 2,
                    Some(0xFF) => i += 1,
                    Some(_) => break,
                }
            }
        }
        if i > b.len() {
            return false;
        }
    }
}

/// Décode une image en mémoire et applique l'orientation EXIF.
pub fn decode_oriented(bytes: &[u8]) -> Result<RgbImage> {
    if bytes.starts_with(&[0xFF, 0xD8]) && !jpeg_is_complete(bytes) {
        return Err(crate::Error::Invalid("fichier JPEG tronqué".into()));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PIXELS_SIDE);
    limits.max_image_height = Some(MAX_PIXELS_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img.into_rgb8())
}

/// Réduit l'image si son grand côté dépasse `max_side`. Renvoie l'image et le
/// facteur à appliquer aux coordonnées pour revenir à l'image d'origine.
pub fn downscale(img: RgbImage, max_side: u32) -> (RgbImage, f32) {
    let (w, h) = img.dimensions();
    let side = w.max(h);
    if side <= max_side {
        return (img, 1.0);
    }
    let scale = max_side as f32 / side as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let small = imageops::resize(&img, nw, nh, imageops::FilterType::Triangle);
    (small, w as f32 / nw as f32)
}

/// Vignette carrée JPEG (96 px) centrée sur le cadre du visage, élargi de 25 %.
pub fn face_thumbnail(img: &RgbImage, bbox: [f32; 4]) -> Vec<u8> {
    let [x, y, w, h] = bbox;
    let side = w.max(h) * 1.25;
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let (iw, ih) = img.dimensions();
    let x0 = (cx - side / 2.0).clamp(0.0, iw.saturating_sub(1) as f32) as u32;
    let y0 = (cy - side / 2.0).clamp(0.0, ih.saturating_sub(1) as f32) as u32;
    let cw = (side as u32).min(iw - x0).max(1);
    let ch = (side as u32).min(ih - y0).max(1);
    let crop = imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    let thumb = imageops::resize(&crop, 96, 96, imageops::FilterType::Triangle);
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
    if thumb.write_with_encoder(enc).is_err() {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_keeps_small_images() {
        let img = RgbImage::new(800, 600);
        let (out, s) = downscale(img, MAX_SIDE);
        assert_eq!(out.dimensions(), (800, 600));
        assert_eq!(s, 1.0);
    }

    #[test]
    fn downscale_large_images() {
        let img = RgbImage::new(4000, 3000);
        let (out, s) = downscale(img, 1600);
        assert_eq!(out.dimensions(), (1600, 1200));
        assert!((s - 2.5).abs() < 1e-4);
    }

    #[test]
    fn truncated_jpeg_is_an_error() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/people3/obama_00.jpg")).unwrap();
        assert!(decode_oriented(&bytes[..bytes.len() / 3]).is_err());
        assert!(!jpeg_is_complete(&bytes[..bytes.len() - 1]));
        assert!(jpeg_is_complete(&bytes));
        let mut motion = bytes.clone();
        motion.extend_from_slice(b"....ftypmp42 donnees video ajoutees");
        assert!(jpeg_is_complete(&motion));
    }

    #[test]
    fn all_fixture_jpegs_are_complete() {
        for dir in ["basic", "people3", "people3_new", "exif"] {
            let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(dir);
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if is_supported(&p) {
                    assert!(jpeg_is_complete(&std::fs::read(&p).unwrap()), "{}", p.display());
                }
            }
        }
    }

    #[test]
    fn exif_orientation_is_applied() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/exif/");
        let up = decode_oriented(&std::fs::read(format!("{dir}upright.jpg")).unwrap()).unwrap();
        let rot = decode_oriented(&std::fs::read(format!("{dir}orientation6.jpg")).unwrap()).unwrap();
        assert_eq!(up.dimensions(), rot.dimensions());
    }

    #[test]
    fn supported_extensions() {
        assert!(is_supported(std::path::Path::new("a/B.JPG")));
        assert!(is_supported(std::path::Path::new("x.png")));
        assert!(!is_supported(std::path::Path::new("x.heic")));
    }
}
