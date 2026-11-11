//! Chaîne d'analyse d'une image : détection, alignement, empreinte, vignette.

use crate::align::align_face;
use crate::detect::{Detection, FaceDetector, YuNet};
use crate::embed::{FaceEmbedder, SFace};
use crate::models::{self, ModelIds};
use crate::{Error, Result, imageio, thresholds};
use image::RgbImage;
use std::path::Path;

/// Un visage analysé ; coordonnées dans l'image d'origine (après orientation).
#[derive(Clone, Debug)]
pub struct AnalyzedFace {
    pub detection: Detection,
    pub embedding: Vec<f32>,
    pub thumb_jpeg: Vec<u8>,
}

pub struct Engine {
    pub detector: Box<dyn FaceDetector>,
    pub embedder: Box<dyn FaceEmbedder>,
    pub ids: ModelIds,
    pub min_face_px: f32,
}

impl Engine {
    /// Charge les modèles du dossier donné, ou du premier dossier `models/` trouvé.
    pub fn load(models_dir: Option<&Path>) -> Result<Self> {
        let dir = match models_dir {
            Some(d) => d.to_path_buf(),
            None => models::find_models_dir().ok_or_else(|| {
                Error::Model(
                    "dossier models/ introuvable (lancer scripts/fetch-models.ps1 ou définir FACESORT_MODELS)".into(),
                )
            })?,
        };
        let ids = models::verify(&dir)?;
        let env = ort::init().with_name("facesort").with_telemetry(false).build()?;
        let detector = YuNet::load(&env, &dir.join(models::DETECTOR_FILE))?;
        let embedder = SFace::load(&env, &dir.join(models::EMBEDDER_FILE))?;
        Ok(Self {
            detector: Box::new(detector),
            embedder: Box::new(embedder),
            ids,
            min_face_px: thresholds::MIN_FACE_PX,
        })
    }

    /// `img` : image d'origine orientée. La détection se fait sur une copie
    /// réduite à [`imageio::MAX_SIDE`] ; l'alignement sur l'image d'origine.
    pub fn analyze(&mut self, img: &RgbImage) -> Result<Vec<AnalyzedFace>> {
        let (small, scale) = imageio::downscale(img.clone(), imageio::MAX_SIDE);
        let detections = self.detector.detect(&small)?;
        let mut faces = Vec::with_capacity(detections.len());
        for d in detections {
            let d = d.scaled(scale);
            if d.bbox[2].min(d.bbox[3]) < self.min_face_px {
                continue;
            }
            let aligned = align_face(img, &d.landmarks);
            let embedding = self.embedder.embed(&aligned)?;
            let thumb_jpeg = imageio::face_thumbnail(img, d.bbox);
            faces.push(AnalyzedFace { detection: d, embedding, thumb_jpeg });
        }
        Ok(faces)
    }
}
