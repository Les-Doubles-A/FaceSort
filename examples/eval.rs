//! Évaluation de la qualité et calibration des seuils (T11).
//!
//! Usage :
//!   cargo run --release --example eval -- <dossier> [labels.csv]
//!
//! `labels.csv` (UTF-8, en-tête `photo,personnes`) : chemin relatif de la photo
//! et prénoms séparés par `;` (vide si personne de connu). Sans fichier, le
//! prénom est le préfixe du nom de fichier avant `_` (ex. `lea_003.jpg`), et
//! seules les photos à un seul visage servent aux mesures par paires.
//!
//! Mesures :
//! 1. paires de visages (photos à visage unique) : taux de vrais et faux
//!    appariements selon le seuil ;
//! 2. chaîne complète : analyse, regroupement, nommage « oracle » de chaque
//!    groupe par vote majoritaire, reconnaissance, puis précision et rappel
//!    des copies (photo, personne).

use facesort::cluster::ClusterParams;
use facesort::embed::cosine;
use facesort::engine::Engine;
use facesort::recognize::RecognizeParams;
use facesort::scan::{self, ScanOptions};
use facesort::store::{NameOutcome, Store};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

type Labels = BTreeMap<PathBuf, BTreeSet<String>>;

fn load_labels(dir: &Path, csv_path: Option<&Path>) -> Labels {
    let mut labels = Labels::new();
    if let Some(p) = csv_path {
        let mut rdr = csv::Reader::from_path(p).expect("labels.csv illisible");
        for rec in rdr.records() {
            let rec = rec.expect("ligne CSV invalide");
            let names = rec.get(1).unwrap_or("").split(';').map(str::trim).filter(|s| !s.is_empty());
            labels.insert(scan::absolute(&dir.join(&rec[0])), names.map(String::from).collect());
        }
    } else {
        let (files, _, _) = scan::collect_files(&[dir.to_path_buf()]);
        for f in files {
            let stem = f.path.file_stem().unwrap().to_string_lossy().to_string();
            if let Some((prefix, _)) = stem.split_once('_')
                && !prefix.eq_ignore_ascii_case("unknown")
                && !prefix.eq_ignore_ascii_case("noface")
            {
                labels.insert(scan::absolute(&f.path), BTreeSet::from([prefix.to_lowercase()]));
            } else {
                labels.insert(scan::absolute(&f.path), BTreeSet::new());
            }
        }
    }
    labels
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first().map(PathBuf::from) else {
        eprintln!("usage : eval <dossier> [labels.csv]");
        std::process::exit(2);
    };
    let labels = load_labels(&dir, args.get(1).map(Path::new));
    let tmp = std::env::temp_dir().join(format!("facesort-eval-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut store = Store::open(&tmp).expect("base temporaire");
    let mut engine = Engine::load(None).expect("modèles");
    let cancel = AtomicBool::new(false);

    // Analyse sans regroupement ni reconnaissance (seuils rendus inopérants).
    let no_cluster = ScanOptions {
        cluster: ClusterParams { threshold: 2.0, min_samples: 2 },
        recognize: RecognizeParams::default(),
    };
    let report = scan::scan(&mut store, &mut engine, std::slice::from_ref(&dir), no_cluster, |_| {}, &cancel).expect("analyse");
    println!("Analyse : {}", report.summary());

    // 1. Paires
    let faces = store.unassigned_embeddings().unwrap();
    let mut per_photo: HashMap<PathBuf, Vec<Vec<f32>>> = HashMap::new();
    for (id, emb) in &faces {
        let info = store.face_info(*id).unwrap().unwrap();
        per_photo.entry(info.photo_path).or_default().push(emb.clone());
    }
    let singles: Vec<(String, Vec<f32>)> = per_photo
        .iter()
        .filter(|(p, v)| v.len() == 1 && labels.get(*p).is_some_and(|l| l.len() == 1))
        .map(|(p, v)| (labels[p].iter().next().unwrap().clone(), v[0].clone()))
        .collect();
    let (mut same, mut diff) = (Vec::new(), Vec::new());
    for i in 0..singles.len() {
        for j in i + 1..singles.len() {
            let s = cosine(&singles[i].1, &singles[j].1);
            if singles[i].0 == singles[j].0 { same.push(s) } else { diff.push(s) }
        }
    }
    println!("\nPaires : {} même personne, {} personnes différentes", same.len(), diff.len());
    let stat = |v: &[f32]| {
        if v.is_empty() {
            return "-".to_string();
        }
        let min = v.iter().copied().fold(f32::MAX, f32::min);
        let max = v.iter().copied().fold(f32::MIN, f32::max);
        format!("min {min:.3}  moy {:.3}  max {max:.3}", v.iter().sum::<f32>() / v.len() as f32)
    };
    println!("  même personne       : {}", stat(&same));
    println!("  personnes différentes : {}", stat(&diff));
    println!("  seuil   vrais appariements   faux appariements");
    for t in [0.30f32, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60] {
        let tp = same.iter().filter(|&&s| s >= t).count() as f32 / same.len().max(1) as f32;
        let fp = diff.iter().filter(|&&s| s >= t).count() as f32 / diff.len().max(1) as f32;
        println!("  {t:.2}    {:>6.1} %              {:>6.2} %", tp * 100.0, fp * 100.0);
    }

    // 2. Chaîne complète avec nommage oracle
    let truth_pairs: BTreeSet<(PathBuf, String)> =
        labels.iter().flat_map(|(p, ls)| ls.iter().map(move |l| (p.clone(), l.clone()))).collect();
    for (ct, rt) in [(0.40, 0.36), (0.45, 0.40), (0.50, 0.45), (0.55, 0.50)] {
        let db = std::env::temp_dir().join(format!("facesort-eval-{}-{ct}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::fs::copy(&tmp, &db).unwrap();
        let mut s = Store::open(&db).unwrap();
        let opts = ScanOptions {
            cluster: ClusterParams { threshold: ct, min_samples: 2 },
            recognize: RecognizeParams { threshold: rt, margin: 0.05 },
        };
        scan::refresh_groups(&mut s, opts).unwrap();
        let mut named_groups = 0;
        for g in s.groups().unwrap() {
            let mut votes: BTreeMap<String, usize> = BTreeMap::new();
            for f in &g.face_ids {
                let path = s.face_info(*f).unwrap().unwrap().photo_path;
                for l in labels.get(&path).into_iter().flatten() {
                    *votes.entry(l.clone()).or_default() += 1;
                }
            }
            if let Some((name, n)) = votes.iter().max_by_key(|(_, n)| **n)
                && *n * 2 > g.face_ids.len()
            {
                match s.name_group(g.cluster_id, name).unwrap() {
                    NameOutcome::Exists { id, .. } => {
                        s.add_group_to_person(g.cluster_id, id).unwrap();
                    }
                    NameOutcome::Created(_) => {}
                    NameOutcome::Invalid(m) => eprintln!("{m}"),
                }
                named_groups += 1;
            }
        }
        let (auto, _) = scan::refresh_groups(&mut s, opts).unwrap();
        let predicted: BTreeSet<(PathBuf, String)> = s
            .photos_for_sort()
            .unwrap()
            .into_iter()
            .flat_map(|p| p.persons.into_iter().map(move |n| (p.path.clone(), n.to_lowercase())))
            .collect();
        let named: BTreeSet<String> = s.persons().unwrap().into_iter().map(|p| p.name.to_lowercase()).collect();
        let truth_named: BTreeSet<_> = truth_pairs.iter().filter(|(_, n)| named.contains(n)).cloned().collect();
        let correct = predicted.intersection(&truth_named).count();
        let precision = correct as f32 / predicted.len().max(1) as f32;
        let recall = correct as f32 / truth_named.len().max(1) as f32;
        let left = s.stats().unwrap();
        println!(
            "\nRegroupement {ct:.2} / reconnaissance {rt:.2} : {named_groups} groupe(s) nommé(s), {auto} attribution(s) auto, \
             {} groupe(s) restant(s), {} visage(s) non attribué(s)\n  précision {:.1} %  rappel {:.1} %  ({correct}/{} copies justes, {} attendues)",
            left.groups,
            left.unassigned_faces,
            precision * 100.0,
            recall * 100.0,
            predicted.len(),
            truth_named.len()
        );
        drop(s);
        let _ = std::fs::remove_file(&db);
    }
    drop(store);
    let _ = std::fs::remove_file(&tmp);
}
