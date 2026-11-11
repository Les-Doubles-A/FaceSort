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
pub mod quality;
pub mod recognize;
pub mod scan;
pub mod sort;
pub mod store;
pub mod util;

pub use error::{Error, Result};

/// Seuils calibrés par modèle de reconnaissance (voir README, « Calibration »).
/// Les deux modèles n'ont pas la même échelle de similarité.
pub mod thresholds {
    use crate::embed::EmbedderKind;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Thresholds {
        /// Similarité cosinus minimale pour attribuer un visage à une personne connue.
        pub recognition: f32,
        /// Écart minimal entre la meilleure et la deuxième personne (sinon : ambigu).
        pub margin: f32,
        /// Similarité cosinus minimale pour relier deux visages lors du regroupement.
        pub cluster: f32,
        /// Nombre minimal de visages voisins (lui compris) pour former un groupe.
        pub min_samples: usize,
        /// Exigence supplémentaire quand un visage est de mauvaise qualité.
        pub low_quality_penalty: f32,
        /// Similarité minimale d'un membre au centre de son groupe (anti-chaînage).
        pub centroid_min: f32,
    }

    /// Version des seuils : quand elle change, les attributions automatiques
    /// faites avec les anciens seuils sont réévaluées une fois.
    pub const REVISION: i64 = 2;

    /// Calibration du 2026-11-11 sur 164 visages de 6 femmes (portraits publics
    /// et versions sombres, ternes, brûlées, floues), voir README. Principe :
    /// mieux vaut deux groupes à fusionner qu'un groupe mélangé.
    pub fn for_kind(kind: EmbedderKind) -> Thresholds {
        match kind {
            // Mélange observé dès 0,35 ; pire paire « différentes » : 0,40 (bonnes
            // photos), 0,54 (photo médiocre).
            EmbedderKind::SFace => Thresholds {
                recognition: 0.45,
                margin: 0.05,
                cluster: 0.50,
                min_samples: 2,
                low_quality_penalty: 0.15,
                centroid_min: 0.45,
            },
            // Mélange observé dès 0,30 ; pire paire « différentes » : 0,33 / 0,42.
            EmbedderKind::AuraFace => Thresholds {
                recognition: 0.40,
                margin: 0.05,
                cluster: 0.45,
                min_samples: 2,
                low_quality_penalty: 0.15,
                centroid_min: 0.40,
            },
        }
    }

    /// Score minimal du détecteur.
    pub const DETECTION_SCORE: f32 = 0.9;
    /// Taille minimale d'un visage, en pixels de l'image d'origine.
    pub const MIN_FACE_PX: f32 = 40.0;
}
