//! Parcours P1/P2 de bout en bout avec les vrais modèles, sans interface :
//! les mêmes appels que la GUI (écrans Analyse, Personnes, Tri).

mod common;

use facesort::scan::{self, ScanOptions};
use facesort::sort::{self, SortOptions};
use facesort::store::{NameOutcome, Store};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn prefix(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().split('_').next().unwrap().to_string()
}

/// Personne réelle (préfixe du fichier) de chaque visage d'un groupe.
fn group_truth(s: &Store, faces: &[i64]) -> Vec<String> {
    faces.iter().map(|f| prefix(&s.face_info(*f).unwrap().unwrap().photo_path)).collect()
}

// AC-F1-1, AC-F1-2, AC-F1-3, AC-F1-4, AC-F6-2 (code 3)
#[test]
fn scan_basic_rescan_truncated_and_ignored() {
    let Some(mut engine) = common::engine() else { return };
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("photos");
    common::copy_dir(&common::fixtures().join("basic"), &src);
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(src.join("expected.json")).unwrap()).unwrap();
    let total = expected["total_faces"].as_u64().unwrap() as i64;
    let mut s = Store::open(&d.path().join("t.db")).unwrap();
    let cancel = AtomicBool::new(false);

    let r = scan::scan(&mut s, &mut engine, std::slice::from_ref(&src), ScanOptions::default(), |_| {}, &cancel).unwrap();
    assert_eq!(r.analyzed, 10);
    assert!((r.faces_found as i64 - total).abs() <= 1, "{} visages au lieu de {total}", r.faces_found);
    assert!(r.summary().starts_with("10 photos analysées"), "{}", r.summary());
    assert_eq!(r.ignored.get(".json"), Some(&1));

    let r2 = scan::scan(&mut s, &mut engine, std::slice::from_ref(&src), ScanOptions::default(), |_| {}, &cancel).unwrap();
    assert!(r2.summary().starts_with("0 photo analysée, 10 inchangées"), "{}", r2.summary());

    // JPEG tronqué + extension ignorée
    let bytes = std::fs::read(src.join("biden_13.jpg")).unwrap();
    std::fs::write(src.join("tronque.jpg"), &bytes[..bytes.len() / 2]).unwrap();
    std::fs::write(src.join("IMG_1.heic"), b"x").unwrap();
    let r3 = scan::scan(&mut s, &mut engine, std::slice::from_ref(&src), ScanOptions::default(), |_| {}, &cancel).unwrap();
    assert_eq!(r3.analyzed, 0);
    assert_eq!(r3.errors.len(), 1);
    assert!(r3.errors[0].0.ends_with("tronque.jpg"));
    assert!(r3.errors[0].1.contains("image illisible"), "{}", r3.errors[0].1);
    assert!(r3.ignored_summary().contains("1 .heic"));

    // Code de sortie 3 côté CLI
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_facesort-cli"))
        .arg("--db")
        .arg(d.path().join("cli.db"))
        .args(["scan", "--quiet"])
        .arg(&src)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", String::from_utf8_lossy(&out.stderr));
}

// AC-F1-5
#[test]
fn exif_orientation_detects_same_faces() {
    let Some(mut engine) = common::engine() else { return };
    let dir = common::fixtures().join("exif");
    let load = |f: &str| facesort::imageio::decode_oriented(&std::fs::read(dir.join(f)).unwrap()).unwrap();
    let up = engine.analyze(&load("upright.jpg")).unwrap();
    let rot = engine.analyze(&load("orientation6.jpg")).unwrap();
    assert!(!up.is_empty());
    assert_eq!(up.len(), rot.len());
}

// P1 + P2 : AC-F2-1, AC-F2-2, AC-F4-1, AC-F4-2, puis tri réel (AC-F5-2)
#[test]
fn full_journey_people3() {
    let Some(mut engine) = common::engine() else { return };
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    common::copy_dir(&common::fixtures().join("people3"), &a);
    common::copy_dir(&common::fixtures().join("people3_new"), &b);
    let mut s = Store::open(&d.path().join("t.db")).unwrap();
    let cancel = AtomicBool::new(false);

    let r = scan::scan(&mut s, &mut engine, std::slice::from_ref(&a), ScanOptions::default(), |_| {}, &cancel).unwrap();
    assert_eq!(r.faces_found, 18);
    let groups = s.groups().unwrap();
    let mut groups_per_person: HashMap<String, usize> = HashMap::new();
    for g in &groups {
        let truth = group_truth(&s, &g.face_ids);
        let set: BTreeSet<_> = truth.iter().collect();
        assert_eq!(set.len(), 1, "groupe impur : {truth:?}"); // AC-F2-1
        *groups_per_person.entry(truth[0].clone()).or_default() += 1;
    }
    for person in ["obama", "biden", "harris"] {
        let n = groups_per_person.get(person).copied().unwrap_or(0);
        assert!((1..=2).contains(&n), "{person} réparti en {n} groupes"); // AC-F2-2
    }

    // Nommage comme dans l'écran Personnes : le premier groupe de chaque personne
    // est nommé, les suivants sont ajoutés via « Existe déjà -> Ajouter ».
    for g in s.groups().unwrap() {
        let who = group_truth(&s, &g.face_ids)[0].clone();
        let name = match who.as_str() {
            "obama" => "Barack",
            "biden" => "Joe",
            _ => "Kamala",
        };
        match s.name_group(g.cluster_id, name).unwrap() {
            NameOutcome::Created(_) => {}
            NameOutcome::Exists { id, .. } => {
                s.add_group_to_person(g.cluster_id, id).unwrap();
            }
            NameOutcome::Invalid(m) => panic!("{m}"),
        }
    }
    assert_eq!(s.stats().unwrap().persons, 3);
    assert_eq!(s.stats().unwrap().unassigned_faces, 0);

    // P2 : nouveau dossier, reconnaissance automatique
    let r = scan::scan(&mut s, &mut engine, std::slice::from_ref(&b), ScanOptions::default(), |_| {}, &cancel).unwrap();
    assert_eq!(r.faces_found, 4);
    let barack = s.person_by_name("Barack").unwrap().unwrap().0;
    let mut auto_barack = 0;
    for f in s.person_faces(barack).unwrap() {
        let info = s.face_info(f).unwrap().unwrap();
        if info.photo_path.starts_with(scan::absolute(&b)) {
            assert_eq!(info.assignment, facesort::store::Assignment::Auto);
            auto_barack += 1;
        }
    }
    assert_eq!(auto_barack, 3, "AC-F4-1 : 3 nouvelles photos d'Obama reconnues"); // AC-F4-1
    let unknown: Vec<PathBuf> = s
        .photos_for_sort()
        .unwrap()
        .into_iter()
        .filter(|p| p.path.file_name().unwrap().to_string_lossy().starts_with("unknown"))
        .flat_map(|p| {
            assert!(p.persons.is_empty(), "AC-F4-2 : inconnu attribué à {:?}", p.persons);
            assert!(p.has_unknown);
            Some(p.path)
        })
        .collect();
    assert_eq!(unknown.len(), 1);

    // Tri comme dans l'écran Tri
    let out = d.path().join("tri");
    let plan = sort::plan(&s.photos_for_sort().unwrap(), &s.roots().unwrap(), &SortOptions { dest: out.clone(), include_unknown: true })
        .unwrap();
    assert_eq!(plan.per_person["Barack"].copies, 9);
    assert_eq!(plan.per_person["Joe"].copies, 6);
    assert_eq!(plan.per_person["Kamala"].copies, 6);
    assert_eq!(plan.per_person["_Inconnus"].copies, 1);
    let rep = sort::execute(&plan, |_, _| {}, &cancel);
    assert!(rep.stopped_on_error.is_none());
    assert_eq!(rep.copied, 22);
    let barack_dir: Vec<_> = std::fs::read_dir(out.join("Barack")).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(barack_dir.len(), 9);
    assert!(barack_dir.iter().all(|n| n.to_string_lossy().starts_with("obama")));
}
