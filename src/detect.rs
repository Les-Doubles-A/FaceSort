//! Détection des visages avec YuNet (OpenCV Zoo, licence MIT).
//!
//! Le pré et post-traitement reproduisent `cv::FaceDetectorYN` d'OpenCV :
//! image BGR en flottants 0..255, complétée à un multiple de 32, sorties
//! `cls/obj/bbox/kps` aux pas 8, 16 et 32, puis suppression des non-maxima.

use crate::{Error, Result};
use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    /// x, y, largeur, hauteur, en pixels de l'image analysée.
    pub bbox: [f32; 4],
    /// Œil droit, œil gauche, nez, coin droit et coin gauche de la bouche.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

impl Detection {
    pub fn scaled(&self, s: f32) -> Detection {
        let mut d = self.clone();
        d.bbox.iter_mut().for_each(|v| *v *= s);
        d.landmarks.iter_mut().flatten().for_each(|v| *v *= s);
        d
    }
}

pub trait FaceDetector: Send {
    fn detect(&mut self, img: &RgbImage) -> Result<Vec<Detection>>;
}

pub struct YuNet {
    session: Session,
    pub score_threshold: f32,
    pub nms_threshold: f32,
    pub top_k: usize,
}

const STRIDES: [usize; 3] = [8, 16, 32];

impl YuNet {
    pub fn load(env: &Environment, path: &Path) -> Result<Self> {
        let session = Session::builder(env)
            .map_err(Error::inference)?
            .commit_from_file(path)
            .map_err(Error::inference)?;
        Ok(Self { session, score_threshold: crate::thresholds::DETECTION_SCORE, nms_threshold: 0.3, top_k: 5000 })
    }
}

/// Image RGB vers tenseur NCHW BGR, complété de zéros jusqu'à `pw` x `ph`.
pub fn to_bgr_blob(img: &RgbImage, pw: usize, ph: usize) -> Vec<f32> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let plane = pw * ph;
    let mut blob = vec![0f32; 3 * plane];
    for (x, y, p) in img.enumerate_pixels() {
        let i = y as usize * pw + x as usize;
        blob[i] = p[2] as f32;
        blob[plane + i] = p[1] as f32;
        blob[2 * plane + i] = p[0] as f32;
    }
    debug_assert!(w <= pw && h <= ph);
    blob
}

pub fn padded(v: usize) -> usize {
    (v.saturating_sub(1) / 32 + 1) * 32
}

impl FaceDetector for YuNet {
    fn detect(&mut self, img: &RgbImage) -> Result<Vec<Detection>> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        if w == 0 || h == 0 {
            return Ok(Vec::new());
        }
        let (pw, ph) = (padded(w), padded(h));
        let blob = to_bgr_blob(img, pw, ph);
        let input = TensorRef::from_array_view(([1usize, 3, ph, pw], blob.as_slice()))?;
        let outputs = self.session.run(ort::inputs![input])?;

        let mut candidates = Vec::new();
        for stride in STRIDES {
            let get = |name: String| -> Result<&[f32]> {
                let v = outputs.get(&name).ok_or_else(|| Error::Inference(format!("sortie {name} absente")))?;
                Ok(v.try_extract_tensor::<f32>()?.1)
            };
            let cls = get(format!("cls_{stride}"))?;
            let obj = get(format!("obj_{stride}"))?;
            let bbox = get(format!("bbox_{stride}"))?;
            let kps = get(format!("kps_{stride}"))?;
            let cols = pw / stride;
            let rows = ph / stride;
            if cls.len() < rows * cols || bbox.len() < rows * cols * 4 || kps.len() < rows * cols * 10 {
                return Err(Error::Inference(format!("sorties de taille inattendue au pas {stride}")));
            }
            let s = stride as f32;
            for r in 0..rows {
                for c in 0..cols {
                    let idx = r * cols + c;
                    let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
                    if score < self.score_threshold {
                        continue;
                    }
                    let b = &bbox[idx * 4..idx * 4 + 4];
                    let cx = (c as f32 + b[0]) * s;
                    let cy = (r as f32 + b[1]) * s;
                    let bw = b[2].exp() * s;
                    let bh = b[3].exp() * s;
                    let k = &kps[idx * 10..idx * 10 + 10];
                    let mut landmarks = [[0f32; 2]; 5];
                    for (n, lm) in landmarks.iter_mut().enumerate() {
                        *lm = [(k[2 * n] + c as f32) * s, (k[2 * n + 1] + r as f32) * s];
                    }
                    candidates.push(Detection { bbox: [cx - bw / 2.0, cy - bh / 2.0, bw, bh], landmarks, score });
                }
            }
        }
        Ok(nms(candidates, self.nms_threshold, self.top_k))
    }
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = (a[0] + a[2]).min(b[0] + b[2]);
    let y2 = (a[1] + a[3]).min(b[1] + b[3]);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Suppression des non-maxima, tri par score décroissant (comme `cv::dnn::NMSBoxes`).
pub fn nms(mut cands: Vec<Detection>, threshold: f32, top_k: usize) -> Vec<Detection> {
    cands.sort_by(|a, b| b.score.total_cmp(&a.score));
    cands.truncate(top_k);
    let mut kept: Vec<Detection> = Vec::new();
    for c in cands {
        if kept.iter().all(|k| iou(&k.bbox, &c.bbox) <= threshold) {
            kept.push(c);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(x: f32, score: f32) -> Detection {
        Detection { bbox: [x, 0.0, 10.0, 10.0], landmarks: [[0.0; 2]; 5], score }
    }

    #[test]
    fn nms_keeps_best_of_overlapping() {
        let out = nms(vec![det(0.0, 0.8), det(1.0, 0.95), det(50.0, 0.9)], 0.3, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].score, 0.95);
        assert_eq!(out[1].score, 0.9);
    }

    #[test]
    fn padding_to_32() {
        assert_eq!(padded(1), 32);
        assert_eq!(padded(32), 32);
        assert_eq!(padded(33), 64);
        assert_eq!(padded(900), 928);
    }

    #[test]
    fn blob_is_bgr_and_padded() {
        let mut img = RgbImage::new(2, 1);
        img.put_pixel(1, 0, image::Rgb([10, 20, 30]));
        let b = to_bgr_blob(&img, 4, 2);
        assert_eq!(b.len(), 24);
        assert_eq!((b[1], b[8 + 1], b[16 + 1]), (30.0, 20.0, 10.0));
        assert_eq!(b[3], 0.0);
    }
}
