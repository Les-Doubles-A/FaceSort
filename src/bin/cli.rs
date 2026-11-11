//! `facesort` : interface en ligne de commande (F6).
//!
//! Codes de sortie : 0 succès, 1 erreur fatale, 2 arguments invalides,
//! 3 succès partiel (au moins un fichier en erreur).

use clap::{Parser, Subcommand};
use facesort::engine::Engine;
use facesort::scan;
use facesort::sort::{self, SortOptions};
use facesort::store::{self, Store};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

const EXIT_FATAL: u8 = 1;
const EXIT_PARTIAL: u8 = 3;

#[derive(Parser)]
#[command(
    name = "facesort-cli",
    version,
    about = "Range des photos par personne grâce à la détection et la reconnaissance faciales, 100 % en local.",
    after_help = "Exemples :\n  facesort-cli scan D:\\Photos\\2024\n  facesort-cli sort E:\\Tri --dry-run\n  facesort-cli status --json\n\n\
Nommez les groupes de visages dans l'application FaceSort (fenêtre) entre « scan » et « sort ».\n\
Codes de sortie : 0 succès, 1 erreur fatale, 2 arguments invalides, 3 succès partiel."
)]
struct Cli {
    /// Fichier de base de données (par défaut : %LOCALAPPDATA%\FaceSort\facesort.db).
    #[arg(long, global = true, value_name = "FICHIER")]
    db: Option<PathBuf>,
    /// Dossier contenant les modèles ONNX et SHA256SUMS.
    #[arg(long, global = true, value_name = "DOSSIER")]
    models: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyse des dossiers de photos (JPEG, PNG), récursivement.
    ///
    /// Les photos déjà analysées et inchangées sont sautées. En fin d'analyse,
    /// les visages des personnes connues leur sont attribués et les autres
    /// sont regroupés par ressemblance.
    #[command(after_help = "Exemple :\n  facesort-cli scan D:\\Photos\\2024 D:\\Photos\\2025 --json")]
    Scan {
        /// Dossiers à analyser.
        #[arg(required = true, value_name = "DOSSIER")]
        dirs: Vec<PathBuf>,
        /// Résumé au format JSON sur la sortie standard.
        #[arg(long)]
        json: bool,
        /// Pas de progression sur la sortie d'erreur.
        #[arg(long, short)]
        quiet: bool,
    },
    /// Copie les photos dans DESTINATION\<Prénom>\ (les originaux ne sont pas modifiés).
    ///
    /// Une photo avec plusieurs personnes est copiée dans le dossier de
    /// chacune. Les photos ne contenant que des visages non nommés vont dans
    /// _Inconnus\, sauf avec --no-unknown. Un journal CSV est écrit dans la destination.
    #[command(after_help = "Exemples :\n  facesort-cli sort E:\\Tri --dry-run\n  facesort-cli sort E:\\Tri --no-unknown")]
    Sort {
        /// Dossier de destination (hors des dossiers analysés).
        #[arg(value_name = "DESTINATION")]
        dest: PathBuf,
        /// Affiche le plan (copies par personne, taille, espace libre) sans rien écrire.
        #[arg(long)]
        dry_run: bool,
        /// Ne copie pas les photos dont aucun visage n'est nommé.
        #[arg(long)]
        no_unknown: bool,
        /// Résultat au format JSON sur la sortie standard.
        #[arg(long)]
        json: bool,
    },
    /// Affiche le contenu de la base : photos, visages, personnes.
    #[command(after_help = "Exemple :\n  facesort-cli status --json")]
    Status {
        /// Résultat au format JSON sur la sortie standard.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("Erreur : {e:#}");
            ExitCode::from(EXIT_FATAL)
        }
    }
}

fn open_store(db: &Option<PathBuf>) -> anyhow::Result<Store> {
    let path = db.clone().unwrap_or_else(store::default_db_path);
    Store::open(&path).map_err(|e| anyhow::anyhow!("base {} illisible : {e}", path.display()))
}

fn run(cli: Cli) -> anyhow::Result<u8> {
    match cli.cmd {
        Cmd::Scan { dirs, json, quiet } => {
            if let Some(missing) = dirs.iter().find(|d| !d.is_dir()) {
                anyhow::bail!("dossier introuvable : {}", missing.display());
            }
            let mut store = open_store(&cli.db)?;
            let mut engine = Engine::load(cli.models.as_deref())?;
            let show = !quiet && std::io::stderr().is_terminal();
            let cancel = AtomicBool::new(false);
            let report = scan::scan(
                &mut store,
                &mut engine,
                &dirs,
                None,
                |p| {
                    if show {
                        eprint!("\r{}/{} photos, {} visages, {} erreurs   ", p.done, p.total, p.faces, p.errors);
                        let _ = std::io::stderr().flush();
                    }
                },
                &cancel,
            )?;
            if show {
                eprintln!();
            }
            for (path, err) in &report.errors {
                eprintln!("  {} : {err}", path.display());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", report.summary());
            }
            Ok(if report.errors.is_empty() { 0 } else { EXIT_PARTIAL })
        }
        Cmd::Sort { dest, dry_run, no_unknown, json } => {
            let store = open_store(&cli.db)?;
            let photos = store.photos_for_sort()?;
            let roots = store.roots()?;
            let opts = SortOptions { dest, include_unknown: !no_unknown };
            let plan = sort::plan(&photos, &roots, &opts)?;
            if store.stats()?.persons == 0 {
                eprintln!("Aucune personne nommée : ouvrez FaceSort pour nommer les groupes de visages.");
            }
            if dry_run {
                if json {
                    println!("{}", serde_json::to_string_pretty(&plan)?);
                } else {
                    println!("Aperçu (rien n'est écrit) : {}", plan.summary());
                }
                return Ok(0);
            }
            if !plan.has_enough_space() {
                anyhow::bail!("espace insuffisant dans la destination : {}", plan.summary());
            }
            let show = std::io::stderr().is_terminal();
            let cancel = AtomicBool::new(false);
            let report = sort::execute(
                &plan,
                |done, total| {
                    if show {
                        eprint!("\r{done}/{total} copies   ");
                        let _ = std::io::stderr().flush();
                    }
                },
                &cancel,
            );
            if show {
                eprintln!();
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", report.summary());
            }
            Ok(if report.stopped_on_error.is_some() || report.missing_sources > 0 { EXIT_PARTIAL } else { 0 })
        }
        Cmd::Status { json } => {
            let store = open_store(&cli.db)?;
            let stats = store.stats()?;
            let people = store.persons()?;
            if json {
                let v = serde_json::json!({
                    "db": store.path(),
                    "photos": stats.photos,
                    "photos_with_errors": stats.photos_with_errors,
                    "faces": stats.faces,
                    "persons": stats.persons,
                    "unassigned_faces": stats.unassigned_faces,
                    "groups": stats.groups,
                    "people": people,
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else if stats.photos == 0 {
                println!("Base vide ({}). Lancez « facesort-cli scan <dossier> ».", store.path().display());
            } else {
                println!("Base : {}", store.path().display());
                println!(
                    "{} photos ({} en erreur), {} visages, {} personnes, {} visages non attribués en {} groupes",
                    stats.photos, stats.photos_with_errors, stats.faces, stats.persons, stats.unassigned_faces, stats.groups
                );
                for p in people {
                    println!("  {} : {} visages ({} auto) sur {} photos", p.name, p.faces, p.auto_faces, p.photos);
                }
            }
            Ok(0)
        }
    }
}
