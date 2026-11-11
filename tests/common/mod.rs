#![allow(dead_code)]
use facesort::engine::Engine;
use std::path::{Path, PathBuf};

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Charge le moteur, ou `None` (test ignoré avec un message) si les modèles manquent.
pub fn engine() -> Option<Engine> {
    if facesort::models::find_models_dir().is_none() {
        eprintln!("modèles absents : test ignoré (lancer scripts/fetch-models.ps1)");
        return None;
    }
    Some(Engine::load(None).expect("chargement des modèles"))
}

/// Copie un dossier de fixtures dans un dossier temporaire (les tests ne touchent pas aux originaux).
pub fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
        }
    }
}
