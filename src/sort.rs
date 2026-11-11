//! Tri par copie dans `Destination/<Prénom>/` (F5).
//!
//! Le même plan sert à l'aperçu (`--dry-run`) et à l'exécution (ADR-4).
//! Les originaux ne sont jamais ouverts en écriture ; chaque copie passe par
//! un fichier temporaire renommé à la fin (pas de fichier partiel).

use crate::names::UNKNOWN_DIR;
use crate::store::PhotoPersons;
use crate::{Error, Result, util};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Règle d'un dossier spécial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// La photo contient au moins une des personnes.
    Any,
    /// La photo contient toutes les personnes (ensemble).
    All,
    /// La photo contient exactement ces personnes, aucune autre personne nommée.
    Only,
}

impl MatchMode {
    pub const ALL: [MatchMode; 3] = [MatchMode::Any, MatchMode::All, MatchMode::Only];

    pub fn as_str(self) -> &'static str {
        match self {
            MatchMode::Any => "any",
            MatchMode::All => "all",
            MatchMode::Only => "only",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// Libellé pour l'interface.
    pub fn label(self) -> &'static str {
        match self {
            MatchMode::Any => "au moins une de ces personnes",
            MatchMode::All => "toutes ces personnes ensemble",
            MatchMode::Only => "uniquement ces personnes",
        }
    }
}

/// Dossier de tri défini par l'utilisateur, par exemple « Parents » = les
/// photos où Léa et Tom sont ensemble.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SpecialFolder {
    pub id: i64,
    pub name: String,
    pub mode: MatchMode,
    /// Prénoms (résolus depuis la base au moment de la lecture).
    pub persons: BTreeSet<String>,
    #[serde(skip)]
    pub person_ids: BTreeSet<i64>,
}

impl SpecialFolder {
    /// La photo, qui contient les personnes nommées `present`, va-t-elle dans ce dossier ?
    pub fn matches(&self, present: &BTreeSet<String>) -> bool {
        if self.persons.is_empty() {
            return false;
        }
        match self.mode {
            MatchMode::Any => !self.persons.is_disjoint(present),
            MatchMode::All => self.persons.is_subset(present),
            MatchMode::Only => self.persons == *present,
        }
    }

    /// « Léa + Tom (toutes ces personnes ensemble) ».
    pub fn describe(&self) -> String {
        let names: Vec<&str> = self.persons.iter().map(String::as_str).collect();
        format!("{} ({})", names.join(" + "), self.mode.label())
    }
}

#[derive(Clone, Debug)]
pub struct SortOptions {
    pub dest: PathBuf,
    pub include_unknown: bool,
    /// Créer un dossier par personne nommée.
    pub per_person: bool,
    /// Dossiers spéciaux à remplir en plus.
    pub special: Vec<SpecialFolder>,
}

impl SortOptions {
    /// Un dossier par personne, `_Inconnus` selon `include_unknown`, pas de dossier spécial.
    pub fn new(dest: PathBuf, include_unknown: bool) -> Self {
        Self { dest, include_unknown, per_person: true, special: Vec::new() }
    }
}

/// Dossiers où copier une photo, sans doublon (casse ignorée, comme Windows).
pub fn folders_for(photo: &PhotoPersons, opts: &SortOptions) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        if !folders.iter().any(|f| f.to_lowercase() == name.to_lowercase()) {
            folders.push(name.to_string());
        }
    };
    if opts.per_person {
        photo.persons.iter().for_each(|p| push(p));
    }
    for sf in &opts.special {
        if sf.matches(&photo.persons) {
            push(&sf.name);
        }
    }
    if photo.persons.is_empty() && photo.has_unknown && opts.include_unknown {
        push(UNKNOWN_DIR);
    }
    folders
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyStatus {
    /// Copie sous le même nom.
    Copy,
    /// Copie sous un nom suffixé « (2) », « (3) »…
    Renamed,
    /// Un fichier identique (SHA-256) existe déjà : rien à faire.
    AlreadyPresent,
    /// La photo d'origine n'existe plus.
    SourceMissing,
}

impl CopyStatus {
    pub fn log_label(self) -> &'static str {
        match self {
            CopyStatus::Copy => "copiee",
            CopyStatus::Renamed => "renommee",
            CopyStatus::AlreadyPresent => "deja_presente",
            CopyStatus::SourceMissing => "source_absente",
        }
    }

    pub fn writes(self) -> bool {
        matches!(self, CopyStatus::Copy | CopyStatus::Renamed)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlannedCopy {
    pub source: PathBuf,
    pub target: PathBuf,
    pub person: String,
    pub size: u64,
    pub status: CopyStatus,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PersonPlan {
    pub copies: usize,
    pub bytes: u64,
    pub already_present: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub dest: PathBuf,
    pub copies: Vec<PlannedCopy>,
    pub per_person: BTreeMap<String, PersonPlan>,
    /// Octets à écrire.
    pub total_bytes: u64,
    pub free_bytes: Option<u64>,
    pub missing_sources: usize,
}

impl Plan {
    pub fn to_write(&self) -> usize {
        self.copies.iter().filter(|c| c.status.writes()).count()
    }

    pub fn summary(&self) -> String {
        let free = self.free_bytes.map(util::human_size).unwrap_or_else(|| "inconnu".into());
        let mut s = format!(
            "{} copie(s) à faire, {} à écrire, espace libre : {free}",
            self.to_write(),
            util::human_size(self.total_bytes)
        );
        for (name, p) in &self.per_person {
            s.push_str(&format!(
                "\n  {name} : {} copie(s), {}{}",
                p.copies,
                util::human_size(p.bytes),
                if p.already_present > 0 { format!(", {} déjà présente(s)", p.already_present) } else { String::new() }
            ));
        }
        if self.missing_sources > 0 {
            s.push_str(&format!("\n  {} photo(s) d'origine introuvable(s)", self.missing_sources));
        }
        s
    }

    pub fn has_enough_space(&self) -> bool {
        self.free_bytes.is_none_or(|f| f >= self.total_bytes)
    }
}

fn lower(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// Refuse une destination située dans (ou égale à) un dossier analysé.
pub fn check_destination(dest: &Path, roots: &[PathBuf]) -> Result<()> {
    let d = lower(dest);
    for r in roots {
        if d.starts_with(lower(r)) {
            return Err(Error::Invalid(format!(
                "la destination {} est dans le dossier analysé {} : les copies seraient réanalysées",
                dest.display(),
                r.display()
            )));
        }
    }
    Ok(())
}

fn candidate_name(file_name: &Path, k: usize) -> OsString {
    if k == 1 {
        return file_name.as_os_str().to_owned();
    }
    let stem = file_name.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match file_name.extension() {
        Some(ext) => format!("{stem} ({k}).{}", ext.to_string_lossy()).into(),
        None => format!("{stem} ({k})").into(),
    }
}

/// Choisit le fichier cible : nom libre, ou fichier identique déjà présent.
fn resolve_target(
    dir: &Path,
    source: &Path,
    size: u64,
    reserved: &HashSet<PathBuf>,
    src_sha: &mut Option<String>,
) -> std::io::Result<(PathBuf, CopyStatus)> {
    let file_name = Path::new(source.file_name().unwrap_or_default());
    for k in 1.. {
        let cand = dir.join(candidate_name(file_name, k));
        if reserved.contains(&lower(&cand)) {
            continue;
        }
        let status = if k == 1 { CopyStatus::Copy } else { CopyStatus::Renamed };
        let meta = match std::fs::metadata(&cand) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((cand, status)),
            Err(e) => return Err(e),
        };
        if meta.is_file() && meta.len() == size {
            if src_sha.is_none() {
                *src_sha = Some(util::sha256_file(source)?);
            }
            if src_sha.as_deref() == Some(util::sha256_file(&cand)?.as_str()) {
                return Ok((cand, CopyStatus::AlreadyPresent));
            }
        }
    }
    unreachable!("la boucle ne se termine que par un retour")
}

/// Calcule le plan de tri sans rien écrire (AC-F5-1).
pub fn plan(photos: &[PhotoPersons], roots: &[PathBuf], opts: &SortOptions) -> Result<Plan> {
    let dest = crate::scan::absolute(&opts.dest);
    check_destination(&dest, roots)?;
    let mut reserved: HashSet<PathBuf> = HashSet::new();
    let mut copies = Vec::new();
    let mut per_person: BTreeMap<String, PersonPlan> = BTreeMap::new();
    let mut total_bytes = 0;
    let mut missing_sources = 0;
    for photo in photos {
        let folders = folders_for(photo, opts);
        if folders.is_empty() {
            continue;
        }
        let size = match std::fs::metadata(&photo.path) {
            Ok(m) => m.len(),
            Err(_) => {
                missing_sources += 1;
                for person in folders {
                    copies.push(PlannedCopy {
                        source: photo.path.clone(),
                        target: dest.join(&person),
                        person,
                        size: 0,
                        status: CopyStatus::SourceMissing,
                    });
                }
                continue;
            }
        };
        let mut src_sha = None;
        for person in folders {
            let (target, status) = resolve_target(&dest.join(&person), &photo.path, size, &reserved, &mut src_sha)?;
            reserved.insert(lower(&target));
            let entry = per_person.entry(person.clone()).or_default();
            if status.writes() {
                entry.copies += 1;
                entry.bytes += size;
                total_bytes += size;
            } else {
                entry.already_present += 1;
            }
            copies.push(PlannedCopy { source: photo.path.clone(), target, person, size, status });
        }
    }
    let free_bytes = free_space(&dest);
    Ok(Plan { dest, copies, per_person, total_bytes, free_bytes, missing_sources })
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SortReport {
    pub copied: usize,
    pub renamed: usize,
    pub already_present: usize,
    pub missing_sources: usize,
    pub log_path: Option<PathBuf>,
    /// Erreur d'écriture qui a arrêté le tri (AC-F5-5).
    pub stopped_on_error: Option<String>,
    pub cancelled: bool,
}

impl SortReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} copie(s) dont {} renommée(s), {} déjà présente(s)",
            self.copied + self.renamed,
            self.renamed,
            self.already_present
        );
        if self.missing_sources > 0 {
            s.push_str(&format!(", {} source(s) introuvable(s)", self.missing_sources));
        }
        if self.cancelled {
            s.push_str(" ; tri annulé");
        }
        if let Some(e) = &self.stopped_on_error {
            s.push_str(&format!(" ; ARRÊT sur erreur : {e}"));
        }
        if let Some(l) = &self.log_path {
            s.push_str(&format!("\nJournal : {}", l.display()));
        }
        s
    }
}

/// Copie `src` vers `target` via un fichier temporaire, en conservant la date.
pub fn copy_atomic(src: &Path, target: &Path) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| std::io::Error::other("cible sans dossier parent"))?;
    std::fs::create_dir_all(dir)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.facesort-tmp"));
    let result = (|| {
        std::fs::copy(src, &tmp)?;
        let mtime = std::fs::metadata(src)?.modified()?;
        std::fs::File::options().write(true).open(&tmp)?.set_modified(mtime)?;
        std::fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

struct Log {
    writer: Option<csv::Writer<std::fs::File>>,
    path: PathBuf,
}

impl Log {
    fn open(dest: &Path) -> Self {
        let path = dest.join(format!("facesort-log-{}.csv", util::timestamp_utc(util::now_secs())));
        let writer = std::fs::create_dir_all(dest).ok().and_then(|_| csv::Writer::from_path(&path).ok());
        let mut log = Self { writer, path };
        log.row(&["source", "destination", "personne", "statut"]);
        log
    }

    fn row(&mut self, fields: &[&str]) {
        if let Some(w) = &mut self.writer
            && w.write_record(fields).is_err()
        {
            self.writer = None;
        }
    }

    fn finish(mut self) -> Option<PathBuf> {
        let w = self.writer.take()?;
        w.into_inner().ok()?.sync_all().ok()?;
        Some(self.path)
    }
}

/// Exécute le plan. S'arrête à la première erreur d'écriture.
pub fn execute(plan: &Plan, mut progress: impl FnMut(usize, usize), cancel: &AtomicBool) -> SortReport {
    let mut report = SortReport::default();
    let mut log = Log::open(&plan.dest);
    let total = plan.copies.len();
    let mut reserved: HashSet<PathBuf> = plan.copies.iter().map(|c| lower(&c.target)).collect();
    for (i, c) in plan.copies.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let src = c.source.to_string_lossy();
        let mut status = c.status;
        let mut target = c.target.clone();
        if status.writes() && target.exists() {
            // La destination a changé depuis l'aperçu : nouvelle résolution.
            reserved.remove(&lower(&target));
            let dir = target.parent().map(Path::to_path_buf).unwrap_or_default();
            match resolve_target(&dir, &c.source, c.size, &reserved, &mut None) {
                Ok((t, s)) => {
                    target = t;
                    status = if s == CopyStatus::Copy { CopyStatus::Renamed } else { s };
                }
                Err(e) => {
                    report.stopped_on_error = Some(format!("{} : {e}", target.display()));
                    log.row(&[&src, &target.to_string_lossy(), &c.person, "erreur"]);
                    break;
                }
            }
            reserved.insert(lower(&target));
        }
        if status.writes()
            && let Err(e) = copy_atomic(&c.source, &target)
        {
            report.stopped_on_error = Some(format!("{} : {e}", target.display()));
            log.row(&[&src, &target.to_string_lossy(), &c.person, "erreur"]);
            break;
        }
        match status {
            CopyStatus::Copy => report.copied += 1,
            CopyStatus::Renamed => report.renamed += 1,
            CopyStatus::AlreadyPresent => report.already_present += 1,
            CopyStatus::SourceMissing => report.missing_sources += 1,
        }
        log.row(&[&src, &target.to_string_lossy(), &c.person, status.log_label()]);
        progress(i + 1, total);
    }
    report.log_path = log.finish();
    report
}

/// Espace libre du volume contenant `path` (ou son premier ancêtre existant).
pub fn free_space(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    free_space_impl(existing)
}

#[cfg(windows)]
fn free_space_impl(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut free = 0u64;
    // SAFETY: chaîne UTF-16 terminée par zéro ; pointeurs nuls acceptés pour les champs non voulus.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space_impl(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn photo(path: &Path, persons: &[&str], unknown: bool) -> PhotoPersons {
        PhotoPersons {
            path: path.to_path_buf(),
            persons: persons.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>(),
            has_unknown: unknown,
        }
    }

    fn folder(name: &str, mode: MatchMode, persons: &[&str]) -> SpecialFolder {
        SpecialFolder {
            id: 0,
            name: name.into(),
            mode,
            persons: persons.iter().map(|s| s.to_string()).collect(),
            person_ids: BTreeSet::new(),
        }
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn special_folder_modes() {
        let any = folder("Cousins", MatchMode::Any, &["Léa", "Tom"]);
        let all = folder("Parents", MatchMode::All, &["Léa", "Tom"]);
        let only = folder("Duo", MatchMode::Only, &["Léa", "Tom"]);
        let cases = [
            (set(&["Léa"]), [true, false, false]),
            (set(&["Léa", "Tom"]), [true, true, true]),
            (set(&["Léa", "Tom", "Zoé"]), [true, true, false]),
            (set(&["Zoé"]), [false, false, false]),
            (set(&[]), [false, false, false]),
        ];
        for (present, [a, b, c]) in cases {
            assert_eq!(any.matches(&present), a, "any {present:?}");
            assert_eq!(all.matches(&present), b, "all {present:?}");
            assert_eq!(only.matches(&present), c, "only {present:?}");
        }
        assert!(!folder("Vide", MatchMode::Any, &[]).matches(&set(&["Léa"])));
        assert_eq!(all.describe(), "Léa + Tom (toutes ces personnes ensemble)");
    }

    #[test]
    fn folders_for_combines_person_special_and_unknown() {
        let mut opts = SortOptions::new(PathBuf::from("out"), true);
        opts.special = vec![folder("Parents", MatchMode::All, &["Léa", "Tom"]), folder("léa", MatchMode::Any, &["Léa"])];
        let p = photo(Path::new("a.jpg"), &["Léa", "Tom"], false);
        assert_eq!(folders_for(&p, &opts), vec!["Léa", "Tom", "Parents"], "« léa » fusionne avec « Léa »");
        opts.per_person = false;
        assert_eq!(folders_for(&p, &opts), vec!["Parents", "léa"]);
        let unknown = photo(Path::new("b.jpg"), &[], true);
        assert_eq!(folders_for(&unknown, &opts), vec![UNKNOWN_DIR]);
        let only_lea = photo(Path::new("c.jpg"), &["Léa"], false);
        assert_eq!(folders_for(&only_lea, &opts), vec!["léa"], "sans dossier par personne");
    }

    #[test]
    fn plan_with_special_folder_copies_there() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("duo.jpg"), b"duo").unwrap();
        std::fs::write(src.join("solo.jpg"), b"solo").unwrap();
        let photos = vec![photo(&src.join("duo.jpg"), &["Léa", "Tom"], false), photo(&src.join("solo.jpg"), &["Léa"], false)];
        let mut opts = SortOptions::new(d.path().join("out"), false);
        opts.special = vec![folder("Parents", MatchMode::All, &["Léa", "Tom"])];
        let p = plan(&photos, std::slice::from_ref(&src), &opts).unwrap();
        assert_eq!(p.per_person["Parents"].copies, 1);
        assert_eq!(p.per_person["Léa"].copies, 2);
        let r = execute(&p, |_, _| {}, &AtomicBool::new(false));
        assert!(d.path().join("out").join("Parents").join("duo.jpg").is_file());
        assert!(!d.path().join("out").join("Parents").join("solo.jpg").exists());
        assert_eq!(r.copied, 4);
    }

    #[test]
    fn candidate_names() {
        assert_eq!(candidate_name(Path::new("IMG_0001.jpg"), 1), OsString::from("IMG_0001.jpg"));
        assert_eq!(candidate_name(Path::new("IMG_0001.jpg"), 2), OsString::from("IMG_0001 (2).jpg"));
        assert_eq!(candidate_name(Path::new("photo"), 3), OsString::from("photo (3)"));
    }

    #[test]
    fn destination_inside_source_is_refused() {
        let roots = vec![PathBuf::from(r"C:\Photos")];
        assert!(check_destination(Path::new(r"c:\photos\tri"), &roots).is_err());
        assert!(check_destination(Path::new(r"C:\Photos"), &roots).is_err());
        assert!(check_destination(Path::new(r"C:\PhotosTri"), &roots).is_ok());
        assert!(check_destination(Path::new(r"D:\Tri"), &roots).is_ok());
    }

    /// AC-F5-2, AC-F5-3, AC-F5-4, AC-F5-6 : copie réelle, originaux intacts, second tri sans copie, journal.
    #[test]
    fn execute_copies_identically_and_is_idempotent() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let both = src.join("both.jpg");
        let unk = src.join("unk.jpg");
        std::fs::write(&both, b"photo avec Lea et Tom").unwrap();
        std::fs::write(&unk, b"photo inconnue").unwrap();
        let mtime_before = std::fs::metadata(&both).unwrap().modified().unwrap();
        let sha_before = util::sha256_file(&both).unwrap();
        let photos = vec![photo(&both, &["Léa", "Tom"], false), photo(&unk, &[], true)];
        let opts = SortOptions::new(d.path().join("out"), true);
        let cancel = AtomicBool::new(false);

        let p = plan(&photos, std::slice::from_ref(&src), &opts).unwrap();
        let r = execute(&p, |_, _| {}, &cancel);
        assert_eq!((r.copied, r.renamed, r.already_present), (3, 0, 0), "{}", r.summary());
        assert!(r.stopped_on_error.is_none());
        let out = d.path().join("out");
        for person in ["Léa", "Tom"] {
            let copy = out.join(person).join("both.jpg");
            assert_eq!(util::sha256_file(&copy).unwrap(), sha_before);
            assert_eq!(std::fs::metadata(&copy).unwrap().modified().unwrap(), mtime_before, "date conservée");
        }
        assert!(out.join(UNKNOWN_DIR).join("unk.jpg").is_file());
        assert_eq!(util::sha256_file(&both).unwrap(), sha_before, "original intact");
        assert_eq!(std::fs::metadata(&both).unwrap().modified().unwrap(), mtime_before);
        let log = std::fs::read_to_string(r.log_path.as_ref().unwrap()).unwrap();
        assert!(log.starts_with("source,destination,personne,statut"));
        assert_eq!(log.lines().count(), 4);
        assert!(!std::fs::read_dir(out.join("Léa")).unwrap().any(|e| {
            e.unwrap().file_name().to_string_lossy().ends_with(".facesort-tmp")
        }));

        // Second tri identique : rien à copier.
        let p2 = plan(&photos, std::slice::from_ref(&src), &opts).unwrap();
        assert_eq!(p2.to_write(), 0);
        let r2 = execute(&p2, |_, _| {}, &cancel);
        assert_eq!((r2.copied, r2.renamed, r2.already_present), (0, 0, 3));

        // Sans _Inconnus
        let opts3 = SortOptions::new(d.path().join("out3"), false);
        let p3 = plan(&photos, std::slice::from_ref(&src), &opts3).unwrap();
        assert!(p3.copies.iter().all(|c| c.person != UNKNOWN_DIR));
    }

    /// AC-F5-5 : écriture impossible -> arrêt, cause et journal, pas de fichier partiel.
    #[test]
    fn write_error_stops_cleanly() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.jpg"), b"aaa").unwrap();
        std::fs::write(src.join("b.jpg"), b"bbb").unwrap();
        let out = d.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let photos = vec![photo(&src.join("a.jpg"), &["Ana"], false), photo(&src.join("b.jpg"), &["Léa"], false)];
        let p = plan(&photos, std::slice::from_ref(&src), &SortOptions::new(out.clone(), false))
            .unwrap();
        // Après l'aperçu, un fichier « Léa » empêche la création du dossier Léa\
        std::fs::write(out.join("Léa"), b"bloque").unwrap();
        let r = execute(&p, |_, _| {}, &AtomicBool::new(false));
        assert!(r.stopped_on_error.as_deref().is_some_and(|e| e.contains("Léa")), "{}", r.summary());
        assert_eq!(r.copied, 1, "la copie précédant l'erreur est faite");
        let log = std::fs::read_to_string(r.log_path.as_ref().unwrap()).unwrap();
        assert!(log.lines().last().unwrap().ends_with(",erreur"), "{log}");
        let leftovers: Vec<_> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".facesort-tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn plan_counts_and_dedupes_same_names_from_different_folders() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::create_dir_all(src.join("b")).unwrap();
        std::fs::write(src.join("a/IMG.jpg"), b"one").unwrap();
        std::fs::write(src.join("b/IMG.jpg"), b"two!").unwrap();
        let photos = vec![photo(&src.join("a/IMG.jpg"), &["Léa"], false), photo(&src.join("b/IMG.jpg"), &["Léa"], false)];
        let p = plan(&photos, std::slice::from_ref(&src), &SortOptions::new(d.path().join("out"), true)).unwrap();
        assert_eq!(p.to_write(), 2);
        assert_eq!(p.total_bytes, 7);
        assert!(p.copies[1].target.ends_with("IMG (2).jpg"));
        assert!(!d.path().join("out").exists(), "l'aperçu n'écrit rien");
    }
}
