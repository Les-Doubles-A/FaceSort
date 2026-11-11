//! FaceSort : détection, reconnaissance et tri de photos par personne, en local.
//!
//! Organisation :
//! - [`imageio`] décodage, orientation EXIF, réduction ;
//! - [`detect`], [`align`], [`embed`], [`engine`] : modèles YuNet et SFace via ONNX Runtime ;
//! - [`store`] : base SQLite (photos, visages, personnes) ;
//! - [`scan`] : analyse d'un dossier ;
//! - [`recognize`], [`cluster`] : attribution aux personnes connues et regroupement ;
//! - [`sort`] : copie dans `Destination/Prénom/` ;
//! - [`names`] : validation des prénoms comme noms de dossier Windows.

pub mod align;
pub mod cluster;
pub mod detect;
pub mod embed;
pub mod engine;
pub mod error;
pub mod imageio;
pub mod models;
pub mod names;
pub mod recognize;
pub mod scan;
pub mod sort;
pub mod store;
pub mod util;

pub use error::{Error, Result};

/// Seuils calibrés sur les fixtures (voir README, section « Calibration »).
pub mod thresholds {
    /// Similarité cosinus minimale pour attribuer un visage à une personne connue.
    pub const RECOGNITION: f32 = 0.40;
    /// Écart minimal entre la meilleure et la deuxième personne (sinon : ambigu).
    pub const AMBIGUITY_MARGIN: f32 = 0.05;
    /// Similarité cosinus minimale pour relier deux visages lors du regroupement.
    pub const CLUSTER: f32 = 0.45;
    /// Nombre minimal de visages voisins (lui compris) pour former un groupe.
    pub const CLUSTER_MIN_SAMPLES: usize = 2;
    /// Score minimal du détecteur.
    pub const DETECTION_SCORE: f32 = 0.9;
    /// Taille minimale d'un visage, en pixels de l'image d'origine.
    pub const MIN_FACE_PX: f32 = 40.0;
}
