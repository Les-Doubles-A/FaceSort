//! Tests du binaire `facesort-cli` (F6).

use facesort::detect::Detection;
use facesort::engine::AnalyzedFace;
use facesort::store::Store;
use std::path::Path;
use std::process::Command;

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_facesort-cli"))
}

fn face(v: f32) -> AnalyzedFace {
    AnalyzedFace {
        detection: Detection { bbox: [0.0, 0.0, 60.0, 60.0], landmarks: [[0.0; 2]; 5], score: 0.95 },
        embedding: vec![v, 1.0],
        quality: Default::default(),
        thumb_jpeg: vec![],
    }
}

// AC-F6-1
#[test]
fn help() {
    let out = cli().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    for cmd in ["scan", "sort", "status", "Exemples"] {
        assert!(text.contains(cmd), "{cmd} absent de --help");
    }
    for sub in ["scan", "sort", "status"] {
        let out = cli().args([sub, "--help"]).output().unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("Exemple"), "{sub} --help sans exemple");
    }
}

// AC-F6-2 (cas sans modèle : 1 et 2 ; le code 3 est couvert par tests/scan.rs)
#[test]
fn exit_codes() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("t.db");
    let out = cli().args(["--db"]).arg(&db).args(["scan", "Z:/n/existe/pas"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("dossier introuvable"));
    let out = cli().arg("commande-inconnue").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = cli().arg("--db").arg(&db).arg("status").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Base vide"));
}

// AC-F6-3
#[test]
fn status_json() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("t.db");
    let out = cli().arg("--db").arg(&db).args(["status", "--json"]).output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON valide");
    for k in ["persons", "faces", "photos", "unassigned_faces"] {
        assert!(v.get(k).is_some(), "clé {k} absente");
    }
}

// AC-F6-4 : la GUI et la CLI partagent la base.
#[test]
fn status_sees_named_person() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("t.db");
    {
        let mut s = Store::open(&db).unwrap();
        s.save_photo(Path::new("a.jpg"), 1, 1, "x", (100, 100), &[face(0.0), face(1.0)]).unwrap();
        s.set_clusters(&[(1, Some(0)), (2, Some(0))]).unwrap();
        let facesort::store::NameOutcome::Created(id) = s.name_group(0, "Léa").unwrap() else { panic!() };
        assert_eq!(s.person_faces(id).unwrap().len(), 2);
    }
    let out = cli().arg("--db").arg(&db).args(["status", "--json"]).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["persons"], 1);
    assert_eq!(v["people"][0]["name"], "Léa");
    assert_eq!(v["people"][0]["faces"], 2);
}
