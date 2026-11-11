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

#[derive(Clone, Copy, Debug, Default)]
pub struct ScanOptions {
    pub recognize: RecognizeParams,
    pub cluster: ClusterParams,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Progress {
    pub total: usize,
    pub done: usize,
    pub faces: usize,
    pub errors: usize,
    pub current: Option<PathBuf>,
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

/// Parcours récursif : fichiers pris en charge, extensions ignorées, erreurs d'accès.
pub fn collect_files(
    roots: &[PathBuf],
) -> (Vec<FileEntry>, BTreeMap<String, usize>, Vec<(PathBuf, String)>) {
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
    opts: ScanOptions,
    mut progress: impl FnMut(&Progress),
    cancel: &AtomicBool,
) -> Result<ScanReport> {
    let start = Instant::now();
    store.check_models(&engine.ids)?;
    let roots: Vec<PathBuf> = roots.iter().map(|r| absolute(r)).collect();
    let (files, ignored, mut errors) = collect_files(&roots);
    for r in &roots {
        if r.is_dir() {
            store.add_root(r)?;
        }
    }
    let mut report = ScanReport { ignored, ..Default::default() };
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
    report.auto_assigned = recognize::recognize_unassigned(store, opts.recognize)?;
    report.groups = cluster::recluster(store, opts.cluster)?;
    report.errors = errors;
    report.elapsed = start.elapsed();
    Ok(report)
}

/// Relance reconnaissance puis regroupement (après un nommage dans la GUI).
pub fn refresh_groups(store: &mut Store, opts: ScanOptions) -> Result<(usize, usize)> {
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
    fn missing_root_is_an_error_entry() {
        let (files, _, errors) = collect_files(&[PathBuf::from("Z:/n/existe/pas")]);
        assert!(files.is_empty());
        assert_eq!(errors.len(), 1);
    }
}
