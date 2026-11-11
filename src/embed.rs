//! Empreintes faciales : SFace (OpenCV Zoo, Apache 2.0) ou AuraFace-v1 (fal.ai, Apache 2.0).
//!
//! Les deux modèles prennent un visage aligné 112x112 (même gabarit 5 points),
//! en RGB :
//! - SFace : valeurs 0..255 (comme `cv::FaceRecognizerSF`), empreinte de 128 ;
//! - AuraFace (`glintr100`, ResNet100 type ArcFace) : valeurs (x - 127,5) / 127,5
//!   (convention InsightFace, vérifiée par mesure), empreinte de 512.
//!
//! L'empreinte est normalisée (norme L2 = 1) : le produit scalaire de deux
//! empreintes est leur similarité cosinus. Des empreintes de deux modèles
//! différents ne sont pas comparables.

use crate::{Error, Result};
use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbedderKind {
    SFace,
    AuraFace,
}

impl EmbedderKind {
    pub fn file(self) -> &'static str {
        match self {
            EmbedderKind::SFace => "face_recognition_sface_2021dec.onnx",
            EmbedderKind::AuraFace => "auraface_glintr100.onnx",
        }
    }

    pub fn dim(self) -> usize {
        match self {
            EmbedderKind::SFace => 128,
            EmbedderKind::AuraFace => 512,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EmbedderKind::SFace => "SFace",
            EmbedderKind::AuraFace => "AuraFace",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sface" => Some(EmbedderKind::SFace),
            "auraface" => Some(EmbedderKind::AuraFace),
            _ => None,
        }
    }

    fn pixel(self, v: u8) -> f32 {
        match self {
            EmbedderKind::SFace => v as f32,
            EmbedderKind::AuraFace => (v as f32 - 127.5) / 127.5,
        }
    }
}

pub trait FaceEmbedder: Send {
    /// `aligned` : visage aligné 112x112.
    fn embed(&mut self, aligned: &RgbImage) -> Result<Vec<f32>>;
}

pub struct OnnxEmbedder {
    session: Session,
    kind: EmbedderKind,
}

impl OnnxEmbedder {
    pub fn load(env: &Environment, path: &Path, kind: EmbedderKind) -> Result<Self> {
        let session = Session::builder(env)
            .map_err(Error::inference)?
            .commit_from_file(path)
            .map_err(Error::inference)?;
        Ok(Self { session, kind })
    }
}

pub fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl FaceEmbedder for OnnxEmbedder {
    fn embed(&mut self, aligned: &RgbImage) -> Result<Vec<f32>> {
        let (w, h) = (aligned.width() as usize, aligned.height() as usize);
        let plane = w * h;
        let mut blob = vec![0f32; 3 * plane];
        for (x, y, p) in aligned.enumerate_pixels() {
            let i = y as usize * w + x as usize;
            blob[i] = self.kind.pixel(p[0]);
            blob[plane + i] = self.kind.pixel(p[1]);
            blob[2 * plane + i] = self.kind.pixel(p[2]);
        }
        let input = TensorRef::from_array_view(([1usize, 3, h, w], blob.as_slice()))?;
        let outputs = self.session.run(ort::inputs![input])?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
        if data.len() != self.kind.dim() {
            return Err(Error::Inference(format!(
                "empreinte de taille {} au lieu de {} ({})",
                data.len(),
                self.kind.dim(),
                self.kind.label()
            )));
        }
        let mut v = data.to_vec();
        l2_normalize(&mut v);
        Ok(v)
    }
}

pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip() {
        let v = vec![0.5, -1.25, 3.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
    }

    #[test]
    fn normalize_unit() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pixel_conventions() {
        assert_eq!(EmbedderKind::SFace.pixel(255), 255.0);
        assert_eq!(EmbedderKind::AuraFace.pixel(255), 1.0);
        assert_eq!(EmbedderKind::AuraFace.pixel(0), -1.0);
        assert_eq!(EmbedderKind::parse(" AuraFace "), Some(EmbedderKind::AuraFace));
    }
}
