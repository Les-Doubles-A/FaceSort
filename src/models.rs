//! Localisation et vérification (SHA-256) des modèles ONNX.

use crate::embed::EmbedderKind;
use crate::{Error, Result, util};
use std::path::{Path, PathBuf};

pub const DETECTOR_FILE: &str = "face_detection_yunet_2026may.onnx";
pub const SUMS_FILE: &str = "SHA256SUMS";
/// Modèle de reconnaissance par défaut pour une nouvelle base : AuraFace
/// confond moins les personnes (mesures du 2026-11-11, README « Choix du
/// modèle ») ; SFace sert de repli s'il n'est pas installé.
pub const DEFAULT_EMBEDDER: EmbedderKind = EmbedderKind::AuraFace;

/// Choix du modèle, par ordre de priorité : variable `FACESORT_EMBEDDER`
/// (`sface` ou `auraface`), modèle qui a rempli la base (`from_store`), défaut
/// s'il est installé, sinon SFace.
pub fn choose_embedder(from_store: Option<EmbedderKind>, models_dir: Option<&Path>) -> Result<EmbedderKind> {
    if let Ok(v) = std::env::var("FACESORT_EMBEDDER")
        && !v.trim().is_empty()
    {
        return EmbedderKind::parse(&v)
            .ok_or_else(|| Error::Invalid(format!("FACESORT_EMBEDDER={v} inconnu (attendu : sface ou auraface)")));
    }
    if let Some(k) = from_store {
        return Ok(k);
    }
    Ok(if is_installed(DEFAULT_EMBEDDER, models_dir) { DEFAULT_EMBEDDER } else { EmbedderKind::SFace })
}

/// Le fichier du modèle est-il présent ?
pub fn is_installed(kind: EmbedderKind, models_dir: Option<&Path>) -> bool {
    let dir = models_dir.map(Path::to_path_buf).or_else(find_models_dir);
    dir.is_some_and(|d| d.join(kind.file()).is_file())
}

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
    candidates.into_iter().find(|d| d.join(DETECTOR_FILE).is_file() && d.join(SUMS_FILE).is_file())
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

/// Vérifie le détecteur et le modèle de reconnaissance contre `SHA256SUMS`.
pub fn verify(dir: &Path, embedder: EmbedderKind) -> Result<ModelIds> {
    let sums = expected_sums(dir)?;
    let check = |file: &str| -> Result<String> {
        let expected = sums
            .iter()
            .find(|(_, f)| f == file)
            .map(|(h, _)| h.clone())
            .ok_or_else(|| Error::Model(format!("{file} absent de {SUMS_FILE}")))?;
        let path = dir.join(file);
        if !path.is_file() {
            return Err(Error::Model(format!("{} introuvable", path.display())));
        }
        let actual = util::sha256_file(&path).map_err(|e| Error::Model(format!("{} ({e})", path.display())))?;
        if actual != expected {
            return Err(Error::Model(format!("{file} : empreinte SHA-256 inattendue")));
        }
        Ok(format!("{file}#{}", &actual[..16]))
    };
    Ok(ModelIds { detector: check(DETECTOR_FILE)?, embedder: check(embedder.file())? })
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
        let file = EmbedderKind::SFace.file();
        std::fs::write(tmp.path().join(file), b"not a model").unwrap();
        let err = verify(tmp.path(), EmbedderKind::SFace).unwrap_err().to_string();
        assert!(err.contains("modèle manquant ou modifié"), "{err}");
        assert!(verify(&src, EmbedderKind::SFace).is_ok());
    }
}
