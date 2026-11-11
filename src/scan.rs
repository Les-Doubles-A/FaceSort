//! Analyse d'un ou plusieurs dossiers (F1), puis reconnaissance (F4) et regroupement (F2).

use crate::cluster::{self, ClusterParams};
use crate::engine::Engine;
use crate::recognize::{self, RecognizeParams};
use crate::store::Store;
use crate::{Result, imageio, util};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Nombre de photos décodées en parallèle avant l'inférence.
const BATCH: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScanOptions {
    pub recognize: RecognizeParams,
    pub cluster: ClusterParams,
}

impl ScanOptions {
    /// Seuils calibrés du modèle de reconnaissance donné.
    pub fn for_kind(kind: crate::embed::EmbedderKind) -> Self {
        Self { recognize: RecognizeParams::for_kind(kind), cluster: ClusterParams::for_kind(kind) }
    }

    /// Seuils du modèle qui a rempli la base (défaut si la base est vide).
    pub fn for_store(store: &Store) -> Result<Self> {
        Ok(Self::for_kind(store.embedder_kind()?.unwrap_or(crate::models::DEFAULT_EMBEDDER)))
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Progress {
    pub total: usize,
    pub done: usize,
    pub faces: usize,
    pub errors: usize,
    pub current: Option<PathBuf>,
}

impl Progress {
    /// Avancement entre 0 et 1 (1 si rien à analyser).
    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 1.0 } else { (self.done as f32 / self.total as f32).clamp(0.0, 1.0) }
    }

    /// Temps restant estimé au rythme moyen observé (`None` avant la première photo).
    pub fn eta(&self, elapsed: Duration) -> Option<Duration> {
        if self.done == 0 || self.done >= self.total {
            return None;
        }
        let per = elapsed.as_secs_f64() / self.done as f64;
        Some(Duration::from_secs_f64(per * (self.total - self.done) as f64))
    }

    /// « 120/1 240 photos, 230 visages, 2 erreurs ».
    pub fn label(&self) -> String {
        format!("{}/{} photos, {} visages, {} erreurs", self.done, self.total, self.faces, self.errors)
    }
}

/// Durée lisible : « 45 s », « 3 min 05 s », « 1 h 12 min ».
pub fn human_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ScanReport {
    pub analyzed: usize,
    pub unchanged: usize,
    pub faces_found: usize,
    pub errors: Vec<(PathBuf, String)>,
    /// Extension (minuscules, avec le point) -> nombre de fichiers ignorés.
    pub ignored: BTreeMap<String, usize>,
    pub auto_assigned: usize,
    pub groups: usize,
    /// Visages d'analyses antérieures dont la note de qualité vient d'être calculée.
    pub quality_backfilled: usize,
    /// Attributions automatiques réévaluées après un changement de seuils.
    pub auto_reset: usize,
    pub cancelled: bool,
    pub elapsed: Duration,
}

impl ScanReport {
    pub fn seconds_per_photo(&self) -> f64 {
        if self.analyzed == 0 { 0.0 } else { self.elapsed.as_secs_f64() / self.analyzed as f64 }
    }

    /// « ignorés : 34 .heic, 2 .mp4 » (AC-F1-4).
    pub fn ignored_summary(&self) -> String {
        if self.ignored.is_empty() {
            return "ignorés : aucun".into();
        }
        let mut v: Vec<_> = self.ignored.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        format!("ignorés : {}", v.iter().map(|(e, n)| format!("{n} {e}")).collect::<Vec<_>>().join(", "))
    }

    pub fn summary(&self) -> String {
        format!(
            "{} photo{} analysée{}, {} inchangée{}, {} visage{}, {} erreur{}, {} attribué{} automatiquement, {} groupe{} à nommer, {:.2} s/photo ; {}",
            self.analyzed,
            plural(self.analyzed),
            plural(self.analyzed),
            self.unchanged,
            plural(self.unchanged),
            self.faces_found,
            plural(self.faces_found),
            self.errors.len(),
            plural(self.errors.len()),
            self.auto_assigned,
            plural(self.auto_assigned),
            self.groups,
            plural(self.groups),
            self.seconds_per_photo(),
            self.ignored_summary()
        )
    }
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: i64,
}

/// Fichier et raison de l'erreur.
pub type FileErrors = Vec<(PathBuf, String)>;

/// Parcours récursif : fichiers pris en charge, extensions ignorées, erreurs d'accès.
pub fn collect_files(roots: &[PathBuf]) -> (Vec<FileEntry>, BTreeMap<String, usize>, FileErrors) {
    let mut files = Vec::new();
    let mut ignored = BTreeMap::new();
    let mut errors = Vec::new();
    for root in roots {
        if !root.is_dir() {
            errors.push((root.clone(), "dossier introuvable ou inaccessible".into()));
            continue;
        }
        for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    errors.push((e.path().map(Path::to_path_buf).unwrap_or_default(), format!("accès refusé ({e})")));
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if imageio::is_supported(path) {
                match entry.metadata() {
                    Ok(m) => files.push(FileEntry { path: path.to_path_buf(), size: m.len(), mtime: util::mtime_nanos(&m) }),
                    Err(e) => errors.push((path.to_path_buf(), format!("métadonnées illisibles ({e})"))),
                }
            } else {
                let ext = path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                    .unwrap_or_else(|| "(sans extension)".into());
                *ignored.entry(ext).or_insert(0) += 1;
            }
        }
    }
    (files, ignored, errors)
}

/// Chemin absolu sans préfixe `\\?\` (pour des chemins lisibles et stables).
pub fn absolute(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let s = abs.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => abs,
    }
}

struct Decoded {
    entry: FileEntry,
    result: std::result::Result<(image::RgbImage, String), String>,
}

fn decode(entry: FileEntry) -> Decoded {
    let result = std::fs::read(&entry.path)
        .map_err(|e| format!("lecture impossible ({e})"))
        .and_then(|bytes| {
            let sha = util::sha256_bytes(&bytes);
            imageio::decode_oriented(&bytes).map(|img| (img, sha)).map_err(|e| format!("image illisible ({e})"))
        });
    Decoded { entry, result }
}

/// Analyse les dossiers, enregistre les visages, attribue les personnes connues
/// et regroupe le reste. `progress` est appelé après chaque photo.
pub fn scan(
    store: &mut Store,
    engine: &mut Engine,
    roots: &[PathBuf],
    opts: Option<ScanOptions>,
    mut progress: impl FnMut(&Progress),
    cancel: &AtomicBool,
) -> Result<ScanReport> {
    let start = Instant::now();
    store.check_models(&engine.ids)?;
    let opts = opts.unwrap_or_else(|| ScanOptions::for_kind(engine.kind));
    let quality_backfilled = backfill_quality(store)?;
    let roots: Vec<PathBuf> = roots.iter().map(|r| absolute(r)).collect();
    let (files, ignored, mut errors) = collect_files(&roots);
    for r in &roots {
        if r.is_dir() {
            store.add_root(r)?;
        }
    }
    let mut report = ScanReport { ignored, quality_backfilled, ..Default::default() };
    let mut todo = Vec::new();
    for f in files {
        if store.is_unchanged(&f.path, f.size, f.mtime)? {
            report.unchanged += 1;
        } else {
            todo.push(f);
        }
    }
    let mut prog = Progress { total: todo.len(), errors: errors.len(), ..Default::default() };
    progress(&prog);

    let mut pending = todo.into_iter();
    loop {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let batch: Vec<FileEntry> = pending.by_ref().take(BATCH).collect();
        if batch.is_empty() {
            break;
        }
        let decoded: Vec<Decoded> = batch.into_par_iter().map(decode).collect();
        for d in decoded {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            prog.current = Some(d.entry.path.clone());
            match d.result {
                Ok((img, sha)) => match engine.analyze(&img) {
                    Ok(faces) => {
                        store.save_photo(&d.entry.path, d.entry.size, d.entry.mtime, &sha, img.dimensions(), &faces)?;
                        report.analyzed += 1;
                        report.faces_found += faces.len();
                        prog.faces += faces.len();
                    }
                    Err(e) => {
                        let msg = format!("analyse impossible ({e})");
                        store.save_photo_error(&d.entry.path, d.entry.size, d.entry.mtime, &msg)?;
                        errors.push((d.entry.path.clone(), msg));
                    }
                },
                Err(msg) => {
                    store.save_photo_error(&d.entry.path, d.entry.size, d.entry.mtime, &msg)?;
                    errors.push((d.entry.path.clone(), msg));
                }
            }
            prog.done += 1;
            prog.errors = errors.len();
            progress(&prog);
        }
        if report.cancelled {
            break;
        }
    }
    report.auto_reset = store.reset_auto_if_thresholds_changed(crate::thresholds::REVISION)?;
    report.auto_assigned = recognize::recognize_unassigned(store, opts.recognize)?;
    report.groups = cluster::recluster(store, opts.cluster)?;
    report.errors = errors;
    report.elapsed = start.elapsed();
    Ok(report)
}

/// Calcule la note de qualité des visages enregistrés avant qu'elle existe,
/// à partir des repères déjà stockés (aucun nouveau calcul d'empreinte : les
/// noms donnés restent attachés). Une photo illisible ou déplacée est sautée.
pub fn backfill_quality(store: &mut Store) -> Result<usize> {
    let missing = store.faces_missing_quality()?;
    if missing.is_empty() {
        return Ok(0);
    }
    let mut by_photo: BTreeMap<PathBuf, Vec<crate::store::StoredFace>> = BTreeMap::new();
    for f in missing {
        by_photo.entry(f.photo.clone()).or_default().push(f);
    }
    let entries: Vec<_> = by_photo.into_iter().collect();
    let values: Vec<(i64, f32)> = entries
        .par_iter()
        .flat_map_iter(|(path, faces)| {
            let img = std::fs::read(path).ok().and_then(|b| imageio::decode_oriented(&b).ok());
            faces
                .iter()
                .filter_map(|f| {
                    let img = img.as_ref()?;
                    let aligned = crate::align::align_face(img, &f.landmarks);
                    Some((f.id, crate::quality::FaceQuality::measure(&aligned, f.size, f.det_score).score()))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    store.set_quality(&values)?;
    Ok(values.len())
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ConvertReport {
    pub converted: usize,
    /// Visages dont la photo est introuvable ou illisible : ignorés, noms conservés.
    pub unreadable: usize,
    pub auto_assigned: usize,
    pub groups: usize,
}

/// Convertit toute la base vers le modèle de reconnaissance du moteur : les
/// empreintes sont recalculées à partir des photos et des repères enregistrés
/// (pas de nouvelle détection), donc les noms donnés, retraits et visages
/// ignorés sont conservés. Rien n'est écrit si la conversion est annulée.
pub fn convert_embeddings(
    store: &mut Store,
    engine: &mut Engine,
    mut progress: impl FnMut(&Progress),
    cancel: &AtomicBool,
) -> Result<ConvertReport> {
    let faces = store.all_stored_faces()?;
    let mut by_photo: BTreeMap<PathBuf, Vec<crate::store::StoredFace>> = BTreeMap::new();
    for f in faces {
        by_photo.entry(f.photo.clone()).or_default().push(f);
    }
    let photos: Vec<_> = by_photo.into_iter().collect();
    let mut prog = Progress { total: photos.len(), ..Default::default() };
    progress(&prog);
    let mut values = Vec::new();
    let mut unreadable = Vec::new();
    for batch in photos.chunks(BATCH) {
        if cancel.load(Ordering::Relaxed) {
            return Err(crate::Error::Invalid("conversion annulée : la base n'a pas été modifiée".into()));
        }
        let decoded: Vec<Option<image::RgbImage>> = batch
            .par_iter()
            .map(|(path, _)| std::fs::read(path).ok().and_then(|b| imageio::decode_oriented(&b).ok()))
            .collect();
        for ((path, fs), img) in batch.iter().zip(decoded) {
            prog.current = Some(path.clone());
            match img {
                Some(img) => {
                    for f in fs {
                        let aligned = crate::align::align_face(&img, &f.landmarks);
                        let q = crate::quality::FaceQuality::measure(&aligned, f.size, f.det_score).score();
                        values.push((f.id, engine.embedder.embed(&aligned)?, q));
                        prog.faces += 1;
                    }
                }
                None => {
                    unreadable.extend(fs.iter().map(|f| f.id));
                    prog.errors += 1;
                }
            }
            prog.done += 1;
            progress(&prog);
        }
    }
    store.replace_embeddings(&engine.ids, &values, &unreadable)?;
    let (auto_assigned, groups) = refresh_groups(store, Some(ScanOptions::for_kind(engine.kind)))?;
    Ok(ConvertReport { converted: values.len(), unreadable: unreadable.len(), auto_assigned, groups })
}

/// Relance reconnaissance puis regroupement (après un nommage dans la GUI).
/// Sans options : seuils du modèle qui a rempli la base.
pub fn refresh_groups(store: &mut Store, opts: Option<ScanOptions>) -> Result<(usize, usize)> {
    let opts = match opts {
        Some(o) => o,
        None => ScanOptions::for_store(store)?,
    };
    store.reset_auto_if_thresholds_changed(crate::thresholds::REVISION)?;
    let auto = recognize::recognize_unassigned(store, opts.recognize)?;
    let groups = cluster::recluster(store, opts.cluster)?;
    Ok((auto, groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_counts_ignored_extensions() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        for f in ["a.jpg", "b.PNG", "sub/c.jpeg", "d.heic", "e.HEIC", "f.mp4", "README"] {
            std::fs::write(d.path().join(f), b"x").unwrap();
        }
        let (files, ignored, errors) = collect_files(&[d.path().to_path_buf()]);
        assert_eq!(files.len(), 3);
        assert_eq!(ignored.get(".heic"), Some(&2));
        assert_eq!(ignored.get(".mp4"), Some(&1));
        assert_eq!(ignored.get("(sans extension)"), Some(&1));
        assert!(errors.is_empty());
        let r = ScanReport { ignored, ..Default::default() };
        assert_eq!(r.ignored_summary(), "ignorés : 2 .heic, 1 (sans extension), 1 .mp4");
    }

    #[test]
    fn progress_fraction_and_eta() {
        let p = Progress { total: 10, done: 4, ..Default::default() };
        assert!((p.fraction() - 0.4).abs() < 1e-6);
        assert_eq!(p.eta(Duration::from_secs(8)), Some(Duration::from_secs(12)));
        assert_eq!(Progress { total: 10, done: 0, ..Default::default() }.eta(Duration::from_secs(1)), None);
        assert_eq!(Progress { total: 10, done: 10, ..Default::default() }.eta(Duration::from_secs(1)), None);
        assert_eq!(Progress::default().fraction(), 1.0);
    }

    #[test]
    fn durations() {
        assert_eq!(human_duration(Duration::from_secs(45)), "45 s");
        assert_eq!(human_duration(Duration::from_secs(185)), "3 min 05 s");
        assert_eq!(human_duration(Duration::from_secs(4320)), "1 h 12 min");
    }

    #[test]
    fn missing_root_is_an_error_entry() {
        let (files, _, errors) = collect_files(&[PathBuf::from("Z:/n/existe/pas")]);
        assert!(files.is_empty());
        assert_eq!(errors.len(), 1);
    }
}
