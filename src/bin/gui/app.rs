//! État de l'application et travaux en arrière-plan.

use eframe::egui::{self, RichText};
use facesort::engine::Engine;
use facesort::embed::EmbedderKind;
use facesort::scan::{self, ConvertReport, Progress, ScanReport};
use facesort::store::{self, Stats, Store};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Analyse,
    Personnes,
    Tri,
}

pub enum Msg {
    Progress(Progress),
    Done(Result<ScanReport, String>),
    Converted(Result<ConvertReport, String>),
}

pub struct ScanJob {
    /// Conversion de modèle (sinon : analyse).
    pub converting: bool,
    pub rx: Receiver<Msg>,
    pub cancel: Arc<AtomicBool>,
    pub progress: Progress,
    pub started: Instant,
}

impl ScanJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub struct FaceSortApp {
    pub tab: Tab,
    pub db_path: PathBuf,
    pub store: Option<Store>,
    pub store_error: Option<String>,
    pub stats: Stats,
    pub known_roots: Vec<PathBuf>,
    /// Dossiers choisis pour la prochaine analyse.
    pub roots: Vec<PathBuf>,
    pub job: Option<ScanJob>,
    /// Résultat de la dernière analyse de cette session.
    pub last: Option<Result<ScanReport, String>>,
    /// Modèle qui a rempli la base (None : base vide).
    pub model: Option<EmbedderKind>,
    /// Message après une conversion de modèle.
    pub notice: Option<Result<String, String>>,
    pub people: crate::people::PeopleState,
    pub tri: crate::tri::TriState,
    /// Vignettes chargées (None : vignette absente ou illisible).
    thumbs: HashMap<i64, Option<egui::TextureHandle>>,
    /// Note de qualité des visages affichés (None : non notée).
    qualities: HashMap<i64, Option<f32>>,
}

impl FaceSortApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let db_path = std::env::var_os("FACESORT_DB").map(PathBuf::from).unwrap_or_else(store::default_db_path);
        let mut app = Self {
            tab: Tab::Analyse,
            db_path,
            store: None,
            store_error: None,
            stats: Stats::default(),
            known_roots: Vec::new(),
            roots: Vec::new(),
            job: None,
            last: None,
            model: None,
            notice: None,
            people: Default::default(),
            tri: Default::default(),
            thumbs: HashMap::new(),
            qualities: HashMap::new(),
        };
        app.reload();
        app
    }

    /// Note de qualité d'un visage (mise en cache ; vidée à chaque rechargement).
    pub fn face_quality(&mut self, face: i64) -> Option<f32> {
        if let Some(q) = self.qualities.get(&face) {
            return *q;
        }
        let q = self.store.as_ref().and_then(|s| s.face_info(face).ok().flatten()).and_then(|i| i.quality);
        self.qualities.insert(face, q);
        q
    }

    pub fn forget_qualities(&mut self) {
        self.qualities.clear();
    }

    /// Texture de la vignette d'un visage, chargée une seule fois.
    pub fn thumb(&mut self, ctx: &egui::Context, face: i64) -> Option<egui::TextureHandle> {
        if let Some(t) = self.thumbs.get(&face) {
            return t.clone();
        }
        let tex = self
            .store
            .as_ref()
            .and_then(|s| s.face_thumb(face).ok().flatten())
            .and_then(|jpeg| image::load_from_memory(&jpeg).ok())
            .map(|img| {
                let rgb = img.to_rgb8();
                let size = [rgb.width() as usize, rgb.height() as usize];
                ctx.load_texture(format!("visage-{face}"), egui::ColorImage::from_rgb(size, rgb.as_raw()), Default::default())
            });
        self.thumbs.insert(face, tex.clone());
        tex
    }

    /// Rouvre la base si besoin et relit les compteurs.
    pub fn reload(&mut self) {
        if self.store.is_none() {
            match Store::open(&self.db_path) {
                Ok(s) => {
                    self.store = Some(s);
                    self.store_error = None;
                }
                Err(e) => self.store_error = Some(format!("Base {} illisible : {e}", self.db_path.display())),
            }
        }
        if let Some(s) = &self.store {
            match (s.stats(), s.roots()) {
                (Ok(st), Ok(r)) => {
                    self.stats = st;
                    self.known_roots = r;
                    self.model = s.embedder_kind().ok().flatten();
                }
                (Err(e), _) | (_, Err(e)) => self.store_error = Some(format!("Lecture de la base impossible : {e}")),
            }
        }
    }

    pub fn add_root(&mut self, dir: PathBuf) {
        let dir = scan::absolute(&dir);
        if !self.roots.contains(&dir) {
            self.roots.push(dir);
        }
    }

    pub fn start_scan(&mut self, ctx: &egui::Context, roots: Vec<PathBuf>) {
        if self.job.is_some() || roots.is_empty() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let db = self.db_path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<ScanReport, String> {
                let mut store = Store::open(&db).map_err(|e| format!("base illisible : {e}"))?;
                let mut engine = Engine::load_for_store(None, &store).map_err(|e| e.to_string())?;
                let ptx = tx.clone();
                let pctx = ctx.clone();
                scan::scan(
                    &mut store,
                    &mut engine,
                    &roots,
                    None,
                    move |p| {
                        let _ = ptx.send(Msg::Progress(p.clone()));
                        pctx.request_repaint();
                    },
                    &flag,
                )
                .map_err(|e| e.to_string())
            })();
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        });
        self.job = Some(ScanJob { converting: false, rx, cancel, progress: Progress::default(), started: Instant::now() });
        self.last = None;
    }

    /// Convertit la base vers un autre modèle, en arrière-plan, sans perdre les noms.
    pub fn start_convert(&mut self, ctx: &egui::Context, kind: EmbedderKind) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let db = self.db_path.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<ConvertReport, String> {
                let mut store = Store::open(&db).map_err(|e| format!("base illisible : {e}"))?;
                let mut engine = Engine::load_with(None, kind).map_err(|e| e.to_string())?;
                let ptx = tx.clone();
                let pctx = ctx.clone();
                scan::convert_embeddings(
                    &mut store,
                    &mut engine,
                    move |p| {
                        let _ = ptx.send(Msg::Progress(p.clone()));
                        pctx.request_repaint();
                    },
                    &flag,
                )
                .map_err(|e| e.to_string())
            })();
            let _ = tx.send(Msg::Converted(result));
            ctx.request_repaint();
        });
        self.job = Some(ScanJob { converting: true, rx, cancel, progress: Progress::default(), started: Instant::now() });
        self.notice = None;
    }

    fn poll_job(&mut self) {
        let mut finished = None;
        let mut converted = None;
        if let Some(job) = &mut self.job {
            while let Ok(msg) = job.rx.try_recv() {
                match msg {
                    Msg::Progress(p) => job.progress = p,
                    Msg::Done(r) => finished = Some(r),
                    Msg::Converted(r) => converted = Some(r),
                }
            }
        }
        if let Some(r) = converted {
            self.job = None;
            self.notice = Some(r.map(|c| {
                let lost = if c.unreadable > 0 {
                    format!(" ; {} visage(s) ignoré(s) (photo introuvable, noms conservés)", c.unreadable)
                } else {
                    String::new()
                };
                format!(
                    "Base convertie : {} visages recalculés, {} attribué(s) automatiquement, {} groupe(s) à nommer{lost}",
                    c.converted, c.auto_assigned, c.groups
                )
            }));
            self.reload();
            self.people.dirty = true;
            self.forget_qualities();
            self.tri.invalidate();
        }
        if let Some(r) = finished {
            self.job = None;
            if r.is_ok() {
                self.roots.clear();
            }
            self.last = Some(r);
            self.reload();
            self.people.dirty = true;
            self.tri.invalidate();
        }
        crate::tri::poll(self);
    }
}

impl eframe::App for FaceSortApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_job();

        egui::Panel::top("onglets").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("FaceSort").size(20.0).strong());
                ui.add_space(12.0);
                for (t, label) in [(Tab::Analyse, "Analyse"), (Tab::Personnes, "Personnes"), (Tab::Tri, "Tri")] {
                    if ui.selectable_label(self.tab == t, RichText::new(label).size(15.0)).clicked() {
                        self.tab = t;
                    }
                }
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("etat").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.job.is_some() || self.tri.is_running() {
                    ui.spinner();
                }
                let s = &self.stats;
                ui.label(
                    RichText::new(format!(
                        "Base : {}  ·  {} photos, {} visages, {} personnes, {} visages non attribués",
                        self.db_path.display(),
                        s.photos,
                        s.faces,
                        s.persons,
                        s.unassigned_faces
                    ))
                    .small(),
                );
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            // Une seule zone défilante par onglet (id distinct : chaque onglet garde sa position).
            let id = match self.tab {
                Tab::Analyse => "defilement_analyse",
                Tab::Personnes => "defilement_personnes",
                Tab::Tri => "defilement_tri",
            };
            egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Analyse => crate::analyse::show(self, ui),
                Tab::Personnes => crate::people::show(self, ui),
                Tab::Tri => crate::tri::show(self, ui),
            });
        });
    }
}
