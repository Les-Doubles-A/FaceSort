//! Évaluation de la qualité et calibration des seuils (T11).
//!
//! Usage :
//!   cargo run --release --example eval -- <dossier> [--model sface|auraface|both] [--labels labels.csv]
//!
//! Étiquettes (qui est sur quelle photo), au choix :
//! - un sous-dossier par personne : `<dossier>\Lea\*.jpg`, `<dossier>\Tom\*.jpg`
//!   (photos où la personne est SEULE de préférence) ;
//! - `--labels labels.csv` (UTF-8, en-tête `photo,personnes`, prénoms séparés par `;`) ;
//! - sinon le préfixe du nom de fichier avant `_` (`lea_003.jpg`).
//!
//! Seules les photos à un seul visage et une seule personne servent aux mesures.
//! Rien n'est écrit dans ta base : l'analyse utilise une base temporaire.
//!
//! Mesures, par modèle :
//! 1. similarité des paires « même personne » / « personnes différentes »,
//!    séparément pour les visages de bonne et de mauvaise qualité ;
//! 2. les paires de personnes les plus confondues ;
//! 3. reconnaissance « une photo contre toutes les autres » : justes, fausses
//!    (= mélanges), non attribuées ;
//! 4. regroupement : groupes mélangés, groupes par personne, visages isolés,
//!    pour plusieurs seuils.

use facesort::cluster::{self, ClusterParams};
use facesort::embed::{EmbedderKind, cosine};
use facesort::engine::Engine;
use facesort::quality::is_good_score;
use facesort::recognize::{RecognizeParams, best_match};
use facesort::scan::{self, ScanOptions};
use facesort::store::Store;
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
            labels.insert(scan::absolute(&dir.join(&rec[0])), names.map(|n| n.to_lowercase()).collect());
        }
        return labels;
    }
    let (files, _, _) = scan::collect_files(&[dir.to_path_buf()]);
    let has_subdirs = files.iter().any(|f| f.path.parent() != Some(dir));
    for f in files {
        let label = if has_subdirs {
            f.path.parent().and_then(|p| p.strip_prefix(dir).ok()).and_then(|r| r.components().next()).map(
                |c| c.as_os_str().to_string_lossy().to_lowercase(),
            )
        } else {
            let stem = f.path.file_stem().unwrap().to_string_lossy().to_lowercase();
            stem.split_once('_').map(|(p, _)| p.to_string())
        };
        let set = label.filter(|l| l != "unknown" && l != "noface" && l != "inconnus").into_iter().collect();
        labels.insert(scan::absolute(&f.path), set);
    }
    labels
}

struct Sample {
    label: String,
    /// Photo d'origine : les variantes `nom__xxx.jpg` d'une même photo ne
    /// doivent pas se servir de référence entre elles (mesure trop optimiste).
    source: String,
    emb: Vec<f32>,
    good: bool,
    file: String,
}

fn stat(v: &[f32]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let pct = |p: f32| s[((s.len() - 1) as f32 * p) as usize];
    format!(
        "n={:<5} min {:.3}  5% {:.3}  médiane {:.3}  95% {:.3}  max {:.3}",
        s.len(),
        s[0],
        pct(0.05),
        pct(0.5),
        pct(0.95),
        s[s.len() - 1]
    )
}

fn evaluate(dir: &Path, labels: &Labels, kind: EmbedderKind) {
    println!("\n==================== {} ====================", kind.label());
    let mut engine = match Engine::load_with(None, kind) {
        Ok(e) => e,
        Err(e) => {
            println!("Modèle indisponible : {e}");
            return;
        }
    };
    let db = std::env::temp_dir().join(format!("facesort-eval-{}-{}.db", std::process::id(), kind.label()));
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", db.display()));
    }
    let mut store = Store::open(&db).expect("base temporaire");
    let cancel = AtomicBool::new(false);
    let mut no_cluster = ScanOptions::for_kind(kind);
    no_cluster.cluster.threshold = 2.0;
    let t = std::time::Instant::now();
    let report = scan::scan(&mut store, &mut engine, &[dir.to_path_buf()], Some(no_cluster), |_| {}, &cancel)
        .expect("analyse");
    println!("Analyse : {} photos, {} visages, {:.2} s/photo", report.analyzed, report.faces_found, t.elapsed().as_secs_f64() / report.analyzed.max(1) as f64);

    // Échantillons : photos à un visage et une personne.
    let faces = store.unassigned_embeddings().unwrap();
    let mut per_photo: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    let infos: Vec<_> = faces.iter().map(|f| store.face_info(f.id).unwrap().unwrap()).collect();
    for (i, info) in infos.iter().enumerate() {
        per_photo.entry(info.photo_path.clone()).or_default().push(i);
    }
    let mut samples = Vec::new();
    let mut sample_face_ids = Vec::new();
    for (path, idx) in &per_photo {
        if idx.len() == 1
            && let Some(ls) = labels.get(path)
            && ls.len() == 1
        {
            let f = &faces[idx[0]];
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            samples.push(Sample {
                source: stem.split("__").next().unwrap_or(&stem).to_string(),
                label: ls.iter().next().unwrap().clone(),
                emb: f.embedding.clone(),
                good: is_good_score(f.quality),
                file: path.file_name().unwrap().to_string_lossy().into_owned(),
            });
            sample_face_ids.push(f.id);
        }
    }
    let persons: BTreeSet<&str> = samples.iter().map(|s| s.label.as_str()).collect();
    let bad = samples.iter().filter(|s| !s.good).count();
    println!(
        "Échantillons : {} visages de {} personnes ({} de mauvaise qualité)",
        samples.len(),
        persons.len(),
        bad
    );
    if samples.len() < 4 {
        println!("Trop peu d'échantillons étiquetés pour mesurer.");
        return;
    }

    // 1. Paires
    let (mut same_g, mut same_b, mut diff_g, mut diff_b) = (vec![], vec![], vec![], vec![]);
    let mut confusions: BTreeMap<(String, String), (f32, String, String)> = BTreeMap::new();
    for i in 0..samples.len() {
        for j in i + 1..samples.len() {
            let (a, b) = (&samples[i], &samples[j]);
            if a.source == b.source {
                continue;
            }
            let s = cosine(&a.emb, &b.emb);
            let both_good = a.good && b.good;
            match (a.label == b.label, both_good) {
                (true, true) => same_g.push(s),
                (true, false) => same_b.push(s),
                (false, true) => diff_g.push(s),
                (false, false) => diff_b.push(s),
            }
            if a.label != b.label {
                let key = if a.label < b.label { (a.label.clone(), b.label.clone()) } else { (b.label.clone(), a.label.clone()) };
                let e = confusions.entry(key).or_insert((f32::MIN, String::new(), String::new()));
                if s > e.0 {
                    *e = (s, a.file.clone(), b.file.clone());
                }
            }
        }
    }
    println!("\n1. Similarité des paires");
    println!("  même personne, bonnes photos       : {}", stat(&same_g));
    println!("  même personne, photo(s) médiocre(s): {}", stat(&same_b));
    println!("  différentes, bonnes photos         : {}", stat(&diff_g));
    println!("  différentes, photo(s) médiocre(s)  : {}", stat(&diff_b));

    let mut conf: Vec<_> = confusions.into_iter().collect();
    conf.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    println!("\n2. Personnes les plus confondues (similarité maximale entre deux personnes différentes)");
    for ((a, b), (s, fa, fb)) in conf.iter().take(8) {
        println!("  {s:.3}  {a} / {b}   ({fa} / {fb})");
    }

    // 3. Reconnaissance une contre toutes
    let base = RecognizeParams::for_kind(kind);
    println!("\n3. Reconnaissance (chaque photo comparée à toutes les autres)");
    println!("  seuil   justes    FAUSSES   non attribuées   (seuil actuel du modèle : {:.2})", base.threshold);
    let mut sweep: Vec<f32> = (0..9).map(|k| base.threshold - 0.10 + 0.05 * k as f32).collect();
    sweep.dedup();
    for t in sweep {
        let p = RecognizeParams { threshold: t, ..base };
        let (mut ok, mut wrong, mut none) = (0, 0, 0);
        for (i, s) in samples.iter().enumerate() {
            let refs: Vec<(i64, Vec<f32>)> = samples
                .iter()
                .enumerate()
                .filter(|(j, o)| *j != i && o.good && o.source != s.source)
                .map(|(_, o)| (persons.iter().position(|x| *x == o.label).unwrap() as i64, o.emb.clone()))
                .collect();
            let pp = if s.good { p } else { RecognizeParams { threshold: p.threshold + p.low_quality_penalty, ..p } };
            match best_match(&s.emb, &refs, None, pp) {
                None => none += 1,
                Some(k) if persons.iter().nth(k as usize) == Some(&s.label.as_str()) => ok += 1,
                Some(_) => wrong += 1,
            }
        }
        let n = samples.len() as f32 / 100.0;
        println!(
            "  {t:.2}    {:>5.1} %   {:>5.1} %   {:>5.1} %",
            ok as f32 / n,
            wrong as f32 / n,
            none as f32 / n
        );
    }

    // 4. Regroupement sur les visages étiquetés
    let label_of: HashMap<i64, &str> =
        sample_face_ids.iter().zip(&samples).map(|(id, s)| (*id, s.label.as_str())).collect();
    let cp = ClusterParams::for_kind(kind);
    println!("\n4. Regroupement (seuil actuel du modèle : {:.2})", cp.threshold);
    println!("  seuil   groupes   MÉLANGÉS   visages en groupe mélangé   groupes/personne   isolés");
    for k in -2..=4 {
        let p = cp.stricter(0.05 * k as f32);
        cluster::recluster(&mut store, p).unwrap();
        let groups = store.groups().unwrap();
        let mut mixed = 0;
        let mut in_mixed = 0;
        let mut per_person: HashMap<&str, usize> = HashMap::new();
        for g in &groups {
            let ls: BTreeSet<&str> = g.face_ids.iter().filter_map(|f| label_of.get(f).copied()).collect();
            if ls.len() > 1 {
                mixed += 1;
                in_mixed += g.face_ids.iter().filter(|f| label_of.contains_key(f)).count();
            }
            for l in ls {
                *per_person.entry(l).or_default() += 1;
            }
        }
        let isolated = store.isolated_faces().unwrap().iter().filter(|f| label_of.contains_key(f)).count();
        let gpp = per_person.values().sum::<usize>() as f32 / persons.len() as f32;
        println!(
            "  {:.2}    {:>5}     {:>4}       {:>5.1} %                   {:>4.1}               {:>5.1} %",
            p.threshold,
            groups.len(),
            mixed,
            100.0 * in_mixed as f32 / samples.len() as f32,
            gpp,
            100.0 * isolated as f32 / samples.len() as f32
        );
    }
    drop(store);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", db.display()));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = None;
    let mut labels_csv = None;
    let mut models = vec![EmbedderKind::SFace, EmbedderKind::AuraFace];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--labels" => labels_csv = it.next().map(PathBuf::from),
            "--model" => {
                models = match it.next().map(String::as_str) {
                    Some("both") | None => vec![EmbedderKind::SFace, EmbedderKind::AuraFace],
                    Some(m) => vec![EmbedderKind::parse(m).expect("modèle : sface, auraface ou both")],
                }
            }
            _ => dir = Some(PathBuf::from(a)),
        }
    }
    let Some(dir) = dir else {
        eprintln!("usage : eval <dossier> [--model sface|auraface|both] [--labels labels.csv]");
        std::process::exit(2);
    };
    let dir = scan::absolute(&dir);
    let labels = load_labels(&dir, labels_csv.as_deref());
    let labelled = labels.values().filter(|l| !l.is_empty()).count();
    println!("{} photos étiquetées dans {}", labelled, dir.display());
    for kind in models {
        evaluate(&dir, &labels, kind);
    }
}
