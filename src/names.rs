//! Validation des prénoms, qui deviennent des noms de dossier Windows (F3).

pub const MAX_NAME_CHARS: usize = 64;
/// Dossier réservé aux photos ne contenant que des visages non nommés.
pub const UNKNOWN_DIR: &str = "_Inconnus";

const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Renvoie le nom nettoyé (espaces de bord retirés) ou la raison du refus.
pub fn validate_person_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("le nom est vide".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("{MAX_NAME_CHARS} caractères maximum"));
    }
    if let Some(c) = name.chars().find(|c| FORBIDDEN.contains(c) || c.is_control()) {
        let shown = if c.is_control() { "caractère de contrôle".to_string() } else { format!("« {c} »") };
        return Err(format!("caractère interdit {shown}"));
    }
    if name.ends_with('.') {
        return Err("le nom ne doit pas finir par un point".into());
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(format!("« {stem} » est un nom réservé par Windows"));
    }
    if name.eq_ignore_ascii_case(UNKNOWN_DIR) {
        return Err(format!("« {UNKNOWN_DIR} » est réservé par FaceSort"));
    }
    Ok(name.to_string())
}

/// Message complet affiché à l'utilisateur (AC-F3-3).
pub fn invalid_message(reason: &str) -> String {
    format!("Nom de dossier invalide sous Windows : {reason}")
}

/// Comparaison de noms insensible à la casse, accents compris (« Léa » = « LÉA »).
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_names() {
        assert_eq!(validate_person_name("  Léa ").unwrap(), "Léa");
        assert_eq!(validate_person_name("Jean-Pierre O'Neil").unwrap(), "Jean-Pierre O'Neil");
        assert_eq!(validate_person_name("Zoé 2").unwrap(), "Zoé 2");
    }

    #[test]
    fn rejects_forbidden_characters() {
        let e = validate_person_name("Tom/Léa").unwrap_err();
        assert!(e.contains("« / »"), "{e}");
        for bad in ["a<b", "a>b", "a:b", "a\"b", "a\\b", "a|b", "a?b", "a*b", "a\tb"] {
            assert!(validate_person_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_reserved_names() {
        for bad in ["CON", "con", "Com1", "LPT9", "nul.txt", "_inconnus"] {
            assert!(validate_person_name(bad).is_err(), "{bad}");
        }
        assert!(validate_person_name("Connie").is_ok());
        assert!(validate_person_name("COM10").is_ok());
    }

    #[test]
    fn rejects_empty_long_and_trailing_dot() {
        assert!(validate_person_name("   ").is_err());
        assert!(validate_person_name(&"é".repeat(65)).is_err());
        assert!(validate_person_name(&"é".repeat(64)).is_ok());
        assert!(validate_person_name("Léa.").is_err());
    }

    #[test]
    fn message_format() {
        let reason = validate_person_name("CON").unwrap_err();
        assert!(invalid_message(&reason).starts_with("Nom de dossier invalide sous Windows : "));
    }

    #[test]
    fn same_name_folds_case_and_accents_case() {
        assert!(same_name("Léa", "LÉA"));
        assert!(same_name("léa", "Léa "));
        assert!(!same_name("Lea", "Léa"));
    }
}
