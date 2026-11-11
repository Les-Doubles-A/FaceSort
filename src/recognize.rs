//! Reconnaissance des personnes déjà nommées (F4).
//!
//! Un visage est comparé aux visages confirmés de chaque personne (similarité
//! maximale). Il est attribué si la meilleure personne dépasse le seuil et
//! devance la deuxième d'au moins la marge ; sinon il reste non attribué.
//! Les visages « auto » ne servent jamais de référence (ADR-3).

use crate::embed::{EmbedderKind, cosine};
use crate::quality::is_good_score;
use crate::store::{Assignment, Store};
use crate::{Result, thresholds};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecognizeParams {
    pub threshold: f32,
    pub margin: f32,
    /// Seuil relevé pour un visage de mauvaise qualité (sombre, flou, petit…).
    pub low_quality_penalty: f32,
}

impl RecognizeParams {
    pub fn for_kind(kind: EmbedderKind) -> Self {
        let t = thresholds::for_kind(kind);
        Self { threshold: t.recognition, margin: t.margin, low_quality_penalty: t.low_quality_penalty }
    }

    fn for_quality(self, good: bool) -> Self {
        if good { self } else { Self { threshold: self.threshold + self.low_quality_penalty, ..self } }
    }
}

impl Default for RecognizeParams {
    fn default() -> Self {
        Self::for_kind(crate::models::DEFAULT_EMBEDDER)
    }
}

/// Meilleure personne pour `emb`, ou `None` (inconnu, ambigu ou exclu).
pub fn best_match(
    emb: &[f32],
    refs: &[(i64, Vec<f32>)],
    excluded: Option<&HashSet<i64>>,
    p: RecognizeParams,
) -> Option<i64> {
    let mut per_person: HashMap<i64, f32> = HashMap::new();
    for (person, r) in refs {
        let s = cosine(emb, r);
        let e = per_person.entry(*person).or_insert(f32::MIN);
        if s > *e {
            *e = s;
        }
    }
    let mut ranked: Vec<(i64, f32)> = per_person.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let (best, best_sim) = *ranked.first()?;
    if best_sim < p.threshold {
        return None;
    }
    if let Some((_, second)) = ranked.get(1)
        && best_sim - second < p.margin
    {
        return None;
    }
    if excluded.is_some_and(|x| x.contains(&best)) {
        return None;
    }
    Some(best)
}

/// Attribue automatiquement les visages non attribués ; renvoie leur nombre.
pub fn recognize_unassigned(store: &mut Store, p: RecognizeParams) -> Result<usize> {
    let refs = store.confirmed_embeddings()?;
    if refs.is_empty() {
        return Ok(0);
    }
    let exclusions = store.exclusions()?;
    let faces = store.unassigned_embeddings()?;
    let matches: Vec<(i64, i64)> = faces
        .par_iter()
        .filter_map(|f| {
            let fp = p.for_quality(is_good_score(f.quality));
            best_match(&f.embedding, &refs, exclusions.get(&f.id), fp).map(|person| (f.id, person))
        })
        .collect();
    let mut by_person: HashMap<i64, Vec<i64>> = HashMap::new();
    for (face, person) in matches {
        by_person.entry(person).or_default().push(face);
    }
    let mut n = 0;
    for (person, faces) in by_person {
        n += store.assign_faces(&faces, person, Assignment::Auto)?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::tests::{Rng, around};

    type Refs = Vec<(i64, Vec<f32>)>;

    fn refs(rng: &mut Rng) -> (Vec<f32>, Vec<f32>, Refs) {
        let a = rng.unit(128);
        let b = rng.unit(128);
        let mut r = Vec::new();
        for _ in 0..4 {
            r.push((1, around(rng, &a, 0.5)));
            r.push((2, around(rng, &b, 0.5)));
        }
        (a, b, r)
    }

    #[test]
    fn known_person_is_recognized() {
        let mut rng = Rng::new(5);
        let (a, b, r) = refs(&mut rng);
        assert_eq!(best_match(&around(&mut rng, &a, 0.5), &r, None, RecognizeParams::default()), Some(1));
        assert_eq!(best_match(&around(&mut rng, &b, 0.5), &r, None, RecognizeParams::default()), Some(2));
    }

    #[test]
    fn stranger_is_not_recognized() {
        let mut rng = Rng::new(6);
        let (_, _, r) = refs(&mut rng);
        assert_eq!(best_match(&rng.unit(128), &r, None, RecognizeParams::default()), None);
    }

    // AC-F4-3
    #[test]
    fn ambiguous_margin() {
        let p = RecognizeParams { threshold: 0.4, margin: 0.05, low_quality_penalty: 0.1 };
        // Références orthogonales construites à la main : similarités exactes.
        let mut e1 = vec![0f32; 4];
        e1[0] = 1.0;
        let mut e2 = vec![0f32; 4];
        e2[1] = 1.0;
        let r = vec![(1, e1), (2, e2)];
        let face = |s1: f32, s2: f32| {
            let rest = (1.0 - s1 * s1 - s2 * s2).max(0.0).sqrt();
            vec![s1, s2, rest, 0.0]
        };
        assert_eq!(best_match(&face(0.60, 0.57), &r, None, p), None, "écart 0,03 < 0,05");
        assert_eq!(best_match(&face(0.60, 0.50), &r, None, p), Some(1), "écart 0,10");
        assert_eq!(best_match(&face(0.30, 0.10), &r, None, p), None, "sous le seuil");
    }

    #[test]
    fn low_quality_needs_higher_similarity() {
        let p = RecognizeParams { threshold: 0.4, margin: 0.05, low_quality_penalty: 0.1 };
        let r = vec![(1, vec![1.0, 0.0, 0.0])];
        let face = vec![0.45, (1.0f32 - 0.45 * 0.45).sqrt(), 0.0];
        assert_eq!(best_match(&face, &r, None, p.for_quality(true)), Some(1));
        assert_eq!(best_match(&face, &r, None, p.for_quality(false)), None);
    }

    #[test]
    fn excluded_person_is_not_assigned() {
        let mut rng = Rng::new(8);
        let (a, _, r) = refs(&mut rng);
        let ex = HashSet::from([1]);
        assert_eq!(best_match(&around(&mut rng, &a, 0.5), &r, Some(&ex), RecognizeParams::default()), None);
    }
}
