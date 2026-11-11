//! Écran E1 « Analyse » : choix des dossiers, progression, résumé et erreurs.

use crate::app::{FaceSortApp, Tab};
use eframe::egui::{self, Color32, RichText};
use facesort::scan::{ScanReport, human_duration};

const ERROR_COLOR: Color32 = Color32::from_rgb(200, 60, 60);

fn pick_folder(app: &mut FaceSortApp) {
    if let Some(dir) = rfd::FileDialog::new().set_title("Dossier de photos à analyser").pick_folder() {
        app.add_root(dir);
    }
}

pub fn show(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    ui.heading("Analyse");
    if let Some(err) = &app.store_error {
        ui.colored_label(ERROR_COLOR, err);
        return;
    }
    ui.add_space(6.0);

    if app.job.is_some() {
        running(app, ui);
        return;
    }

    let nothing_yet = app.stats.photos == 0 && app.roots.is_empty() && app.last.is_none();
    if nothing_yet {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Aucun dossier analysé").size(18.0));
            ui.label("Choisissez un dossier de photos (JPEG, PNG) : FaceSort y cherche les visages, sans rien envoyer en ligne.");
            ui.add_space(8.0);
            if ui.button(RichText::new("Choisir un dossier…").size(16.0)).clicked() {
                pick_folder(app);
            }
        });
        return;
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        model_section(app, ui);
        ui.add_space(8.0);
        folders(app, ui);
        ui.add_space(12.0);
        match app.last.clone() {
            Some(Ok(report)) => success(app, ui, &report),
            Some(Err(msg)) => {
                ui.colored_label(ERROR_COLOR, format!("L'analyse a échoué : {msg}"));
                if msg.contains("modèle") {
                    ui.label("Vérifiez que le dossier « models » est à côté de FaceSort.exe (voir le README).");
                }
            }
            None => {}
        }
    });
}

/// Modèle de reconnaissance de la base, et passage à l'autre modèle.
fn model_section(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    use facesort::embed::EmbedderKind;
    let Some(current) = app.model else { return };
    let other = match current {
        EmbedderKind::SFace => EmbedderKind::AuraFace,
        EmbedderKind::AuraFace => EmbedderKind::SFace,
    };
    ui.horizontal(|ui| {
        ui.label(format!("Modèle de reconnaissance : {}", current.label()));
        if facesort::models::is_installed(other, None) {
            let (label, hint) = match other {
                EmbedderKind::AuraFace => (
                    "Passer à AuraFace (moins de confusions)",
                    "Recalcule les empreintes avec AuraFace, plus fiable sur les visages ressemblants et les photos mal éclairées, mais plus lent. Vos noms, retraits et groupes ignorés sont conservés.",
                ),
                EmbedderKind::SFace => (
                    "Revenir à SFace (plus rapide)",
                    "Recalcule les empreintes avec SFace, plus rapide mais qui confond davantage. Vos noms sont conservés.",
                ),
            };
            if ui.button(label).on_hover_text(hint).clicked() {
                app.start_convert(ui.ctx(), other);
            }
        }
    });
    match &app.notice {
        Some(Ok(m)) => {
            ui.label(m);
        }
        Some(Err(e)) => {
            ui.colored_label(ERROR_COLOR, format!("Conversion impossible : {e}"));
        }
        None => {}
    }
}

fn folders(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    ui.label(RichText::new("Dossiers à analyser").strong());
    let mut remove = None;
    if app.roots.is_empty() {
        ui.label("Aucun dossier choisi.");
    }
    for (i, r) in app.roots.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(r.display().to_string());
            if ui.small_button("Retirer").on_hover_text("Retirer ce dossier de la liste").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        app.roots.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("Ajouter un dossier…").clicked() {
            pick_folder(app);
        }
        let can = !app.roots.is_empty();
        if ui.add_enabled(can, egui::Button::new(RichText::new("Analyser").strong())).clicked() {
            let roots = app.roots.clone();
            app.start_scan(ui.ctx(), roots);
        }
        if !app.known_roots.is_empty() {
            let label = format!("Réanalyser les dossiers connus ({})", app.known_roots.len());
            if ui
                .button(label)
                .on_hover_text("Seules les photos nouvelles ou modifiées sont analysées")
                .clicked()
            {
                let roots = app.known_roots.clone();
                app.start_scan(ui.ctx(), roots);
            }
        }
    });
}

fn running(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    let Some(job) = &app.job else { return };
    let p = &job.progress;
    let title = if job.converting { "Conversion de la base en cours" } else { "Analyse en cours" };
    ui.label(RichText::new(title).strong());
    ui.add(egui::ProgressBar::new(p.fraction()).show_percentage().animate(true));
    ui.label(p.label());
    let eta = p.eta(job.started.elapsed()).map(human_duration).unwrap_or_else(|| "calcul…".into());
    ui.label(format!("Temps écoulé : {}  ·  restant : {eta}", human_duration(job.started.elapsed())));
    if let Some(cur) = &p.current {
        ui.label(RichText::new(cur.display().to_string()).small());
    }
    ui.add_space(8.0);
    if job.is_cancelling() {
        ui.label("Annulation… (la photo en cours se termine)");
    } else if ui.button("Annuler").on_hover_text("Les photos déjà analysées restent enregistrées").clicked() {
        job.cancel();
    }
}

fn success(app: &mut FaceSortApp, ui: &mut egui::Ui, r: &ScanReport) {
    egui::Frame::group(ui.style()).inner_margin(10.0).show(ui, |ui| {
        let title = if r.cancelled { "Analyse annulée" } else { "Analyse terminée" };
        ui.label(RichText::new(title).size(17.0).strong());
        ui.label(format!(
            "{} photos analysées, {} inchangées, {} visages trouvés, {} erreurs",
            r.analyzed,
            r.unchanged,
            r.faces_found,
            r.errors.len()
        ));
        ui.label(format!(
            "{} visages reconnus automatiquement, {} groupes à nommer",
            r.auto_assigned, r.groups
        ));
        if r.quality_backfilled > 0 || r.auto_reset > 0 {
            ui.label(format!(
                "Mise à niveau : {} visage(s) déjà connus notés en qualité, {} attribution(s) automatique(s) réévaluée(s) avec les nouveaux seuils",
                r.quality_backfilled, r.auto_reset
            ));
        }
        ui.label(format!(
            "Durée : {} ({:.2} s par photo)  ·  {}",
            human_duration(r.elapsed),
            r.seconds_per_photo(),
            r.ignored_summary()
        ));
        ui.add_space(6.0);
        if ui.button(RichText::new("Voir les personnes").strong()).clicked() {
            app.tab = Tab::Personnes;
        }
    });
    if !r.errors.is_empty() {
        ui.add_space(8.0);
        egui::CollapsingHeader::new(RichText::new(format!("{} fichier(s) en erreur", r.errors.len())).color(ERROR_COLOR))
            .default_open(r.errors.len() <= 5)
            .show(ui, |ui| {
                for (path, reason) in &r.errors {
                    ui.label(format!("{} : {reason}", path.display()));
                }
            });
    }
}
