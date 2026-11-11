//! Regroupement des visages non attribués (F2).
//!
//! DBSCAN sur la similarité cosinus, avec trois protections contre les
//! mélanges de personnes (mieux vaut deux groupes à fusionner qu'un groupe
//! mélangé à trier) :
//! 1. un visage de mauvaise qualité (sombre, flou, petit…) n'est jamais
//!    « cœur » : il peut rejoindre un groupe mais pas en relier deux ;
//! 2. une paire impliquant un visage de mauvaise qualité doit être plus
//!    ressemblante (`low_quality_penalty`) ;
//! 3. un membre trop éloigné du centre de son groupe en est détaché
//!    (`centroid_min`), ce qui limite l'effet de chaîne.
//!
//! Parcours dans l'ordre des indices : le résultat est déterministe (AC-F2-3).

use crate::embed::{EmbedderKind, cosine, l2_normalize};
use crate::quality::is_good_score;
use crate::store::Store;
use crate::{Result, thresholds};
use rayon::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClusterParams {
    pub threshold: f32,
    pub min_samples: usize,
    pub low_quality_penalty: f32,
    pub centroid_min: f32,
}

impl ClusterParams {
    pub fn for_kind(kind: EmbedderKind) -> Self {
        let t = thresholds::for_kind(kind);
        Self {
            threshold: t.cluster,
            min_samples: t.min_samples,
            low_quality_penalty: t.low_quality_penalty,
            centroid_min: t.centroid_min,
        }
    }

    /// Paramètres plus stricts de `step` (pour « Séparer ce groupe »).
    pub fn stricter(self, step: f32) -> Self {
        Self { threshold: self.threshold + step, centroid_min: self.centroid_min + step, ..self }
    }
}

impl Default for ClusterParams {
    fn default() -> Self {
        Self::for_kind(crate::models::DEFAULT_EMBEDDER)
    }
}

/// Renvoie une étiquette de groupe par empreinte (`None` = isolé), numérotées
/// dans l'ordre d'apparition. `good[i]` : le visage i est de bonne qualité.
pub fn dbscan(embs: &[Vec<f32>], good: &[bool], p: ClusterParams) -> Vec<Option<usize>> {
    let n = embs.len();
    assert_eq!(good.len(), n);
    let neighbors: Vec<Vec<usize>> = (0..n)
        .into_par_iter()
        .map(|i| {
            (0..n)
                .filter(|&j| {
                    if j == i {
                        return true;
                    }
                    let need = if good[i] && good[j] { p.threshold } else { p.threshold + p.low_quality_penalty };
                    cosine(&embs[i], &embs[j]) >= need
                })
                .collect()
        })
        .collect();
    let is_core: Vec<bool> = (0..n).map(|i| good[i] && neighbors[i].len() >= p.min_samples.max(1)).collect();
    let mut labels: Vec<Option<usize>> = vec![None; n];
    let mut next = 0;
    for start in 0..n {
        if labels[start].is_some() || !is_core[start] {
            continue;
        }
        labels[start] = Some(next);
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            for &j in &neighbors[i] {
                if labels[j].is_none() {
                    labels[j] = Some(next);
                    if is_core[j] {
                        stack.push(j);
                    }
                }
            }
        }
        next += 1;
    }
    prune_far_members(embs, good, &mut labels, p.centroid_min);
    renumber(&mut labels);
    labels
}

/// Détache les membres trop éloignés du centre (calculé sur les visages de
/// bonne qualité du groupe) ; un groupe réduit à un visage disparaît.
fn prune_far_members(embs: &[Vec<f32>], good: &[bool], labels: &mut [Option<usize>], min_sim: f32) {
    let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, l) in labels.iter().enumerate() {
        if let Some(l) = l {
            members.entry(*l).or_default().push(i);
        }
    }
    for idx in members.values() {
        let basis: Vec<usize> = {
            let g: Vec<usize> = idx.iter().copied().filter(|&i| good[i]).collect();
            if g.is_empty() { idx.clone() } else { g }
        };
        let dim = embs[idx[0]].len();
        let mut c = vec![0f32; dim];
        for &i in &basis {
            c.iter_mut().zip(&embs[i]).for_each(|(a, b)| *a += b);
        }
        l2_normalize(&mut c);
        for &i in idx {
            if cosine(&embs[i], &c) < min_sim {
                labels[i] = None;
            }
        }
    }
    // Groupes devenus trop petits
    let mut count: BTreeMap<usize, usize> = BTreeMap::new();
    for l in labels.iter().flatten() {
        *count.entry(*l).or_default() += 1;
    }
    for l in labels.iter_mut() {
        if l.is_some_and(|v| count[&v] < 2) {
            *l = None;
        }
    }
}

fn renumber(labels: &mut [Option<usize>]) {
    let mut map: BTreeMap<usize, usize> = BTreeMap::new();
    for l in labels.iter_mut().flatten() {
        let n = map.len();
        *l = *map.entry(*l).or_insert(n);
    }
}

/// Regroupe tous les visages non attribués de la base et renvoie le nombre de groupes.
pub fn recluster(store: &mut Store, p: ClusterParams) -> Result<usize> {
    let faces = store.unassigned_embeddings()?;
    let embs: Vec<Vec<f32>> = faces.iter().map(|f| f.embedding.clone()).collect();
    let good: Vec<bool> = faces.iter().map(|f| is_good_score(f.quality)).collect();
    let labels = dbscan(&embs, &good, p);
    let groups = labels.iter().flatten().max().map_or(0, |m| m + 1);
    let pairs: Vec<(i64, Option<i64>)> =
        faces.iter().zip(&labels).map(|(f, l)| (f.id, l.map(|v| v as i64))).collect();
    store.set_clusters(&pairs)?;
    Ok(groups)
}

/// « Séparer ce groupe » : regroupe à nouveau ses seuls visages en relevant
/// le seuil jusqu'à la plus petite valeur qui coupe le groupe en deux (on
/// coupe le lien le plus faible). Les seuils essayés sont pris parmi les
/// similarités internes au groupe, car deux personnes très ressemblantes
/// peuvent rester liées bien au-dessus du seuil normal.
/// Renvoie le nombre de groupes obtenus (1 : le groupe semble homogène).
pub fn split_group(store: &mut Store, cluster_id: i64, p: ClusterParams) -> Result<usize> {
    let faces = store.group_embeddings(cluster_id)?;
    if faces.len() < 2 {
        return Ok(faces.len());
    }
    let embs: Vec<Vec<f32>> = faces.iter().map(|f| f.embedding.clone()).collect();
    let good: Vec<bool> = faces.iter().map(|f| is_good_score(f.quality)).collect();
    let mut sims: Vec<f32> = Vec::new();
    for i in 0..embs.len() {
        for j in i + 1..embs.len() {
            let s = cosine(&embs[i], &embs[j]);
            if s > p.threshold {
                sims.push(s);
            }
        }
    }
    sims.sort_by(f32::total_cmp);
    sims.dedup();
    // Au plus 60 essais, répartis sur les similarités observées.
    let step = (sims.len() / 60).max(1);
    let candidates: Vec<f32> = sims.iter().step_by(step).map(|s| s + 1e-4).collect();
    let mut labels = vec![Some(0); embs.len()];
    for t in candidates {
        let trial = dbscan(&embs, &good, ClusterParams { threshold: t, ..p });
        let groups = trial.iter().flatten().collect::<std::collections::HashSet<_>>().len();
        if groups >= 2 {
            labels = trial;
            break;
        }
    }
    let base = store.next_cluster_id()?;
    let pairs: Vec<(i64, Option<i64>)> =
        faces.iter().zip(&labels).map(|(f, l)| (f.id, l.map(|v| base + v as i64))).collect();
    store.relabel(&pairs)?;
    Ok(labels.iter().flatten().collect::<std::collections::HashSet<_>>().len())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Générateur pseudo-aléatoire déterministe (xorshift).
    pub struct Rng(u64);
    impl Rng {
        pub fn new(seed: u64) -> Self {
            Self(seed.max(1))
        }
        pub fn next_f32(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        }
        pub fn unit(&mut self, dim: usize) -> Vec<f32> {
            let mut v: Vec<f32> = (0..dim).map(|_| self.next_f32()).collect();
            l2_normalize(&mut v);
            v
        }
    }

    /// Empreinte proche de `center` (similarité ~ 1 - noise²/2).
    pub fn around(rng: &mut Rng, center: &[f32], noise: f32) -> Vec<f32> {
        let mut v: Vec<f32> =
            center.iter().map(|c| c + noise * rng.next_f32() / (center.len() as f32).sqrt() * 1.7).collect();
        l2_normalize(&mut v);
        v
    }

    fn mix(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
        let mut v: Vec<f32> = a.iter().zip(b).map(|(x, y)| x * (1.0 - t) + y * t).collect();
        l2_normalize(&mut v);
        v
    }

    fn params() -> ClusterParams {
        ClusterParams { threshold: 0.45, min_samples: 2, low_quality_penalty: 0.10, centroid_min: 0.40 }
    }

    fn three_clouds(rng: &mut Rng) -> (Vec<Vec<f32>>, Vec<usize>) {
        let centers: Vec<_> = (0..3).map(|_| rng.unit(128)).collect();
        let mut embs = Vec::new();
        let mut truth = Vec::new();
        for _ in 0..6 {
            for (c, center) in centers.iter().enumerate() {
                embs.push(around(rng, center, 0.6));
                truth.push(c);
            }
        }
        (embs, truth)
    }

    fn assert_pure(labels: &[Option<usize>], truth: &[usize]) {
        for (i, li) in labels.iter().enumerate() {
            for (j, lj) in labels.iter().enumerate() {
                if li.is_some() && li == lj {
                    assert_eq!(truth[i], truth[j], "groupe impur");
                }
            }
        }
    }

    #[test]
    fn separated_clouds_give_pure_groups() {
        let mut rng = Rng::new(42);
        let (embs, truth) = three_clouds(&mut rng);
        let labels = dbscan(&embs, &vec![true; embs.len()], params());
        assert_pure(&labels, &truth);
        let groups = labels.iter().flatten().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(groups, 3);
    }

    /// Une photo sombre, à mi-chemin entre deux personnes, ne doit pas les réunir.
    #[test]
    fn low_quality_face_does_not_bridge_two_people() {
        let mut rng = Rng::new(9);
        let a = rng.unit(128);
        let b = rng.unit(128);
        let mut embs: Vec<Vec<f32>> = (0..5).map(|_| around(&mut rng, &a, 0.5)).collect();
        embs.extend((0..5).map(|_| around(&mut rng, &b, 0.5)));
        let bridge = mix(&a, &b, 0.5);
        embs.push(bridge);
        let truth: Vec<usize> = (0..5).map(|_| 0).chain((0..5).map(|_| 1)).chain([2]).collect();
        let p = ClusterParams { threshold: 0.30, min_samples: 2, low_quality_penalty: 0.30, centroid_min: 0.0 };

        let mut good = vec![true; 11];
        let naive = dbscan(&embs, &good, p);
        assert_eq!(naive[0], naive[9], "sans la qualité, le pont réunit les deux personnes (sanity)");

        good[10] = false;
        let labels = dbscan(&embs, &good, p);
        assert_pure(&labels[..10], &truth[..10]);
        assert_ne!(labels[0], labels[9]);
    }

    #[test]
    fn far_member_is_detached() {
        let mut rng = Rng::new(12);
        let a = rng.unit(128);
        let b = rng.unit(128);
        let mut embs: Vec<Vec<f32>> = (0..6).map(|_| around(&mut rng, &a, 0.3)).collect();
        // Un visage relié au groupe mais loin de son centre
        embs.push(mix(&a, &b, 0.6));
        let p = ClusterParams { threshold: 0.30, min_samples: 2, low_quality_penalty: 0.0, centroid_min: 0.0 };
        let loose = dbscan(&embs, &[true; 7], p);
        assert!(loose[6].is_some(), "sanity : rattaché sans le contrôle du centre");
        let strict = dbscan(&embs, &[true; 7], ClusterParams { centroid_min: 0.75, ..p });
        assert_eq!(strict[6], None);
        assert!(strict[..6].iter().all(|l| *l == Some(0)));
    }

    #[test]
    fn lone_face_is_isolated() {
        let mut rng = Rng::new(7);
        let (mut embs, _) = three_clouds(&mut rng);
        embs.push(rng.unit(128));
        let labels = dbscan(&embs, &vec![true; embs.len()], params());
        assert_eq!(*labels.last().unwrap(), None);
    }

    // AC-F2-3
    #[test]
    fn deterministic() {
        let mut rng = Rng::new(3);
        let (embs, _) = three_clouds(&mut rng);
        let good = vec![true; embs.len()];
        assert_eq!(dbscan(&embs, &good, params()), dbscan(&embs, &good, params()));
    }

    #[test]
    fn split_group_separates_a_mixed_group() {
        let (_d, mut s) = crate::store::tests::mem_store();
        let mut rng = Rng::new(21);
        let a = rng.unit(128);
        let b = rng.unit(128);
        // Deux personnes assez proches pour être réunies au seuil normal
        let close_b = mix(&a, &b, 0.35);
        let mut embs: Vec<Vec<f32>> = (0..4).map(|_| around(&mut rng, &a, 0.25)).collect();
        embs.extend((0..4).map(|_| around(&mut rng, &close_b, 0.25)));
        let faces: Vec<_> = embs.into_iter().map(crate::store::tests::fake_face).collect();
        s.save_photo(std::path::Path::new("x.jpg"), 1, 1, "x", (10, 10), &faces).unwrap();
        let p = ClusterParams { threshold: 0.45, min_samples: 2, low_quality_penalty: 0.1, centroid_min: 0.40 };
        assert_eq!(recluster(&mut s, p).unwrap(), 1, "sanity : un seul groupe mélangé");
        let g = s.groups().unwrap()[0].cluster_id;
        let n = split_group(&mut s, g, p).unwrap();
        assert!(n >= 2, "{n} groupe(s)");
        for grp in s.groups().unwrap() {
            let ids = &grp.face_ids;
            assert!(ids.iter().all(|&f| f <= 4) || ids.iter().all(|&f| f > 4), "groupe encore mélangé : {ids:?}");
        }
    }

    // AC-F2-4 (mesure significative en release : cargo test --release -- --ignored)
    #[test]
    #[ignore = "mesure de performance, à lancer en release"]
    fn perf_5000() {
        let mut rng = Rng::new(11);
        let centers: Vec<_> = (0..250).map(|_| rng.unit(128)).collect();
        let embs: Vec<_> = (0..5000).map(|i| around(&mut rng, &centers[i % 250], 0.6)).collect();
        let t = std::time::Instant::now();
        let labels = dbscan(&embs, &vec![true; 5000], params());
        let elapsed = t.elapsed();
        assert_eq!(labels.len(), 5000);
        assert!(elapsed.as_secs_f32() < 10.0, "{elapsed:?}");
        eprintln!("regroupement de 5000 empreintes : {elapsed:?}");
    }
}
