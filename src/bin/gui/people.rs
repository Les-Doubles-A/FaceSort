//! Écran E2 « Personnes » : nommer les groupes, fusionner, corriger (F3).

use crate::app::{FaceSortApp, Tab};
use eframe::egui::{self, Color32, RichText};
use facesort::scan;
use facesort::store::{Group, NameOutcome, PersonSummary};
use std::collections::HashMap;

const ERROR_COLOR: Color32 = Color32::from_rgb(200, 60, 60);
const POOR_COLOR: Color32 = Color32::from_rgb(190, 120, 20);
const THUMB: f32 = 72.0;
const GROUP_THUMBS: usize = 12;
const PERSON_THUMBS: usize = 60;
const GROUPS_PAGE: usize = 20;

/// Message affiché sous le champ « Prénom » d'un groupe.
#[derive(Clone)]
pub enum Note {
    Invalid(String),
    Exists { id: i64, name: String },
}

pub struct PeopleState {
    pub dirty: bool,
    pub groups: Vec<Group>,
    pub isolated: Vec<i64>,
    pub persons: Vec<PersonSummary>,
    pub names: HashMap<i64, String>,
    pub notes: HashMap<i64, Note>,
    pub shown_groups: usize,
    pub person_faces: HashMap<i64, Vec<i64>>,
    pub rename: HashMap<i64, String>,
    pub merge_into: HashMap<i64, i64>,
    pub error: Option<String>,
    pub info: Option<String>,
}

impl Default for PeopleState {
    fn default() -> Self {
        Self {
            dirty: true,
            groups: Vec::new(),
            isolated: Vec::new(),
            persons: Vec::new(),
            names: HashMap::new(),
            notes: HashMap::new(),
            shown_groups: GROUPS_PAGE,
            person_faces: HashMap::new(),
            rename: HashMap::new(),
            merge_into: HashMap::new(),
            error: None,
            info: None,
        }
    }
}

enum Action {
    NameGroup(i64, String),
    AddGroupTo(i64, i64),
    CancelNote(i64),
    IgnoreGroup(i64),
    SplitGroup(i64),
    RemoveFromGroup(i64),
    RemoveFromPerson(i64, i64),
    Rename(i64, String),
    Merge(i64, i64),
    ConfirmAuto(i64),
    LoadPersonFaces(i64),
    IgnoreFaces(Vec<i64>),
    Refresh,
    ShowMore,
}

fn reload(app: &mut FaceSortApp) {
    let Some(store) = &app.store else { return };
    let st = &mut app.people;
    let result = (|| -> facesort::Result<()> {
        st.groups = store.groups()?;
        st.isolated = store.isolated_faces()?;
        st.persons = store.persons()?;
        Ok(())
    })();
    if let Err(e) = result {
        st.error = Some(format!("Lecture de la base impossible : {e}"));
    }
    let live: std::collections::HashSet<i64> = st.groups.iter().map(|g| g.cluster_id).collect();
    st.names.retain(|k, _| live.contains(k));
    st.notes.retain(|k, _| live.contains(k));
    st.person_faces.clear();
    st.dirty = false;
}

fn apply(app: &mut FaceSortApp, action: Action) {
    let Some(store) = &mut app.store else { return };
    let st = &mut app.people;
    st.error = None;
    let result: facesort::Result<()> = (|| {
        match action {
            Action::NameGroup(cluster, raw) => match store.name_group(cluster, &raw)? {
                NameOutcome::Created(_) => {
                    st.names.remove(&cluster);
                    st.notes.remove(&cluster);
                    st.info = Some(format!("Groupe nommé « {} ».", raw.trim()));
                }
                NameOutcome::Exists { id, name } => {
                    st.notes.insert(cluster, Note::Exists { id, name });
                    return Ok(());
                }
                NameOutcome::Invalid(msg) => {
                    st.notes.insert(cluster, Note::Invalid(msg));
                    return Ok(());
                }
            },
            Action::AddGroupTo(cluster, person) => {
                let n = store.add_group_to_person(cluster, person)?;
                st.names.remove(&cluster);
                st.notes.remove(&cluster);
                st.info = Some(format!("{n} visage(s) ajouté(s)."));
            }
            Action::CancelNote(cluster) => {
                st.notes.remove(&cluster);
                return Ok(());
            }
            Action::IgnoreGroup(cluster) => {
                let faces = store.group_faces(cluster)?;
                store.ignore_faces(&faces)?;
                st.info = Some(format!("{} visage(s) ignoré(s) : ils ne seront ni regroupés ni triés.", faces.len()));
            }
            Action::SplitGroup(cluster) => {
                let params = facesort::scan::ScanOptions::for_store(store)?.cluster;
                let n = facesort::cluster::split_group(store, cluster, params)?;
                st.info = Some(if n >= 2 {
                    format!("Groupe séparé en {n} groupes : nommez chacun, ou fusionnez-les s'il s'agit de la même personne.")
                } else {
                    "Ce groupe semble homogène : retirez les visages en trop avec « Retirer ».".into()
                });
            }
            Action::RemoveFromGroup(face) => store.remove_face_from_group(face)?,
            Action::RemoveFromPerson(face, person) => {
                store.remove_face_from_person(face)?;
                st.person_faces.entry(person).or_default().retain(|f| *f != face);
                st.info = Some("Visage retiré : il ne sera plus attribué à cette personne.".into());
            }
            Action::Rename(person, raw) => match store.rename_person(person, &raw)? {
                NameOutcome::Created(_) => {
                    st.rename.remove(&person);
                    st.info = Some(format!("Renommé en « {} ».", raw.trim()));
                }
                NameOutcome::Exists { name, .. } => {
                    st.error = Some(format!("« {name} » existe déjà : utilisez « Fusionner avec… »."));
                    return Ok(());
                }
                NameOutcome::Invalid(msg) => {
                    st.error = Some(msg);
                    return Ok(());
                }
            },
            Action::Merge(from, into) => {
                store.merge_persons(from, into)?;
                st.merge_into.remove(&from);
                st.info = Some("Personnes fusionnées.".into());
            }
            Action::ConfirmAuto(person) => {
                let n = store.confirm_auto_faces(person)?;
                st.info = Some(format!("{n} visage(s) confirmé(s) : ils servent maintenant de référence."));
            }
            Action::LoadPersonFaces(person) => {
                st.person_faces.insert(person, store.person_faces(person)?);
                return Ok(());
            }
            Action::IgnoreFaces(faces) => {
                store.ignore_faces(&faces)?;
                st.info = Some(format!("{} visage(s) ignoré(s).", faces.len()));
            }
            Action::Refresh => {
                let (auto, groups) = scan::refresh_groups(store, None)?;
                st.info = Some(format!("{auto} visage(s) attribué(s) automatiquement, {groups} groupe(s) à nommer."));
            }
            Action::ShowMore => {
                st.shown_groups += GROUPS_PAGE;
                return Ok(());
            }
        }
        st.dirty = true;
        Ok(())
    })();
    if let Err(e) = result {
        app.people.error = Some(e.to_string());
    }
    if app.people.dirty {
        let keep = std::mem::take(&mut app.people.person_faces);
        reload(app);
        app.people.person_faces = keep;
        app.forget_qualities();
        app.reload();
    }
}

/// Vignette d'un visage ; survol : chemin de la photo.
/// Les visages de mauvaise qualité (sombres, flous, petits…) portent la
/// mention « médiocre » : ce sont eux qui causent le plus de mélanges.
fn thumb(app: &mut FaceSortApp, ui: &mut egui::Ui, face: i64) -> egui::Response {
    let tex = app.thumb(ui.ctx(), face);
    let poor = !facesort::quality::is_good_score(app.face_quality(face));
    let resp = match tex {
        Some(t) => ui.add(egui::Image::from_texture(&t).fit_to_exact_size(egui::vec2(THUMB, THUMB))),
        None => ui.add_sized([THUMB, THUMB], egui::Label::new("?")),
    };
    if poor {
        ui.label(RichText::new("médiocre").small().color(POOR_COLOR))
            .on_hover_text("Photo sombre, floue, peu contrastée ou visage petit : reconnaissance peu fiable");
    }
    if resp.hovered()
        && let Some(info) = app.store.as_ref().and_then(|s| s.face_info(face).ok().flatten())
    {
        return resp.on_hover_text(info.photo_path.display().to_string());
    }
    resp
}

pub fn show(app: &mut FaceSortApp, ui: &mut egui::Ui) {
    if app.people.dirty {
        reload(app);
    }
    ui.horizontal(|ui| {
        ui.heading("Personnes");
        ui.add_space(16.0);
        if ui
            .button("Relancer la reconnaissance")
            .on_hover_text("Attribue aux personnes nommées les visages qui leur ressemblent, puis regroupe le reste")
            .clicked()
        {
            apply(app, Action::Refresh);
        }
    });
    if let Some(e) = &app.people.error {
        ui.colored_label(ERROR_COLOR, e);
    }
    if let Some(i) = &app.people.info {
        ui.label(i);
    }
    ui.add_space(6.0);

    if app.stats.faces == 0 {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Aucun visage à nommer").size(18.0));
            ui.label("Analysez d'abord un dossier de photos.");
            if ui.button("Aller à l'analyse").clicked() {
                app.tab = Tab::Analyse;
            }
        });
        return;
    }

    let mut actions = Vec::new();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        groups_section(app, ui, &mut actions);
        ui.add_space(14.0);
        persons_section(app, ui, &mut actions);
        ui.add_space(14.0);
        isolated_section(app, ui, &mut actions);
    });
    for a in actions {
        apply(app, a);
    }
}

fn groups_section(app: &mut FaceSortApp, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let groups = app.people.groups.clone();
    ui.label(RichText::new(format!("Groupes à nommer ({})", groups.len())).size(16.0).strong());
    if groups.is_empty() {
        ui.label("Aucun groupe à nommer.");
        return;
    }
    let persons = app.people.persons.clone();
    for g in groups.iter().take(app.people.shown_groups) {
        egui::Frame::group(ui.style()).inner_margin(8.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(format!("Groupe de {} visage(s)", g.face_ids.len()));
            ui.horizontal_wrapped(|ui| {
                for &f in g.face_ids.iter().take(GROUP_THUMBS) {
                    ui.vertical(|ui| {
                        thumb(app, ui, f);
                        if ui.small_button("Retirer").on_hover_text("Ce visage n'appartient pas au groupe").clicked() {
                            actions.push(Action::RemoveFromGroup(f));
                        }
                    });
                }
                if g.face_ids.len() > GROUP_THUMBS {
                    ui.label(format!("+{}", g.face_ids.len() - GROUP_THUMBS));
                }
            });
            ui.horizontal(|ui| {
                let buf = app.people.names.entry(g.cluster_id).or_default();
                let r = ui.add(
                    egui::TextEdit::singleline(buf)
                        .hint_text("Prénom")
                        .desired_width(200.0)
                        .id_salt(("nom", g.cluster_id)),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let value = buf.clone();
                if (ui.button("Nommer").clicked() || enter) && !value.trim().is_empty() {
                    actions.push(Action::NameGroup(g.cluster_id, value));
                }
                if !persons.is_empty() {
                    egui::ComboBox::from_id_salt(("ajout", g.cluster_id)).selected_text("Ajouter à…").show_ui(ui, |ui| {
                        for p in &persons {
                            if ui.selectable_label(false, &p.name).clicked() {
                                actions.push(Action::AddGroupTo(g.cluster_id, p.id));
                            }
                        }
                    });
                }
                if g.face_ids.len() >= 3
                    && ui
                        .button("Séparer ce groupe")
                        .on_hover_text("Plusieurs personnes mélangées ? Coupe le groupe là où la ressemblance est la plus faible")
                        .clicked()
                {
                    actions.push(Action::SplitGroup(g.cluster_id));
                }
                if ui.button("Ignorer ce groupe").on_hover_text("Inconnus sans intérêt : ni regroupés, ni triés").clicked() {
                    actions.push(Action::IgnoreGroup(g.cluster_id));
                }
            });
            match app.people.notes.get(&g.cluster_id).cloned() {
                Some(Note::Invalid(msg)) => {
                    ui.colored_label(ERROR_COLOR, msg);
                }
                Some(Note::Exists { id, name }) => {
                    ui.horizontal(|ui| {
                        ui.label(format!("« {name} » existe déjà."));
                        if ui.button(format!("Ajouter ces visages à {name}")).clicked() {
                            actions.push(Action::AddGroupTo(g.cluster_id, id));
                        }
                        if ui.button("Annuler").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            actions.push(Action::CancelNote(g.cluster_id));
                        }
                    });
                }
                None => {}
            }
        });
        ui.add_space(6.0);
    }
    if groups.len() > app.people.shown_groups && ui.button("Afficher plus de groupes").clicked() {
        actions.push(Action::ShowMore);
    }
}

fn persons_section(app: &mut FaceSortApp, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let persons = app.people.persons.clone();
    ui.label(RichText::new(format!("Personnes nommées ({})", persons.len())).size(16.0).strong());
    if persons.is_empty() {
        ui.label("Aucune personne nommée pour l'instant : donnez un prénom à un groupe ci-dessus.");
        return;
    }
    for p in &persons {
        let header = format!("{} : {} visage(s) dont {} auto, sur {} photo(s)", p.name, p.faces, p.auto_faces, p.photos);
        egui::CollapsingHeader::new(header).id_salt(("personne", p.id)).show(ui, |ui| {
            match app.people.person_faces.get(&p.id).cloned() {
                None => actions.push(Action::LoadPersonFaces(p.id)),
                Some(faces) => {
                    ui.horizontal_wrapped(|ui| {
                        for &f in faces.iter().take(PERSON_THUMBS) {
                            ui.vertical(|ui| {
                                thumb(app, ui, f);
                                if ui
                                    .small_button("Retirer")
                                    .on_hover_text("Ce n'est pas cette personne : le visage ne lui sera plus attribué")
                                    .clicked()
                                {
                                    actions.push(Action::RemoveFromPerson(f, p.id));
                                }
                            });
                        }
                        if faces.len() > PERSON_THUMBS {
                            ui.label(format!("+{}", faces.len() - PERSON_THUMBS));
                        }
                    });
                }
            }
            ui.horizontal(|ui| {
                let buf = app.people.rename.entry(p.id).or_insert_with(|| p.name.clone());
                let r = ui.add(egui::TextEdit::singleline(buf).desired_width(180.0).id_salt(("renommer", p.id)));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let value = buf.clone();
                if (ui.button("Renommer").clicked() || enter) && value.trim() != p.name {
                    actions.push(Action::Rename(p.id, value));
                }
                let others: Vec<_> = persons.iter().filter(|o| o.id != p.id).collect();
                if !others.is_empty() {
                    let target = app.people.merge_into.get(&p.id).copied();
                    let label = target
                        .and_then(|t| others.iter().find(|o| o.id == t))
                        .map(|o| o.name.clone())
                        .unwrap_or_else(|| "Fusionner avec…".into());
                    egui::ComboBox::from_id_salt(("fusion", p.id)).selected_text(label).show_ui(ui, |ui| {
                        for o in &others {
                            if ui.selectable_label(target == Some(o.id), &o.name).clicked() {
                                app.people.merge_into.insert(p.id, o.id);
                            }
                        }
                    });
                    if let Some(t) = target
                        && let Some(o) = others.iter().find(|o| o.id == t)
                        && ui.button(format!("Fusionner « {} » dans « {} »", p.name, o.name)).clicked()
                    {
                        actions.push(Action::Merge(p.id, t));
                    }
                }
                if p.auto_faces > 0
                    && ui
                        .button(format!("Confirmer les {} visage(s) auto", p.auto_faces))
                        .on_hover_text("Après vérification : ces visages serviront de référence pour la reconnaissance")
                        .clicked()
                {
                    actions.push(Action::ConfirmAuto(p.id));
                }
            });
        });
    }
}

fn isolated_section(app: &mut FaceSortApp, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let isolated = app.people.isolated.clone();
    if isolated.is_empty() {
        return;
    }
    egui::CollapsingHeader::new(RichText::new(format!("Visages isolés ({})", isolated.len())).size(16.0).strong())
        .id_salt("isoles")
        .show(ui, |ui| {
            ui.label("Visages sans ressemblant parmi les visages non nommés. Ils seront regroupés si d'autres photos de la même personne sont analysées.");
            ui.horizontal_wrapped(|ui| {
                for &f in isolated.iter().take(PERSON_THUMBS) {
                    thumb(app, ui, f);
                }
            });
            if ui.button("Ignorer tous les visages isolés").clicked() {
                actions.push(Action::IgnoreFaces(isolated.clone()));
            }
        });
}
