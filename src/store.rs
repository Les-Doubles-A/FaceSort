//! Base SQLite : photos, visages, personnes, exclusions.
//!
//! Une seule source de vérité partagée par la GUI et la CLI (mode WAL).

use crate::embed;
use crate::engine::AnalyzedFace;
use crate::models::ModelIds;
use crate::names;
use crate::{Error, Result, util};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: i32 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS roots(path TEXT PRIMARY KEY);
CREATE TABLE IF NOT EXISTS photos(
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    sha256 TEXT,
    width INTEGER,
    height INTEGER,
    scanned_at INTEGER NOT NULL,
    error TEXT
);
CREATE TABLE IF NOT EXISTS persons(
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS faces(
    id INTEGER PRIMARY KEY,
    photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
    score REAL NOT NULL,
    landmarks BLOB NOT NULL,
    embedding BLOB NOT NULL,
    thumb BLOB,
    person_id INTEGER REFERENCES persons(id) ON DELETE SET NULL,
    assignment TEXT NOT NULL DEFAULT 'none' CHECK(assignment IN ('none','manual','auto')),
    cluster_id INTEGER,
    ignored INTEGER NOT NULL DEFAULT 0,
    quality REAL
);
CREATE INDEX IF NOT EXISTS faces_photo ON faces(photo_id);
CREATE INDEX IF NOT EXISTS faces_person ON faces(person_id);
CREATE INDEX IF NOT EXISTS faces_cluster ON faces(cluster_id);
CREATE TABLE IF NOT EXISTS face_exclusions(
    face_id INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES persons(id) ON DELETE CASCADE,
    PRIMARY KEY(face_id, person_id)
);
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assignment {
    None,
    Manual,
    Auto,
}

impl Assignment {
    fn as_str(self) -> &'static str {
        match self {
            Assignment::None => "none",
            Assignment::Manual => "manual",
            Assignment::Auto => "auto",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub photos: i64,
    pub photos_with_errors: i64,
    pub faces: i64,
    pub persons: i64,
    pub unassigned_faces: i64,
    pub groups: i64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct PersonSummary {
    pub id: i64,
    pub name: String,
    pub faces: i64,
    pub auto_faces: i64,
    pub photos: i64,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub cluster_id: i64,
    pub face_ids: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct FaceInfo {
    pub id: i64,
    pub photo_path: PathBuf,
    pub person_id: Option<i64>,
    pub assignment: Assignment,
    /// Note 0..1 (absente pour les visages analysés avant le schéma 2).
    pub quality: Option<f32>,
}

/// Visage enregistré : de quoi recalculer sa qualité sans nouvelle détection.
#[derive(Clone, Debug)]
pub struct StoredFace {
    pub id: i64,
    pub photo: PathBuf,
    pub landmarks: [[f32; 2]; 5],
    /// Plus petit côté du cadre, en pixels de l'image d'origine.
    pub size: f32,
    pub det_score: f32,
}

/// Empreinte d'un visage et sa note de qualité.
#[derive(Clone, Debug)]
pub struct FaceEmb {
    pub id: i64,
    pub embedding: Vec<f32>,
    pub quality: Option<f32>,
}

/// Résultat de la tentative de nommage d'un groupe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameOutcome {
    /// Nouvelle personne créée et visages attribués.
    Created(i64),
    /// Une personne de même nom existe : rien n'a changé, proposer la fusion.
    Exists { id: i64, name: String },
    /// Nom refusé (raison lisible).
    Invalid(String),
}

/// Entrée du tri : une photo et les personnes qui y figurent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhotoPersons {
    pub path: PathBuf,
    pub persons: BTreeSet<String>,
    pub has_unknown: bool,
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

/// `%LOCALAPPDATA%\FaceSort\facesort.db` (ou `./facesort.db` à défaut).
pub fn default_db_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("FaceSort").join("facesort.db"))
        .unwrap_or_else(|| PathBuf::from("facesort.db"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "base créée par une version plus récente de FaceSort (schéma {version})"
            )));
        }
        conn.execute_batch(SCHEMA)?;
        // Schéma 1 -> 2 : note de qualité des visages (ajout de colonne, sans perte).
        let has_quality: bool = conn
            .prepare("SELECT 1 FROM pragma_table_info('faces') WHERE name='quality'")?
            .exists([])?;
        if !has_quality {
            conn.execute("ALTER TABLE faces ADD COLUMN quality REAL", [])?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { conn, path: path.to_path_buf() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---------- modèles ----------

    fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0)).optional()?)
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES(?1, ?2)", params![key, value])?;
        Ok(())
    }

    /// Enregistre les modèles à la première utilisation ; refuse un modèle
    /// d'empreinte différent si des visages existent déjà.
    pub fn check_models(&self, ids: &ModelIds) -> Result<()> {
        match self.meta("embedder_model")? {
            Some(m) if m != ids.embedder && self.count("SELECT COUNT(*) FROM faces")? > 0 => Err(Error::Invalid(
                format!("la base a été remplie avec un autre modèle ({m}) ; supprimez-la pour tout réanalyser"),
            )),
            _ => {
                self.set_meta("embedder_model", &ids.embedder)?;
                self.set_meta("detector_model", &ids.detector)
            }
        }
    }

    /// Modèle de reconnaissance qui a produit les empreintes de la base.
    pub fn embedder_kind(&self) -> Result<Option<crate::embed::EmbedderKind>> {
        use crate::embed::EmbedderKind;
        Ok(self.meta("embedder_model")?.and_then(|m| {
            [EmbedderKind::SFace, EmbedderKind::AuraFace].into_iter().find(|k| m.starts_with(k.file()))
        }))
    }

    // ---------- photos ----------

    pub fn add_root(&self, root: &Path) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO roots(path) VALUES(?1)", [path_str(root)])?;
        Ok(())
    }

    pub fn roots(&self) -> Result<Vec<PathBuf>> {
        let mut st = self.conn.prepare("SELECT path FROM roots ORDER BY path")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?.into_iter().map(PathBuf::from).collect())
    }

    /// Vrai si la photo est connue avec la même taille et la même date.
    pub fn is_unchanged(&self, path: &Path, size: u64, mtime: i64) -> Result<bool> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row("SELECT size, mtime FROM photos WHERE path=?1", [path_str(path)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok(row == Some((size as i64, mtime)))
    }

    fn upsert_photo(
        tx: &rusqlite::Transaction,
        path: &Path,
        size: u64,
        mtime: i64,
        sha: Option<&str>,
        dims: Option<(u32, u32)>,
        error: Option<&str>,
    ) -> Result<i64> {
        let p = path_str(path);
        tx.execute("DELETE FROM photos WHERE path=?1", [&p])?;
        tx.execute(
            "INSERT INTO photos(path, size, mtime, sha256, width, height, scanned_at, error) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![p, size as i64, mtime, sha, dims.map(|d| d.0), dims.map(|d| d.1), util::now_secs(), error],
        )?;
        Ok(tx.last_insert_rowid())
    }

    /// Enregistre le résultat d'analyse d'une photo (remplace l'analyse précédente).
    pub fn save_photo(
        &mut self,
        path: &Path,
        size: u64,
        mtime: i64,
        sha: &str,
        dims: (u32, u32),
        faces: &[AnalyzedFace],
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        let photo_id = Self::upsert_photo(&tx, path, size, mtime, Some(sha), Some(dims), None)?;
        {
            let mut st = tx.prepare(
                "INSERT INTO faces(photo_id, x, y, w, h, score, landmarks, embedding, thumb, quality) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?;
            for f in faces {
                let d = &f.detection;
                let lm: Vec<f32> = d.landmarks.iter().flatten().copied().collect();
                st.execute(params![
                    photo_id,
                    d.bbox[0],
                    d.bbox[1],
                    d.bbox[2],
                    d.bbox[3],
                    d.score,
                    embed::to_blob(&lm),
                    embed::to_blob(&f.embedding),
                    f.thumb_jpeg,
                    f.quality.score()
                ])?;
            }
        }
        tx.commit()?;
        Ok(photo_id)
    }

    pub fn save_photo_error(&mut self, path: &Path, size: u64, mtime: i64, error: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::upsert_photo(&tx, path, size, mtime, None, None, Some(error))?;
        tx.commit()?;
        Ok(())
    }

    // ---------- lecture ----------

    fn count(&self, sql: &str) -> Result<i64> {
        Ok(self.conn.query_row(sql, [], |r| r.get(0))?)
    }

    pub fn stats(&self) -> Result<Stats> {
        Ok(Stats {
            photos: self.count("SELECT COUNT(*) FROM photos")?,
            photos_with_errors: self.count("SELECT COUNT(*) FROM photos WHERE error IS NOT NULL")?,
            faces: self.count("SELECT COUNT(*) FROM faces")?,
            persons: self.count("SELECT COUNT(*) FROM persons")?,
            unassigned_faces: self.count("SELECT COUNT(*) FROM faces WHERE person_id IS NULL AND ignored=0")?,
            groups: self.count("SELECT COUNT(DISTINCT cluster_id) FROM faces WHERE cluster_id IS NOT NULL")?,
        })
    }

    fn face_embs(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<FaceEmb>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map(params, |r| {
            Ok(FaceEmb { id: r.get(0)?, embedding: embed::from_blob(&r.get::<_, Vec<u8>>(1)?), quality: r.get(2)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Visages non attribués et non ignorés.
    pub fn unassigned_embeddings(&self) -> Result<Vec<FaceEmb>> {
        self.face_embs(
            "SELECT id, embedding, quality FROM faces WHERE person_id IS NULL AND ignored=0 ORDER BY id",
            [],
        )
    }

    /// Visages d'un groupe non nommé.
    pub fn group_embeddings(&self, cluster_id: i64) -> Result<Vec<FaceEmb>> {
        self.face_embs(
            "SELECT id, embedding, quality FROM faces WHERE cluster_id=?1 AND person_id IS NULL AND ignored=0 ORDER BY id",
            [cluster_id],
        )
    }

    /// Références de la reconnaissance : visages confirmés (nommés par
    /// l'utilisateur) de bonne qualité ; pour une personne qui n'en a aucun
    /// de bonne qualité, tous ses visages confirmés.
    pub fn confirmed_embeddings(&self) -> Result<Vec<(i64, Vec<f32>)>> {
        let mut st = self
            .conn
            .prepare("SELECT person_id, embedding, quality FROM faces WHERE assignment='manual' ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, embed::from_blob(&r.get::<_, Vec<u8>>(1)?), r.get::<_, Option<f32>>(2)?))
        })?;
        let all: Vec<_> = rows.collect::<std::result::Result<_, _>>()?;
        let has_good: HashSet<i64> =
            all.iter().filter(|(_, _, q)| crate::quality::is_good_score(*q)).map(|(p, _, _)| *p).collect();
        Ok(all
            .into_iter()
            .filter(|(p, _, q)| crate::quality::is_good_score(*q) || !has_good.contains(p))
            .map(|(p, e, _)| (p, e))
            .collect())
    }

    /// Visages sans note de qualité (analysés avant le schéma 2).
    pub fn faces_missing_quality(&self) -> Result<Vec<StoredFace>> {
        let mut st = self.conn.prepare(
            "SELECT f.id, p.path, f.landmarks, MIN(f.w, f.h), f.score FROM faces f JOIN photos p ON p.id=f.photo_id
             WHERE f.quality IS NULL ORDER BY p.path",
        )?;
        let rows = st.query_map([], |r| {
            let lm = embed::from_blob(&r.get::<_, Vec<u8>>(2)?);
            let mut pts = [[0f32; 2]; 5];
            for (k, p) in pts.iter_mut().enumerate() {
                *p = [lm.get(2 * k).copied().unwrap_or(0.0), lm.get(2 * k + 1).copied().unwrap_or(0.0)];
            }
            Ok(StoredFace {
                id: r.get(0)?,
                photo: PathBuf::from(r.get::<_, String>(1)?),
                landmarks: pts,
                size: r.get(3)?,
                det_score: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn set_quality(&mut self, values: &[(i64, f32)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare("UPDATE faces SET quality=?2 WHERE id=?1")?;
            for (f, q) in values {
                st.execute(params![f, q])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Si les seuils ont changé depuis la dernière fois, remet les visages
    /// attribués automatiquement dans « non attribué » (ils seront réévalués
    /// avec les nouveaux seuils). Les choix de l'utilisateur ne sont pas touchés.
    /// Renvoie le nombre de visages remis en jeu.
    pub fn reset_auto_if_thresholds_changed(&mut self, revision: i64) -> Result<usize> {
        let current = self.meta("thresholds_rev")?.and_then(|v| v.parse::<i64>().ok());
        if current == Some(revision) {
            return Ok(0);
        }
        let n = self
            .conn
            .execute("UPDATE faces SET person_id=NULL, assignment='none', cluster_id=NULL WHERE assignment='auto'", [])?;
        self.set_meta("thresholds_rev", &revision.to_string())?;
        Ok(n)
    }

    /// Prochain numéro de groupe libre.
    pub fn next_cluster_id(&self) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(cluster_id), -1) + 1 FROM faces", [], |r| r.get(0))?)
    }

    /// Change le groupe de quelques visages (sans toucher aux autres groupes).
    pub fn relabel(&mut self, labels: &[(i64, Option<i64>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare("UPDATE faces SET cluster_id=?2 WHERE id=?1 AND person_id IS NULL")?;
            for (face, label) in labels {
                st.execute(params![face, label])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn exclusions(&self) -> Result<HashMap<i64, HashSet<i64>>> {
        let mut st = self.conn.prepare("SELECT face_id, person_id FROM face_exclusions")?;
        let mut map: HashMap<i64, HashSet<i64>> = HashMap::new();
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (f, p) = row?;
            map.entry(f).or_default().insert(p);
        }
        Ok(map)
    }

    /// Remplace les groupes de tous les visages non attribués.
    pub fn set_clusters(&mut self, labels: &[(i64, Option<i64>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE faces SET cluster_id=NULL WHERE person_id IS NULL", [])?;
        {
            let mut st = tx.prepare("UPDATE faces SET cluster_id=?2 WHERE id=?1")?;
            for (face, label) in labels {
                st.execute(params![face, label])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Groupes non nommés, du plus grand au plus petit.
    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut st = self.conn.prepare(
            "SELECT cluster_id, id FROM faces WHERE cluster_id IS NOT NULL AND person_id IS NULL AND ignored=0 ORDER BY cluster_id, id",
        )?;
        let mut map: std::collections::BTreeMap<i64, Vec<i64>> = Default::default();
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (c, f) = row?;
            map.entry(c).or_default().push(f);
        }
        let mut groups: Vec<Group> = map.into_iter().map(|(cluster_id, face_ids)| Group { cluster_id, face_ids }).collect();
        groups.sort_by(|a, b| b.face_ids.len().cmp(&a.face_ids.len()).then(a.cluster_id.cmp(&b.cluster_id)));
        Ok(groups)
    }

    /// Visages non attribués hors groupe (« Visages isolés »).
    pub fn isolated_faces(&self) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare(
            "SELECT id FROM faces WHERE cluster_id IS NULL AND person_id IS NULL AND ignored=0 ORDER BY id",
        )?;
        let rows = st.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn persons(&self) -> Result<Vec<PersonSummary>> {
        let mut st = self.conn.prepare(
            "SELECT p.id, p.name,
                    COUNT(f.id),
                    COALESCE(SUM(f.assignment='auto'), 0),
                    COUNT(DISTINCT f.photo_id)
             FROM persons p LEFT JOIN faces f ON f.person_id = p.id
             GROUP BY p.id ORDER BY p.name COLLATE NOCASE",
        )?;
        let rows = st.query_map([], |r| {
            Ok(PersonSummary { id: r.get(0)?, name: r.get(1)?, faces: r.get(2)?, auto_faces: r.get(3)?, photos: r.get(4)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn person_by_name(&self, name: &str) -> Result<Option<(i64, String)>> {
        Ok(self.persons()?.into_iter().find(|p| names::same_name(&p.name, name)).map(|p| (p.id, p.name)))
    }

    pub fn person_faces(&self, person_id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT id FROM faces WHERE person_id=?1 ORDER BY assignment DESC, id")?;
        let rows = st.query_map([person_id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn face_thumb(&self, face_id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT thumb FROM faces WHERE id=?1", [face_id], |r| r.get(0)).optional()?.flatten())
    }

    pub fn face_info(&self, face_id: i64) -> Result<Option<FaceInfo>> {
        Ok(self
            .conn
            .query_row(
                "SELECT f.id, p.path, f.person_id, f.assignment, f.quality FROM faces f JOIN photos p ON p.id=f.photo_id WHERE f.id=?1",
                [face_id],
                |r| {
                    let a: String = r.get(3)?;
                    Ok(FaceInfo {
                        id: r.get(0)?,
                        photo_path: PathBuf::from(r.get::<_, String>(1)?),
                        person_id: r.get(2)?,
                        assignment: match a.as_str() {
                            "manual" => Assignment::Manual,
                            "auto" => Assignment::Auto,
                            _ => Assignment::None,
                        },
                        quality: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn group_faces(&self, cluster_id: i64) -> Result<Vec<i64>> {
        let mut st = self
            .conn
            .prepare("SELECT id FROM faces WHERE cluster_id=?1 AND person_id IS NULL AND ignored=0 ORDER BY id")?;
        let rows = st.query_map([cluster_id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---------- opérations sur les personnes (F3) ----------

    /// Attribue des visages à une personne, sauf ceux qui en sont exclus.
    /// Renvoie le nombre de visages attribués.
    pub fn assign_faces(&mut self, face_ids: &[i64], person_id: i64, how: Assignment) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut n = 0;
        {
            let mut st = tx.prepare(
                "UPDATE faces SET person_id=?2, assignment=?3, cluster_id=NULL
                 WHERE id=?1 AND NOT EXISTS (SELECT 1 FROM face_exclusions e WHERE e.face_id=?1 AND e.person_id=?2)",
            )?;
            for f in face_ids {
                n += st.execute(params![f, person_id, how.as_str()])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn create_person(&mut self, name: &str) -> Result<i64> {
        let name = names::validate_person_name(name).map_err(|r| Error::Invalid(names::invalid_message(&r)))?;
        if let Some((_, existing)) = self.person_by_name(&name)? {
            return Err(Error::Invalid(format!("« {existing} » existe déjà")));
        }
        self.conn.execute("INSERT INTO persons(name, created_at) VALUES(?1, ?2)", params![name, util::now_secs()])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Nomme un groupe : crée la personne, ou signale qu'elle existe déjà (AC-F3-2).
    pub fn name_group(&mut self, cluster_id: i64, raw_name: &str) -> Result<NameOutcome> {
        let name = match names::validate_person_name(raw_name) {
            Ok(n) => n,
            Err(reason) => return Ok(NameOutcome::Invalid(names::invalid_message(&reason))),
        };
        if let Some((id, name)) = self.person_by_name(&name)? {
            return Ok(NameOutcome::Exists { id, name });
        }
        let faces = self.group_faces(cluster_id)?;
        let id = self.create_person(&name)?;
        self.assign_faces(&faces, id, Assignment::Manual)?;
        Ok(NameOutcome::Created(id))
    }

    /// Ajoute les visages d'un groupe à une personne existante.
    pub fn add_group_to_person(&mut self, cluster_id: i64, person_id: i64) -> Result<usize> {
        let faces = self.group_faces(cluster_id)?;
        self.assign_faces(&faces, person_id, Assignment::Manual)
    }

    /// Fusionne `from` dans `into` ; les exclusions de `from` sont reportées.
    pub fn merge_persons(&mut self, from: i64, into: i64) -> Result<()> {
        if from == into {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO face_exclusions(face_id, person_id) SELECT face_id, ?2 FROM face_exclusions WHERE person_id=?1",
            params![from, into],
        )?;
        tx.execute("UPDATE faces SET person_id=?2 WHERE person_id=?1", params![from, into])?;
        tx.execute("DELETE FROM persons WHERE id=?1", [from])?;
        tx.commit()?;
        Ok(())
    }

    pub fn rename_person(&mut self, person_id: i64, raw_name: &str) -> Result<NameOutcome> {
        let name = match names::validate_person_name(raw_name) {
            Ok(n) => n,
            Err(reason) => return Ok(NameOutcome::Invalid(names::invalid_message(&reason))),
        };
        if let Some((id, existing)) = self.person_by_name(&name)?
            && id != person_id
        {
            return Ok(NameOutcome::Exists { id, name: existing });
        }
        self.conn.execute("UPDATE persons SET name=?2 WHERE id=?1", params![person_id, name])?;
        Ok(NameOutcome::Created(person_id))
    }

    /// Retire un visage de sa personne ; il ne lui sera plus jamais attribué (AC-F3-4).
    pub fn remove_face_from_person(&mut self, face_id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        let person: Option<i64> =
            tx.query_row("SELECT person_id FROM faces WHERE id=?1", [face_id], |r| r.get(0)).optional()?.flatten();
        if let Some(p) = person {
            tx.execute("INSERT OR IGNORE INTO face_exclusions(face_id, person_id) VALUES(?1, ?2)", params![face_id, p])?;
        }
        tx.execute("UPDATE faces SET person_id=NULL, assignment='none', cluster_id=NULL WHERE id=?1", [face_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Confirme les visages « auto » d'une personne (ils deviennent des références).
    pub fn confirm_auto_faces(&mut self, person_id: i64) -> Result<usize> {
        Ok(self
            .conn
            .execute("UPDATE faces SET assignment='manual' WHERE person_id=?1 AND assignment='auto'", [person_id])?)
    }

    /// Masque des visages (groupe sans intérêt) : ni regroupés, ni triés.
    pub fn ignore_faces(&mut self, face_ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut st = tx.prepare("UPDATE faces SET ignored=1, cluster_id=NULL WHERE id=?1 AND person_id IS NULL")?;
            for f in face_ids {
                st.execute([f])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Retire un visage d'un groupe non nommé (il redevient isolé jusqu'au prochain regroupement).
    pub fn remove_face_from_group(&mut self, face_id: i64) -> Result<()> {
        self.conn.execute("UPDATE faces SET cluster_id=NULL WHERE id=?1 AND person_id IS NULL", [face_id])?;
        Ok(())
    }

    // ---------- tri ----------

    /// Photos analysées sans erreur contenant au moins un visage non ignoré.
    pub fn photos_for_sort(&self) -> Result<Vec<PhotoPersons>> {
        let mut st = self.conn.prepare(
            "SELECT p.path, pe.name, f.person_id IS NULL
             FROM photos p JOIN faces f ON f.photo_id = p.id
             LEFT JOIN persons pe ON pe.id = f.person_id
             WHERE p.error IS NULL AND f.ignored = 0
             ORDER BY p.path",
        )?;
        let mut out: Vec<PhotoPersons> = Vec::new();
        for row in st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, bool>(2)?))
        })? {
            let (path, name, unknown) = row?;
            let path = PathBuf::from(path);
            if out.last().is_none_or(|p| p.path != path) {
                out.push(PhotoPersons { path, persons: BTreeSet::new(), has_unknown: false });
            }
            let last = out.last_mut().expect("élément ajouté ci-dessus");
            if let Some(n) = name {
                last.persons.insert(n);
            }
            last.has_unknown |= unknown;
        }
        Ok(out)
    }

    pub fn photo_errors(&self) -> Result<Vec<(PathBuf, String)>> {
        let mut st = self.conn.prepare("SELECT path, error FROM photos WHERE error IS NOT NULL ORDER BY path")?;
        let rows = st.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get::<_, String>(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

pub fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::detect::Detection;

    pub fn fake_face(emb: Vec<f32>) -> AnalyzedFace {
        AnalyzedFace {
            detection: Detection { bbox: [0.0, 0.0, 50.0, 50.0], landmarks: [[0.0; 2]; 5], score: 0.95 },
            embedding: emb,
            quality: crate::quality::FaceQuality {
                brightness: 128.0,
                contrast: 50.0,
                sharpness: 200.0,
                size: 150.0,
                det_score: 0.97,
            },
            thumb_jpeg: vec![],
        }
    }

    pub fn mem_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, store)
    }

    /// Ajoute une photo avec `n` visages et renvoie leurs identifiants.
    pub fn add_photo(store: &mut Store, path: &str, n: usize) -> Vec<i64> {
        let faces: Vec<_> = (0..n).map(|i| fake_face(vec![i as f32, 1.0])).collect();
        let pid = store.save_photo(Path::new(path), 10, 1, "abc", (100, 100), &faces).unwrap();
        let mut st = store.conn.prepare("SELECT id FROM faces WHERE photo_id=?1 ORDER BY id").unwrap();
        st.query_map([pid], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn reopen_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("f.db");
        {
            let mut s = Store::open(&p).unwrap();
            add_photo(&mut s, "a.jpg", 2);
            s.create_person("Léa").unwrap();
        }
        let s = Store::open(&p).unwrap();
        let st = s.stats().unwrap();
        assert_eq!((st.photos, st.faces, st.persons), (1, 2, 1));
    }

    #[test]
    fn unchanged_detection() {
        let (_d, mut s) = mem_store();
        add_photo(&mut s, "a.jpg", 1);
        assert!(s.is_unchanged(Path::new("a.jpg"), 10, 1).unwrap());
        assert!(!s.is_unchanged(Path::new("a.jpg"), 11, 1).unwrap());
        assert!(!s.is_unchanged(Path::new("b.jpg"), 10, 1).unwrap());
    }

    #[test]
    fn rescan_replaces_faces() {
        let (_d, mut s) = mem_store();
        add_photo(&mut s, "a.jpg", 3);
        add_photo(&mut s, "a.jpg", 1);
        assert_eq!(s.stats().unwrap().faces, 1);
    }

    #[test]
    fn name_group_creates_person_and_persists() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 2);
        s.set_clusters(&[(f[0], Some(0)), (f[1], Some(0))]).unwrap();
        let out = s.name_group(0, "Léa").unwrap();
        let NameOutcome::Created(id) = out else { panic!("{out:?}") };
        assert_eq!(s.person_faces(id).unwrap().len(), 2);
        assert!(s.groups().unwrap().is_empty());
    }

    // AC-F3-2
    #[test]
    fn merge_on_same_name() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 3);
        s.set_clusters(&[(f[0], Some(0)), (f[1], Some(1)), (f[2], Some(1))]).unwrap();
        let NameOutcome::Created(lea) = s.name_group(0, "Léa").unwrap() else { panic!() };
        let out = s.name_group(1, "léa").unwrap();
        assert_eq!(out, NameOutcome::Exists { id: lea, name: "Léa".into() });
        assert_eq!(s.person_faces(lea).unwrap().len(), 1, "rien ne change avant confirmation");
        s.add_group_to_person(1, lea).unwrap();
        assert_eq!(s.persons().unwrap().len(), 1);
        assert_eq!(s.person_faces(lea).unwrap().len(), 3);
    }

    #[test]
    fn merge_persons_moves_faces_and_exclusions() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 3);
        let a = s.create_person("A").unwrap();
        let b = s.create_person("B").unwrap();
        s.assign_faces(&f[..2], a, Assignment::Manual).unwrap();
        s.assign_faces(&f[2..], b, Assignment::Manual).unwrap();
        s.remove_face_from_person(f[1]).unwrap();
        s.merge_persons(a, b).unwrap();
        assert_eq!(s.persons().unwrap().len(), 1);
        assert_eq!(s.person_faces(b).unwrap(), vec![f[0], f[2]]);
        assert_eq!(s.assign_faces(&[f[1]], b, Assignment::Manual).unwrap(), 0, "exclusion reportée");
    }

    // AC-F3-3 côté base
    #[test]
    fn invalid_name_is_refused_and_nothing_saved() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 1);
        s.set_clusters(&[(f[0], Some(0))]).unwrap();
        for bad in ["Tom/Léa", "CON"] {
            let NameOutcome::Invalid(msg) = s.name_group(0, bad).unwrap() else { panic!() };
            assert!(msg.starts_with("Nom de dossier invalide sous Windows : "), "{msg}");
        }
        assert_eq!(s.stats().unwrap().persons, 0);
    }

    // AC-F3-4
    #[test]
    fn exclusion_persists() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 2);
        let lea = s.create_person("Léa").unwrap();
        s.assign_faces(&f, lea, Assignment::Manual).unwrap();
        s.remove_face_from_person(f[1]).unwrap();
        assert_eq!(s.person_faces(lea).unwrap(), vec![f[0]]);
        // Un nouveau regroupement qui remet le visage avec le groupe de Léa ne le réattribue pas
        s.set_clusters(&[(f[1], Some(7))]).unwrap();
        assert_eq!(s.add_group_to_person(7, lea).unwrap(), 0);
        assert_eq!(s.assign_faces(&[f[1]], lea, Assignment::Auto).unwrap(), 0);
        assert_eq!(s.exclusions().unwrap()[&f[1]], HashSet::from([lea]));
        assert_eq!(s.person_faces(lea).unwrap(), vec![f[0]]);
    }

    #[test]
    fn photos_for_sort_lists_persons_and_unknowns() {
        let (_d, mut s) = mem_store();
        let f1 = add_photo(&mut s, "1.jpg", 2);
        let f2 = add_photo(&mut s, "2.jpg", 1);
        let f3 = add_photo(&mut s, "3.jpg", 1);
        add_photo(&mut s, "4.jpg", 0);
        let lea = s.create_person("Léa").unwrap();
        let tom = s.create_person("Tom").unwrap();
        s.assign_faces(&[f1[0]], lea, Assignment::Manual).unwrap();
        s.assign_faces(&[f1[1]], tom, Assignment::Auto).unwrap();
        s.ignore_faces(&f3).unwrap();
        let rows = s.photos_for_sort().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].persons, BTreeSet::from(["Léa".to_string(), "Tom".to_string()]));
        assert!(!rows[0].has_unknown);
        assert!(rows[1].persons.is_empty() && rows[1].has_unknown);
        let _ = f2;
    }

    #[test]
    fn auto_assignments_are_reset_once_when_thresholds_change() {
        let (_d, mut s) = mem_store();
        let f = add_photo(&mut s, "a.jpg", 2);
        let lea = s.create_person("Léa").unwrap();
        s.assign_faces(&f[..1], lea, Assignment::Manual).unwrap();
        s.assign_faces(&f[1..], lea, Assignment::Auto).unwrap();
        assert_eq!(s.reset_auto_if_thresholds_changed(2).unwrap(), 1);
        assert_eq!(s.person_faces(lea).unwrap(), vec![f[0]], "le choix manuel reste");
        s.assign_faces(&f[1..], lea, Assignment::Auto).unwrap();
        assert_eq!(s.reset_auto_if_thresholds_changed(2).unwrap(), 0, "une seule fois par révision");
    }

    #[test]
    fn schema_v1_database_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v1.db");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(&SCHEMA.replace(",\n    quality REAL", "")).unwrap();
            c.execute("INSERT INTO photos(path,size,mtime,scanned_at) VALUES('a.jpg',1,1,1)", []).unwrap();
            c.execute(
                "INSERT INTO faces(photo_id,x,y,w,h,score,landmarks,embedding) VALUES(1,0,0,50,50,0.9,x'00',x'0000803f')",
                [],
            )
            .unwrap();
            c.pragma_update(None, "user_version", 1).unwrap();
        }
        let s = Store::open(&p).unwrap();
        assert_eq!(s.stats().unwrap().faces, 1, "données conservées");
        assert_eq!(s.faces_missing_quality().unwrap().len(), 1);
        assert_eq!(s.unassigned_embeddings().unwrap()[0].quality, None);
    }

    #[test]
    fn model_change_is_refused_when_faces_exist() {
        let (_d, mut s) = mem_store();
        let a = ModelIds { detector: "d".into(), embedder: "e1".into() };
        let b = ModelIds { detector: "d".into(), embedder: "e2".into() };
        s.check_models(&a).unwrap();
        s.check_models(&b).unwrap();
        add_photo(&mut s, "a.jpg", 1);
        assert!(s.check_models(&a).is_err());
        s.check_models(&b).unwrap();
    }
}
