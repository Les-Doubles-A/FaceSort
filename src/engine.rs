//! Chaîne d'analyse d'une image : détection, alignement, empreinte, qualité, vignette.

use crate::align::align_face;
use crate::detect::{Detection, FaceDetector, YuNet};
use crate::embed::{EmbedderKind, FaceEmbedder, OnnxEmbedder};
use crate::models::{self, ModelIds};
use crate::quality::FaceQuality;
use crate::{Error, Result, imageio, thresholds};
use image::RgbImage;
use std::path::Path;

/// Un visage analysé ; coordonnées dans l'image d'origine (après orientation).
#[derive(Clone, Debug)]
pub struct AnalyzedFace {
    pub detection: Detection,
    pub embedding: Vec<f32>,
    pub quality: FaceQuality,
    pub thumb_jpeg: Vec<u8>,
}

/// ONNX Runtime n'accepte qu'un environnement par processus : il est créé une
/// fois et partagé (sinon une deuxième analyse dans la même session échoue).
fn environment() -> Result<ort::environment::Environment> {
    static ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();
    if let Some(e) = ENV.get() {
        return Ok(e.clone());
    }
    let env = ort::init().with_name("facesort").with_telemetry(false).build()?;
    Ok(ENV.get_or_init(|| env).clone())
}

pub struct Engine {
    pub detector: Box<dyn FaceDetector>,
    pub embedder: Box<dyn FaceEmbedder>,
    pub kind: EmbedderKind,
    pub ids: ModelIds,
    pub min_face_px: f32,
}

impl Engine {
    /// Charge les modèles du dossier donné (ou du premier dossier `models/`
    /// trouvé) avec le modèle de reconnaissance choisi (`FACESORT_EMBEDDER`).
    pub fn load(models_dir: Option<&Path>) -> Result<Self> {
        Self::load_with(models_dir, models::selected_embedder()?)
    }

    pub fn load_with(models_dir: Option<&Path>, kind: EmbedderKind) -> Result<Self> {
        let dir = match models_dir {
            Some(d) => d.to_path_buf(),
            None => models::find_models_dir().ok_or_else(|| {
                Error::Model(
                    "dossier models/ introuvable (lancer scripts/fetch-models.ps1 ou définir FACESORT_MODELS)".into(),
                )
            })?,
        };
        let ids = models::verify(&dir, kind)?;
        let env = environment()?;
        let detector = YuNet::load(&env, &dir.join(models::DETECTOR_FILE))?;
        let embedder = OnnxEmbedder::load(&env, &dir.join(kind.file()), kind)?;
        Ok(Self {
            detector: Box::new(detector),
            embedder: Box::new(embedder),
            kind,
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
            let size = d.bbox[2].min(d.bbox[3]);
            if size < self.min_face_px {
                continue;
            }
            let aligned = align_face(img, &d.landmarks);
            let quality = FaceQuality::measure(&aligned, size, d.score);
            let embedding = self.embedder.embed(&aligned)?;
            let thumb_jpeg = imageio::face_thumbnail(img, d.bbox);
            faces.push(AnalyzedFace { detection: d, embedding, quality, thumb_jpeg });
        }
        Ok(faces)
    }
}
