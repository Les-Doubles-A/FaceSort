//! Empreintes faciales avec SFace (OpenCV Zoo, licence Apache 2.0).
//!
//! Entrée `data` 1x3x112x112 en RGB flottant 0..255 (comme `blobFromImage`
//! avec `swapRB=true` dans `cv::FaceRecognizerSF`), sortie `fc1` 1x128.
//! L'empreinte est normalisée (norme L2 = 1) : le produit scalaire de deux
//! empreintes est leur similarité cosinus.

use crate::{Error, Result};
use image::RgbImage;
use ort::environment::Environment;
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;

pub const EMBED_DIM: usize = 128;

pub trait FaceEmbedder: Send {
    /// `aligned` : visage aligné 112x112.
    fn embed(&mut self, aligned: &RgbImage) -> Result<Vec<f32>>;
}

pub struct SFace {
    session: Session,
}

impl SFace {
    pub fn load(env: &Environment, path: &Path) -> Result<Self> {
        let session = Session::builder(env)
            .map_err(Error::inference)?
            .commit_from_file(path)
            .map_err(Error::inference)?;
        Ok(Self { session })
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

impl FaceEmbedder for SFace {
    fn embed(&mut self, aligned: &RgbImage) -> Result<Vec<f32>> {
        let (w, h) = (aligned.width() as usize, aligned.height() as usize);
        let plane = w * h;
        let mut blob = vec![0f32; 3 * plane];
        for (x, y, p) in aligned.enumerate_pixels() {
            let i = y as usize * w + x as usize;
            blob[i] = p[0] as f32;
            blob[plane + i] = p[1] as f32;
            blob[2 * plane + i] = p[2] as f32;
        }
        let input = TensorRef::from_array_view(([1usize, 3, h, w], blob.as_slice()))?;
        let outputs = self.session.run(ort::inputs![input])?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
        if data.len() != EMBED_DIM {
            return Err(Error::Inference(format!("empreinte de taille {} au lieu de {EMBED_DIM}", data.len())));
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
}
