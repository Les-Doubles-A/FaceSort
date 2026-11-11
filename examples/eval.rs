//! Évaluation de la qualité et calibration des seuils (T11).
//!
//! Usage :
//!   eval                         lit ta base FaceSort (copie temporaire, rien n'est modifié)
//!   eval --db <fichier.db>       lit une autre base
//!   eval <dossier>               dossier de test : un sous-dossier par personne
//!   options : --model sface|auraface|both   --labels labels.csv (avec <dossier>)
//!
//! Mode base (le plus simple) : les visages que tu as nommés dans l'onglet
//! Personnes servent d'étiquettes, et ceux que tu as retirés d'une personne
//! (« ce n'est pas elle ») sont des exemples réels de mélanges. Les empreintes
//! sont recalculées pour chaque modèle à partir des photos et des repères
//! enregistrés : les deux modèles sont comparés sur exactement les mêmes visages.
//!
//! Mesures, par modèle :
//! 1. similarité des paires « même personne » / « personnes différentes »,
//!    séparément pour les visages de bonne et de mauvaise qualité ;
//! 2. les paires de personnes les plus confondues ;
//! 3. reconnaissance « un visage contre tous les autres » : justes, fausses
//!    (= mélanges), non attribuées ;
//! 4. regroupement : groupes mélangés, groupes par personne, isolés ;
//! 5. (mode base) visages retirés : combien seraient encore attribués à tort.

use facesort::align::align_face;
use facesort::cluster::{ClusterParams, dbscan};
use facesort::embed::{EmbedderKind, cosine};
use facesort::engine::Engine;
use facesort::imageio;
use facesort::quality::FaceQuality;
use facesort::recognize::{RecognizeParams, best_match};
use facesort::scan;
use facesort::store::{self, LabelledFace, Store};
use image::RgbImage;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Visage prêt à être comparé : aligné une fois, empreinte recalculée par modèle.
struct Face {
    label: String,
    /// Photo d'origine : deux visages d'une même photo (ou ses variantes
    /// `nom__xxx.jpg`) ne se servent pas de référence entre eux.
    source: String,
    file: String,
    aligned: RgbImage,
    quality: FaceQuality,
}

struct Sample<'a> {
    face: &'a Face,
    emb: Vec<f32>,
    good: bool,
}

fn source_of(path: &Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let base = stem.split("__").next().unwrap_or(&stem).to_string();
    path.with_file_name(base).to_string_lossy().into_owned()
}

/// Aligne les visages enregistrés (photos décodées en parallèle).
fn align_stored(faces: &[LabelledFace]) -> (Vec<Face>, usize) {
    let mut by_photo: BTreeMap<PathBuf, Vec<&LabelledFace>> = BTreeMap::new();
    for f in faces {
        by_photo.entry(f.face.photo.clone()).or_default().push(f);
    }
    let groups: Vec<_> = by_photo.into_iter().collect();
    let out: Vec<Option<Vec<Face>>> = groups
        .par_iter()
        .map(|(path, fs)| {
            let img = imageio::decode_oriented(&std::fs::read(path).ok()?).ok()?;
            Some(
                fs.iter()
                    .map(|l| {
                        let aligned = align_face(&img, &l.face.landmarks);
                        let quality = FaceQuality::measure(&aligned, l.face.size, l.face.det_score);
                        Face {
                            label: l.person.to_lowercase(),
                            source: source_of(path),
                            file: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                            aligned,
                            quality,
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    let missing = groups.iter().zip(&out).filter(|(_, o)| o.is_none()).map(|((_, fs), _)| fs.len()).sum();
    (out.into_iter().flatten().flatten().collect(), missing)
}

fn copy_db(src: &Path) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("facesort-eval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("copie.db");
    for ext in ["", "-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{ext}", src.display()));
        if from.exists() {
            std::fs::copy(&from, PathBuf::from(format!("{}{ext}", dst.display()))).expect("copie de la base");
        }
    }
    dst
}

/// Mode base : visages nommés (étiquettes) et visages retirés (mélanges réels).
fn faces_from_db(db: &Path) -> (Vec<Face>, Vec<Face>) {
    if !db.exists() {
        eprintln!("Base introuvable : {}", db.display());
        std::process::exit(1);
    }
    let copy = copy_db(db);
    let store = Store::open(&copy).expect("lecture de la copie de la base");
    let confirmed = store.confirmed_faces().unwrap();
    let excluded = store.excluded_faces().unwrap();
    drop(store);
    let _ = std::fs::remove_dir_all(copy.parent().unwrap());
    let persons: BTreeSet<&str> = confirmed.iter().map(|f| f.person.as_str()).collect();
    println!(
        "Base {} : {} visages nommés pour {} personnes, {} visages retirés d'une personne",
        db.display(),
        confirmed.len(),
        persons.len(),
        excluded.len()
    );
    if persons.len() < 2 {
        println!(
            "\nIl faut au moins 2 personnes nommées pour mesurer. Dans l'onglet Personnes, nomme les groupes\n\
             (surtout les personnes qui se mélangent) et retire les visages mal classés, puis relance eval."
        );
        std::process::exit(0);
    }
    let (named, m1) = align_stored(&confirmed);
    let (removed, m2) = align_stored(&excluded);
    if m1 + m2 > 0 {
        println!("({} visage(s) ignoré(s) : photo déplacée ou illisible)", m1 + m2);
    }
    (named, removed)
}

/// Mode dossier : analyse dans une base temporaire, étiquettes par sous-dossier,
/// fichier CSV ou préfixe ; seules les photos à un visage et une personne comptent.
fn faces_from_folder(dir: &Path, csv_path: Option<&Path>) -> Vec<Face> {
    let mut labels: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    if let Some(p) = csv_path {
        let mut rdr = csv::Reader::from_path(p).expect("labels.csv illisible");
        for rec in rdr.records() {
            let rec = rec.expect("ligne CSV invalide");
            let names = rec.get(1).unwrap_or("").split(';').map(str::trim).filter(|s| !s.is_empty());
            labels.insert(scan::absolute(&dir.join(&rec[0])), names.map(|n| n.to_lowercase()).collect());
        }
    } else {
        let (files, _, _) = scan::collect_files(&[dir.to_path_buf()]);
        let has_subdirs = files.iter().any(|f| f.path.parent() != Some(dir));
        for f in files {
            let label = if has_subdirs {
                f.path
                    .parent()
                    .and_then(|p| p.strip_prefix(dir).ok())
                    .and_then(|r| r.components().next())
                    .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            } else {
                let stem = f.path.file_stem().unwrap().to_string_lossy().to_lowercase();
                stem.split_once('_').map(|(p, _)| p.to_string())
            };
            let set = label.filter(|l| l != "unknown" && l != "noface" && l != "inconnus").into_iter().collect();
            labels.insert(scan::absolute(&f.path), set);
        }
    }
    let db = std::env::temp_dir().join(format!("facesort-eval-dossier-{}.db", std::process::id()));
    let mut store = Store::open(&db).expect("base temporaire");
    let mut engine = Engine::load_with(None, EmbedderKind::SFace).expect("modèles");
    let mut opts = scan::ScanOptions::for_kind(EmbedderKind::SFace);
    opts.cluster.threshold = 2.0;
    opts.recognize.threshold = 2.0;
    let report =
        scan::scan(&mut store, &mut engine, &[dir.to_path_buf()], Some(opts), |_| {}, &AtomicBool::new(false))
            .expect("analyse");
    println!("Analyse de {} : {} photos, {} visages", dir.display(), report.analyzed, report.faces_found);
    // Visages par photo ; une photo à un seul visage et une seule personne étiquetée
    // devient un visage « nommé », exactement comme dans le mode base.
    let mut per_photo: BTreeMap<PathBuf, Vec<i64>> = BTreeMap::new();
    for f in store.unassigned_embeddings().unwrap() {
        let info = store.face_info(f.id).unwrap().unwrap();
        per_photo.entry(info.photo_path).or_default().push(f.id);
    }
    let mut person_ids: HashMap<String, i64> = HashMap::new();
    for (path, ids) in &per_photo {
        let Some(label) = labels.get(path).filter(|l| l.len() == 1).and_then(|l| l.iter().next()) else { continue };
        if ids.len() != 1 {
            continue;
        }
        let pid = match person_ids.get(label) {
            Some(&p) => p,
            None => {
                let p = store.create_person(label).expect("étiquette invalide comme nom");
                person_ids.insert(label.clone(), p);
                p
            }
        };
        store.assign_faces(ids, pid, facesort::store::Assignment::Manual).unwrap();
    }
    let confirmed = store.confirmed_faces().unwrap();
    drop(store);
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", db.display()));
    }
    align_stored(&confirmed).0
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

fn embed_all<'a>(engine: &mut Engine, faces: &'a [Face]) -> Vec<Sample<'a>> {
    faces
        .iter()
        .map(|f| Sample { face: f, emb: engine.embedder.embed(&f.aligned).expect("empreinte"), good: f.quality.is_good() })
        .collect()
}

fn evaluate(kind: EmbedderKind, named: &[Face], removed: &[Face]) {
    println!("\n==================== {} ====================", kind.label());
    let mut engine = match Engine::load_with(None, kind) {
        Ok(e) => e,
        Err(e) => {
            println!("Modèle indisponible : {e}");
            return;
        }
    };
    let t = std::time::Instant::now();
    let samples = embed_all(&mut engine, named);
    let negatives = embed_all(&mut engine, removed);
    println!(
        "{} visages comparés ({} de mauvaise qualité), {:.0} ms par visage",
        samples.len(),
        samples.iter().filter(|s| !s.good).count(),
        t.elapsed().as_secs_f64() * 1000.0 / (samples.len() + negatives.len()).max(1) as f64
    );
    let persons: Vec<String> =
        samples.iter().map(|s| s.face.label.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let pid = |l: &str| persons.iter().position(|p| p == l).unwrap() as i64;

    // 1. Paires
    let (mut same_g, mut same_b, mut diff_g, mut diff_b) = (vec![], vec![], vec![], vec![]);
    let mut confusions: BTreeMap<(String, String), (f32, String, String)> = BTreeMap::new();
    for i in 0..samples.len() {
        for j in i + 1..samples.len() {
            let (a, b) = (&samples[i], &samples[j]);
            if a.face.source == b.face.source {
                continue;
            }
            let s = cosine(&a.emb, &b.emb);
            match (a.face.label == b.face.label, a.good && b.good) {
                (true, true) => same_g.push(s),
                (true, false) => same_b.push(s),
                (false, true) => diff_g.push(s),
                (false, false) => diff_b.push(s),
            }
            if a.face.label != b.face.label {
                let (x, y) = if a.face.label < b.face.label { (a, b) } else { (b, a) };
                let e = confusions
                    .entry((x.face.label.clone(), y.face.label.clone()))
                    .or_insert((f32::MIN, String::new(), String::new()));
                if s > e.0 {
                    *e = (s, x.face.file.clone(), y.face.file.clone());
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
    println!("\n2. Personnes les plus confondues (plus forte similarité entre deux personnes différentes)");
    for ((a, b), (s, fa, fb)) in conf.iter().take(10) {
        println!("  {s:.3}  {a} / {b}   ({fa} / {fb})");
    }

    // 3. Reconnaissance un contre tous
    let base = RecognizeParams::for_kind(kind);
    let refs_for = |skip_source: &str| -> Vec<(i64, Vec<f32>)> {
        samples
            .iter()
            .filter(|o| o.good && o.face.source != skip_source)
            .map(|o| (pid(&o.face.label), o.emb.clone()))
            .collect()
    };
    println!("\n3. Reconnaissance (chaque visage comparé à tous les autres)");
    println!("  seuil   justes    FAUSSES   non attribuées   (seuil actuel du modèle : {:.2})", base.threshold);
    for k in 0..9 {
        let t = base.threshold - 0.10 + 0.05 * k as f32;
        let p = RecognizeParams { threshold: t, ..base };
        let (mut ok, mut wrong, mut none) = (0, 0, 0);
        for s in &samples {
            let pp = if s.good { p } else { RecognizeParams { threshold: t + p.low_quality_penalty, ..p } };
            match best_match(&s.emb, &refs_for(&s.face.source), None, pp) {
                None => none += 1,
                Some(id) if id == pid(&s.face.label) => ok += 1,
                Some(_) => wrong += 1,
            }
        }
        let n = samples.len().max(1) as f32 / 100.0;
        let mark = if (t - base.threshold).abs() < 1e-3 { "  <- actuel" } else { "" };
        println!(
            "  {t:.2}    {:>5.1} %   {:>5.1} %   {:>5.1} %{mark}",
            ok as f32 / n,
            wrong as f32 / n,
            none as f32 / n
        );
    }

    // 4. Regroupement
    let cp = ClusterParams::for_kind(kind);
    let embs: Vec<Vec<f32>> = samples.iter().map(|s| s.emb.clone()).collect();
    let good: Vec<bool> = samples.iter().map(|s| s.good).collect();
    println!("\n4. Regroupement (seuil actuel du modèle : {:.2})", cp.threshold);
    println!("  seuil   groupes   MÉLANGÉS   visages en groupe mélangé   groupes/personne   isolés");
    for k in -2..=4 {
        let p = cp.stricter(0.05 * k as f32);
        let labels = dbscan(&embs, &good, p);
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, l) in labels.iter().enumerate() {
            if let Some(l) = l {
                groups.entry(*l).or_default().push(i);
            }
        }
        let (mut mixed, mut in_mixed) = (0, 0);
        let mut per_person: HashMap<&str, usize> = HashMap::new();
        for members in groups.values() {
            let ls: BTreeSet<&str> = members.iter().map(|&i| samples[i].face.label.as_str()).collect();
            if ls.len() > 1 {
                mixed += 1;
                in_mixed += members.len();
            }
            for l in ls {
                *per_person.entry(l).or_default() += 1;
            }
        }
        let isolated = labels.iter().filter(|l| l.is_none()).count();
        let mark = if k == 0 { "  <- actuel" } else { "" };
        println!(
            "  {:.2}    {:>5}     {:>4}       {:>5.1} %                   {:>4.1}               {:>5.1} %{mark}",
            p.threshold,
            groups.len(),
            mixed,
            100.0 * in_mixed as f32 / samples.len().max(1) as f32,
            per_person.values().sum::<usize>() as f32 / persons.len().max(1) as f32,
            100.0 * isolated as f32 / samples.len().max(1) as f32
        );
    }

    // 5. Visages retirés par l'utilisateur
    if !negatives.is_empty() {
        let refs: Vec<(i64, Vec<f32>)> = samples.iter().filter(|o| o.good).map(|o| (pid(&o.face.label), o.emb.clone())).collect();
        let mut still_wrong = 0;
        let mut worst: Vec<(f32, String, String)> = Vec::new();
        for n in &negatives {
            let Some(target) = persons.iter().position(|p| *p == n.face.label) else { continue };
            let p = if n.good { base } else { RecognizeParams { threshold: base.threshold + base.low_quality_penalty, ..base } };
            if best_match(&n.emb, &refs, None, p) == Some(target as i64) {
                still_wrong += 1;
            }
            let best = refs.iter().filter(|(id, _)| *id == target as i64).map(|(_, e)| cosine(&n.emb, e)).fold(f32::MIN, f32::max);
            worst.push((best, n.face.label.clone(), n.face.file.clone()));
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("\n5. Visages que tu as retirés d'une personne ({})", negatives.len());
        println!("  encore attribués à tort avec les seuils actuels : {still_wrong}");
        for (s, who, file) in worst.iter().take(5) {
            println!("  {s:.3}  ressemblance avec {who}  ({file})");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir = None;
    let mut db = None;
    let mut labels_csv = None;
    let mut models = vec![EmbedderKind::SFace, EmbedderKind::AuraFace];
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--labels" => labels_csv = it.next().map(PathBuf::from),
            "--db" => db = Some(it.next().map(PathBuf::from).unwrap_or_else(store::default_db_path)),
            "--model" => {
                models = match it.next().map(String::as_str) {
                    Some("both") | None => vec![EmbedderKind::SFace, EmbedderKind::AuraFace],
                    Some(m) => vec![EmbedderKind::parse(m).expect("modèle : sface, auraface ou both")],
                }
            }
            "-h" | "--help" => {
                println!("eval [--db fichier.db] | eval <dossier> [--labels labels.csv]   [--model sface|auraface|both]");
                return;
            }
            _ => dir = Some(PathBuf::from(a)),
        }
    }
    let (named, removed) = match dir {
        Some(d) => (faces_from_folder(&scan::absolute(&d), labels_csv.as_deref()), Vec::new()),
        None => faces_from_db(&db.unwrap_or_else(store::default_db_path)),
    };
    if named.len() < 4 {
        println!("Trop peu de visages étiquetés pour mesurer ({}).", named.len());
        return;
    }
    for kind in models {
        evaluate(kind, &named, &removed);
    }
}
