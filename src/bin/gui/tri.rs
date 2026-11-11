//! Écran E3 « Tri » : destination, aperçu, copie (F5).

use crate::app::{FaceSortApp, Tab};
use eframe::egui::{self, Color32, RichText};
use facesort::sort::{self, MatchMode, Plan, SortOptions, SortReport, SpecialFolder};
use facesort::util::human_size;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

const ERROR_COLOR: Color32 = Color32::from_rgb(200, 60, 60);

pub enum SortMsg {
    Progress(usize, usize),
    Done(SortReport),
}

pub struct SortJob {
    rx: Receiver<SortMsg>,
    cancel: Arc<AtomicBool>,
    done: usize,
    total: usize,
}

/// Formulaire de création ou de modification d'un dossier spécial.
pub struct SpecialEditor {
    /// None : nouveau dossier.
    pub id: Option<i64>,
    pub name: String,
    pub mode: MatchMode,
    pub persons: BTreeSet<i64>,
    pub error: Option<String>,
}

impl Default for SpecialEditor {
    fn default() -> Self {
        Self { id: None, name: String::new(), mode: MatchMode::All, persons: BTreeSet::new(), error: None }
    }
}

pub struct TriState {
    pub dest: Option<PathBuf>,
    pub include_unknown: bool,
    /// Créer un dossier par personne (en plus des dossiers spéciaux).
    pub per_person: bool,
    pub editor: SpecialEditor,
    pub plan: Option<Result<Plan, String>>,
    pub job: Option<SortJob>,
    pub report: Option<SortReport>,
}

impl Default for TriState {
    fn default() -> Self {
        Self {
            dest: None,
            include_unknown: true,
            per_person: true,
            editor: SpecialEditor::default(),
            plan: None,
            job: None,
            report: None,
        }
    }
}

impl TriState {
    /// Toute modification de la base ou des options rend l'aperçu périmé.
    pub fn invalidate(&mut self) {
        if self.job.is_none() {
            self.plan = None;
        }
    }

    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }
}

pub fn poll(app: &mut FaceSortApp) {
    let st = &mut app.tri;
    let mut finished = None;
    if let Some(job) = &mut st.job {
        while let Ok(msg) = job.rx.try_recv() {
            match msg {
                SortMsg::Progress(d, t) => {
                    job.done = d;
                    job.total = t;
                }
                SortMsg::Done(r) => finished = Some(r),
            }
        }
    }
    if let Some(r) = finished {
        st.job = None;
        st.plan = None;
        st.report = Some(r);
    }
}

fn compute_plan(app: &mut FaceSortApp) {
    let Some(dest) = app.tri.dest.clone() else { return };
    let Some(store) = &app.store else { return };
    let result = (|| -> facesort::Result<Plan> {
        let photos = store.photos_for_sort()?;
        let roots = store.roots()?;
        let mut opts = SortOptions::new(dest, app.tri.include_unknown);
        opts.per_person = app.tri.per_person;
        opts.special = store.special_folders()?;
        sort::plan(&photos, &roots, &opts)
    })();
    app.tri.plan = Some(result.map_err(|e| e.to_string()));
    app.tri.report = None;
}

enum SpecialAction {
    Edit(SpecialFolder),
    Delete(i64),
    Save,
    Cancel,
}

/// Section « Dossiers spéciaux » : liste, modification, création.
fn special_section(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    let specials = app.store.as_ref().and_then(|s| s.special_folders().ok()).unwrap_or_default();
    let persons = app.store.as_ref().and_then(|s| s.persons().ok()).unwrap_or_default();
    let mut action = None;
    egui::CollapsingHeader::new(RichText::new(format!("Dossiers spéciaux ({})", specials.len())).strong())
        .id_salt("dossiers_speciaux")
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "Un dossier spécial regroupe des photos selon une règle, par exemple « Parents » = les photos où Léa et Tom sont ensemble.",
                )
                .small(),
            );
            for f in &specials {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&f.name).strong());
                    ui.label(format!(": {}", f.describe()));
                    if ui.small_button("Modifier").clicked() {
                        action = Some(SpecialAction::Edit(f.clone()));
                    }
                    if ui.small_button("Supprimer").on_hover_text("Supprime la règle, pas les photos déjà copiées").clicked() {
                        action = Some(SpecialAction::Delete(f.id));
                    }
                });
            }
            ui.add_space(6.0);
            if persons.is_empty() {
                ui.label("Nommez d'abord des personnes dans l'onglet Personnes pour créer un dossier spécial.");
                return;
            }
            egui::Frame::group(ui.style()).inner_margin(8.0).show(ui, |ui| {
                let ed = &mut app.tri.editor;
                let title = match ed.id {
                    Some(_) => "Modifier le dossier spécial",
                    None => "Nouveau dossier spécial",
                };
                ui.label(RichText::new(title).strong());
                ui.horizontal(|ui| {
                    ui.label("Nom du dossier :");
                    ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text("ex. Parents").desired_width(220.0));
                });
                ui.horizontal(|ui| {
                    ui.label("Photos contenant :");
                    egui::ComboBox::from_id_salt("regle_speciale").selected_text(ed.mode.label()).show_ui(ui, |ui| {
                        for m in MatchMode::ALL {
                            ui.selectable_value(&mut ed.mode, m, m.label());
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    for p in &persons {
                        let mut on = ed.persons.contains(&p.id);
                        if ui.checkbox(&mut on, &p.name).changed() {
                            if on {
                                ed.persons.insert(p.id);
                            } else {
                                ed.persons.remove(&p.id);
                            }
                        }
                    }
                });
                if let Some(e) = &ed.error {
                    ui.colored_label(ERROR_COLOR, e);
                }
                ui.horizontal(|ui| {
                    let label = if ed.id.is_some() { "Enregistrer" } else { "Créer le dossier" };
                    if ui.button(RichText::new(label).strong()).clicked() {
                        action = Some(SpecialAction::Save);
                    }
                    if (ed.id.is_some() || !ed.name.is_empty() || !ed.persons.is_empty()) && ui.button("Annuler").clicked() {
                        action = Some(SpecialAction::Cancel);
                    }
                });
            });
        });
    let Some(action) = action else { return };
    let Some(store) = &mut app.store else { return };
    match action {
        SpecialAction::Edit(f) => {
            app.tri.editor =
                SpecialEditor { id: Some(f.id), name: f.name, mode: f.mode, persons: f.person_ids, error: None };
        }
        SpecialAction::Delete(id) => {
            if let Err(e) = store.delete_special_folder(id) {
                app.tri.editor.error = Some(e.to_string());
            }
            if app.tri.editor.id == Some(id) {
                app.tri.editor = SpecialEditor::default();
            }
            app.tri.invalidate();
        }
        SpecialAction::Save => {
            let ed = &app.tri.editor;
            let ids: Vec<i64> = ed.persons.iter().copied().collect();
            match store.save_special_folder(ed.id, &ed.name, ed.mode, &ids) {
                Ok(Ok(_)) => {
                    app.tri.editor = SpecialEditor::default();
                    app.tri.invalidate();
                }
                Ok(Err(msg)) => app.tri.editor.error = Some(msg),
                Err(e) => app.tri.editor.error = Some(e.to_string()),
            }
        }
        SpecialAction::Cancel => app.tri.editor = SpecialEditor::default(),
    }
}

fn start(app: &mut FaceSortApp, ctx: &egui::Context) {
    let Some(Ok(plan)) = app.tri.plan.clone() else { return };
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let ctx = ctx.clone();
    let total = plan.copies.len();
    std::thread::spawn(move || {
        let ptx = tx.clone();
        let pctx = ctx.clone();
        let report = sort::execute(
            &plan,
            move |d, t| {
                let _ = ptx.send(SortMsg::Progress(d, t));
                pctx.request_repaint();
            },
            &flag,
        );
        let _ = tx.send(SortMsg::Done(report));
        ctx.request_repaint();
    });
    app.tri.job = Some(SortJob { rx, cancel, done: 0, total });
}

pub fn show(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    ui.heading("Tri");
    ui.add_space(6.0);

    if let Some(job) = &app.tri.job {
        ui.label(RichText::new("Copie en cours").strong());
        let frac = if job.total == 0 { 1.0 } else { job.done as f32 / job.total as f32 };
        ui.add(egui::ProgressBar::new(frac).show_percentage().animate(true));
        ui.label(format!("{}/{} copies traitées", job.done, job.total));
        if job.cancel.load(Ordering::Relaxed) {
            ui.label("Annulation… (la copie en cours se termine)");
        } else if ui.button("Annuler").on_hover_text("Les copies déjà faites restent en place").clicked() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        return;
    }

    if app.stats.persons == 0 && app.stats.faces == 0 {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Aucune personne nommée").size(18.0));
            ui.label("Analysez un dossier puis nommez les groupes de visages avant de trier.");
            if ui.button("Aller à l'analyse").clicked() {
                app.tab = Tab::Analyse;
            }
        });
        return;
    }
    if app.stats.persons == 0 {
        ui.horizontal(|ui| {
            ui.label("Aucune personne nommée : seules les photos d'inconnus seraient copiées.");
            if ui.button("Nommer des personnes").clicked() {
                app.tab = Tab::Personnes;
            }
        });
        ui.add_space(6.0);
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new("Destination :").strong());
        match &app.tri.dest {
            Some(d) => ui.label(d.display().to_string()),
            None => ui.label("aucune"),
        };
        if ui.button("Choisir…").clicked()
            && let Some(dir) = rfd::FileDialog::new().set_title("Dossier de destination du tri").pick_folder()
        {
            app.tri.dest = Some(dir);
            app.tri.invalidate();
            app.tri.report = None;
        }
    });
    if ui.checkbox(&mut app.tri.per_person, "Créer un dossier par personne").changed() {
        app.tri.invalidate();
    }
    if ui
        .checkbox(&mut app.tri.include_unknown, "Copier les photos sans personne nommée dans « _Inconnus »")
        .changed()
    {
        app.tri.invalidate();
    }
    ui.label(RichText::new("Les originaux ne sont jamais modifiés : chaque photo est copiée dans chaque dossier qui la concerne.").small());
    ui.add_space(6.0);
    special_section(app, ui);
    ui.add_space(6.0);
    if ui.add_enabled(app.tri.dest.is_some(), egui::Button::new("Calculer l'aperçu")).clicked() {
        compute_plan(app);
    }
    ui.add_space(8.0);

    let mut go = false;
    match &app.tri.plan {
        Some(Err(e)) => {
            ui.colored_label(ERROR_COLOR, format!("Aperçu impossible : {e}"));
        }
        Some(Ok(plan)) => {
            egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
                ui.label(RichText::new("Aperçu (rien n'est encore écrit)").strong());
                egui::Grid::new("apercu").striped(true).num_columns(4).show(ui, |ui| {
                    ui.label(RichText::new("Dossier").strong());
                    ui.label(RichText::new("Copies").strong());
                    ui.label(RichText::new("Taille").strong());
                    ui.label(RichText::new("Déjà présentes").strong());
                    ui.end_row();
                    let special: BTreeSet<String> = app
                        .store
                        .as_ref()
                        .and_then(|st| st.special_folders().ok())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|f| f.name)
                        .collect();
                    for (name, p) in &plan.per_person {
                        if special.contains(name) {
                            ui.label(format!("{name} (spécial)"));
                        } else {
                            ui.label(name);
                        }
                        ui.label(p.copies.to_string());
                        ui.label(human_size(p.bytes));
                        ui.label(p.already_present.to_string());
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                let free = plan.free_bytes.map(human_size).unwrap_or_else(|| "inconnu".into());
                ui.label(format!(
                    "Total : {} copie(s), {} à écrire ; espace libre : {free}",
                    plan.to_write(),
                    human_size(plan.total_bytes)
                ));
                if plan.missing_sources > 0 {
                    ui.colored_label(ERROR_COLOR, format!("{} photo(s) d'origine introuvable(s) (déplacées ?)", plan.missing_sources));
                }
                let enough = plan.has_enough_space();
                if !enough {
                    ui.colored_label(ERROR_COLOR, "Espace insuffisant dans la destination.");
                }
                let n = plan.to_write();
                ui.add_space(4.0);
                if ui
                    .add_enabled(enough && n > 0, egui::Button::new(RichText::new(format!("Copier {n} photo(s)")).strong()))
                    .clicked()
                {
                    go = true;
                }
                if n == 0 {
                    ui.label("Rien à copier : tout est déjà en place.");
                }
            });
        }
        None => {}
    }
    if go {
        start(app, ui.ctx());
    }

    if let Some(r) = app.tri.report.clone() {
        ui.add_space(10.0);
        egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
            let title = if r.stopped_on_error.is_some() {
                "Tri interrompu"
            } else if r.cancelled {
                "Tri annulé"
            } else {
                "Tri terminé"
            };
            ui.label(RichText::new(title).size(17.0).strong());
            ui.label(format!(
                "{} copie(s) dont {} renommée(s), {} déjà présente(s)",
                r.copied + r.renamed,
                r.renamed,
                r.already_present
            ));
            if let Some(e) = &r.stopped_on_error {
                ui.colored_label(ERROR_COLOR, format!("Erreur d'écriture : {e}"));
            }
            match &r.log_path {
                Some(l) => ui.label(format!("Journal : {}", l.display())),
                None => ui.colored_label(ERROR_COLOR, "Journal non écrit (destination inaccessible)."),
            };
            if let Some(d) = &app.tri.dest
                && ui.button("Ouvrir le dossier de destination").clicked()
            {
                let _ = std::process::Command::new("explorer").arg(d).spawn();
            }
        });
    }
}
