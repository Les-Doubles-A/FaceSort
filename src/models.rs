//! Localisation et vérification (SHA-256) des modèles ONNX.

use crate::{Error, Result, util};
use std::path::{Path, PathBuf};

pub const DETECTOR_FILE: &str = "face_detection_yunet_2026may.onnx";
pub const EMBEDDER_FILE: &str = "face_recognition_sface_2021dec.onnx";
pub const SUMS_FILE: &str = "SHA256SUMS";

/// Identité des modèles utilisés, enregistrée dans la base : des empreintes
/// produites par un autre modèle ne sont pas comparables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelIds {
    pub detector: String,
    pub embedder: String,
}

/// Cherche le dossier `models/` : variable `FACESORT_MODELS`, à côté de
/// l'exécutable, dans le dossier courant, puis dans le dossier du projet.
pub fn find_models_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(p) = std::env::var("FACESORT_MODELS") {
        candidates.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("models"));
    }
    candidates.push(PathBuf::from("models"));
    candidates.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/models")));
    candidates.into_iter().find(|d| d.join(DETECTOR_FILE).is_file() && d.join(EMBEDDER_FILE).is_file())
}

fn expected_sums(dir: &Path) -> Result<Vec<(String, String)>> {
    let text = std::fs::read_to_string(dir.join(SUMS_FILE))
        .map_err(|e| Error::Model(format!("{} illisible ({e})", dir.join(SUMS_FILE).display())))?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_lowercase(), it.next()?.trim_start_matches('*').to_string()))
        })
        .collect())
}

/// Vérifie les deux modèles contre `SHA256SUMS` et renvoie leur identité.
pub fn verify(dir: &Path) -> Result<ModelIds> {
    let sums = expected_sums(dir)?;
    let check = |file: &str| -> Result<String> {
        let expected = sums
            .iter()
            .find(|(_, f)| f == file)
            .map(|(h, _)| h.clone())
            .ok_or_else(|| Error::Model(format!("{file} absent de {SUMS_FILE}")))?;
        let path = dir.join(file);
        let actual = util::sha256_file(&path).map_err(|e| Error::Model(format!("{} ({e})", path.display())))?;
        if actual != expected {
            return Err(Error::Model(format!("{file} : empreinte SHA-256 inattendue")));
        }
        Ok(format!("{file}#{}", &actual[..16]))
    };
    Ok(ModelIds { detector: check(DETECTOR_FILE)?, embedder: check(EMBEDDER_FILE)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tampered_model_is_rejected() {
        let Some(src) = find_models_dir() else {
            eprintln!("modèles absents : test ignoré (lancer scripts/fetch-models.ps1)");
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        std::fs::copy(src.join(SUMS_FILE), tmp.path().join(SUMS_FILE)).unwrap();
        std::fs::copy(src.join(DETECTOR_FILE), tmp.path().join(DETECTOR_FILE)).unwrap();
        std::fs::write(tmp.path().join(EMBEDDER_FILE), b"not a model").unwrap();
        let err = verify(tmp.path()).unwrap_err().to_string();
        assert!(err.contains("modèle manquant ou modifié"), "{err}");
        assert!(verify(&src).is_ok());
    }
}
