//! Qualité d'un visage : un visage sombre, peu contrasté, flou ou petit donne
//! une empreinte peu fiable. Ces visages ne doivent ni relier deux groupes
//! (regroupement) ni servir de référence (reconnaissance).
//!
//! Mesures sur le visage aligné 112x112 (échelle fixe, donc comparables) :
//! luminance moyenne, écart-type (contraste), variance du laplacien (netteté).

use image::RgbImage;

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct FaceQuality {
    /// Luminance moyenne, 0..255.
    pub brightness: f32,
    /// Écart-type de la luminance, 0..128.
    pub contrast: f32,
    /// Variance du laplacien de la luminance.
    pub sharpness: f32,
    /// Plus petit côté du visage dans l'image d'origine, en pixels.
    pub size: f32,
    /// Score du détecteur, 0..1.
    pub det_score: f32,
}

/// Seuils de qualité (calibrés, voir README « Calibration »).
pub mod limits {
    pub const MIN_BRIGHTNESS: f32 = 50.0;
    pub const MAX_BRIGHTNESS: f32 = 215.0;
    pub const MIN_CONTRAST: f32 = 22.0;
    pub const MIN_SHARPNESS: f32 = 40.0;
    pub const MIN_SIZE: f32 = 64.0;
    pub const MIN_DET_SCORE: f32 = 0.92;
    /// En dessous de cette note globale, un visage est « peu fiable ».
    pub const GOOD: f32 = 0.5;
}

fn ramp(v: f32, bad: f32, good: f32) -> f32 {
    ((v - bad) / (good - bad)).clamp(0.0, 1.0)
}

impl FaceQuality {
    pub fn measure(aligned: &RgbImage, size: f32, det_score: f32) -> Self {
        let (w, h) = (aligned.width() as usize, aligned.height() as usize);
        let luma: Vec<f32> =
            aligned.pixels().map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).collect();
        // Zone centrale (yeux, nez, bouche) : on ignore les bords noirs de l'alignement.
        let (x0, x1, y0, y1) = (w / 6, w - w / 6, h / 6, h - h / 12);
        let mut sum = 0.0;
        let mut sum2 = 0.0;
        let mut n = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                let v = luma[y * w + x];
                sum += v;
                sum2 += v * v;
                n += 1.0;
            }
        }
        let mean = sum / n;
        let contrast = (sum2 / n - mean * mean).max(0.0).sqrt();
        let mut lap_sum = 0.0;
        let mut lap_sum2 = 0.0;
        let mut m = 0.0;
        for y in y0.max(1)..y1.min(h - 1) {
            for x in x0.max(1)..x1.min(w - 1) {
                let c = luma[y * w + x];
                let l = luma[(y - 1) * w + x] + luma[(y + 1) * w + x] + luma[y * w + x - 1] + luma[y * w + x + 1] - 4.0 * c;
                lap_sum += l;
                lap_sum2 += l * l;
                m += 1.0;
            }
        }
        let lap_mean = lap_sum / m;
        let sharpness = (lap_sum2 / m - lap_mean * lap_mean).max(0.0);
        Self { brightness: mean, contrast, sharpness, size, det_score }
    }

    /// Note globale 0..1 : le pire critère l'emporte.
    pub fn score(&self) -> f32 {
        use limits::*;
        let bright = ramp(self.brightness, MIN_BRIGHTNESS * 0.5, MIN_BRIGHTNESS)
            .min(ramp(-self.brightness, -(MAX_BRIGHTNESS + 255.0) / 2.0, -MAX_BRIGHTNESS));
        let contrast = ramp(self.contrast, MIN_CONTRAST * 0.5, MIN_CONTRAST);
        let sharp = ramp(self.sharpness, MIN_SHARPNESS * 0.3, MIN_SHARPNESS);
        let size = ramp(self.size, MIN_SIZE * 0.6, MIN_SIZE);
        let det = ramp(self.det_score, 0.9, MIN_DET_SCORE);
        bright.min(contrast).min(sharp).min(size).min(det)
    }

    pub fn is_good(&self) -> bool {
        self.score() >= limits::GOOD
    }

    /// Raison principale d'une mauvaise note, pour l'affichage.
    pub fn main_issue(&self) -> Option<&'static str> {
        use limits::*;
        if self.is_good() {
            return None;
        }
        Some(if self.brightness < MIN_BRIGHTNESS {
            "visage sombre"
        } else if self.brightness > MAX_BRIGHTNESS {
            "visage surexposé"
        } else if self.contrast < MIN_CONTRAST {
            "peu de contraste"
        } else if self.sharpness < MIN_SHARPNESS {
            "visage flou"
        } else if self.size < MIN_SIZE {
            "visage trop petit"
        } else {
            "détection incertaine"
        })
    }
}

/// Une note de qualité absente (base antérieure) est considérée comme bonne.
pub fn is_good_score(q: Option<f32>) -> bool {
    q.is_none_or(|v| v >= limits::GOOD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn textured(scale: f32, offset: f32) -> RgbImage {
        RgbImage::from_fn(112, 112, |x, y| {
            let v = (((x * 7 + y * 13) % 32) as f32 * 4.0 - 64.0) * scale + offset;
            let v = v.clamp(0.0, 255.0) as u8;
            Rgb([v, v, v])
        })
    }

    #[test]
    fn good_face_scores_high() {
        let q = FaceQuality::measure(&textured(1.0, 128.0), 150.0, 0.97);
        assert!(q.is_good(), "{q:?} -> {}", q.score());
        assert_eq!(q.main_issue(), None);
    }

    #[test]
    fn dark_face_is_flagged() {
        let q = FaceQuality::measure(&textured(0.2, 20.0), 150.0, 0.97);
        assert!(!q.is_good(), "{q:?}");
        assert_eq!(q.main_issue(), Some("visage sombre"));
    }

    #[test]
    fn flat_face_is_flagged() {
        let q = FaceQuality::measure(&textured(0.05, 128.0), 150.0, 0.97);
        assert!(!q.is_good());
        assert_eq!(q.main_issue(), Some("peu de contraste"));
    }

    #[test]
    fn small_face_is_flagged() {
        let q = FaceQuality::measure(&textured(1.0, 128.0), 30.0, 0.97);
        assert_eq!(q.main_issue(), Some("visage trop petit"));
    }

    #[test]
    fn missing_score_counts_as_good() {
        assert!(is_good_score(None));
        assert!(!is_good_score(Some(0.1)));
    }
}
