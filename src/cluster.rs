//! Regroupement des visages non attribués (F2) : DBSCAN sur la similarité cosinus.
//!
//! Un visage est « cœur » s'il a au moins `min_samples` voisins (lui compris)
//! de similarité >= `threshold`. Les groupes s'étendent de cœur en cœur ; un
//! visage sans cœur voisin reste isolé. Parcours dans l'ordre des indices :
//! le résultat est déterministe (AC-F2-3).

use crate::embed::cosine;
use crate::store::Store;
use crate::{Result, thresholds};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct ClusterParams {
    pub threshold: f32,
    pub min_samples: usize,
}

impl Default for ClusterParams {
    fn default() -> Self {
        Self { threshold: thresholds::CLUSTER, min_samples: thresholds::CLUSTER_MIN_SAMPLES }
    }
}

/// Renvoie une étiquette de groupe par empreinte (`None` = isolé). Les
/// étiquettes sont numérotées dans l'ordre d'apparition.
pub fn dbscan(embs: &[Vec<f32>], p: ClusterParams) -> Vec<Option<usize>> {
    let n = embs.len();
    let neighbors: Vec<Vec<usize>> = (0..n)
        .into_par_iter()
        .map(|i| (0..n).filter(|&j| j == i || cosine(&embs[i], &embs[j]) >= p.threshold).collect())
        .collect();
    let is_core: Vec<bool> = neighbors.iter().map(|v| v.len() >= p.min_samples.max(1)).collect();
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
    labels
}

/// Regroupe tous les visages non attribués de la base et renvoie le nombre de groupes.
pub fn recluster(store: &mut Store, p: ClusterParams) -> Result<usize> {
    let faces = store.unassigned_embeddings()?;
    let embs: Vec<Vec<f32>> = faces.iter().map(|(_, e)| e.clone()).collect();
    let labels = dbscan(&embs, p);
    let groups = labels.iter().flatten().max().map_or(0, |m| m + 1);
    let pairs: Vec<(i64, Option<i64>)> = faces.iter().zip(&labels).map(|((id, _), l)| (*id, l.map(|v| v as i64))).collect();
    store.set_clusters(&pairs)?;
    Ok(groups)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::embed::l2_normalize;

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
        let mut v: Vec<f32> = center.iter().map(|c| c + noise * rng.next_f32() / (center.len() as f32).sqrt() * 1.7).collect();
        l2_normalize(&mut v);
        v
    }

    fn three_clouds(rng: &mut Rng) -> (Vec<Vec<f32>>, Vec<usize>) {
        let centers: Vec<_> = (0..3).map(|_| rng.unit(128)).collect();
        let mut embs = Vec::new();
        let mut truth = Vec::new();
        for k in 0..6 {
            for (c, center) in centers.iter().enumerate() {
                let _ = k;
                embs.push(around(rng, center, 0.6));
                truth.push(c);
            }
        }
        (embs, truth)
    }

    #[test]
    fn separated_clouds_give_pure_groups() {
        let mut rng = Rng::new(42);
        let (embs, truth) = three_clouds(&mut rng);
        let labels = dbscan(&embs, ClusterParams::default());
        for (i, li) in labels.iter().enumerate() {
            for (j, lj) in labels.iter().enumerate() {
                if li.is_some() && li == lj {
                    assert_eq!(truth[i], truth[j], "groupe impur");
                }
            }
        }
        let groups = labels.iter().flatten().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(groups, 3);
    }

    #[test]
    fn lone_face_is_isolated() {
        let mut rng = Rng::new(7);
        let (mut embs, _) = three_clouds(&mut rng);
        embs.push(rng.unit(128));
        let labels = dbscan(&embs, ClusterParams::default());
        assert_eq!(*labels.last().unwrap(), None);
    }

    // AC-F2-3
    #[test]
    fn deterministic() {
        let mut rng = Rng::new(3);
        let (embs, _) = three_clouds(&mut rng);
        assert_eq!(dbscan(&embs, ClusterParams::default()), dbscan(&embs, ClusterParams::default()));
    }

    // AC-F2-4 (mesure significative en release : cargo test --release -- --ignored)
    #[test]
    #[ignore = "mesure de performance, à lancer en release"]
    fn perf_5000() {
        let mut rng = Rng::new(11);
        let centers: Vec<_> = (0..250).map(|_| rng.unit(128)).collect();
        let embs: Vec<_> = (0..5000).map(|i| around(&mut rng, &centers[i % 250], 0.6)).collect();
        let t = std::time::Instant::now();
        let labels = dbscan(&embs, ClusterParams::default());
        let elapsed = t.elapsed();
        assert_eq!(labels.len(), 5000);
        assert!(elapsed.as_secs_f32() < 10.0, "{elapsed:?}");
        eprintln!("regroupement de 5000 empreintes : {elapsed:?}");
    }
}
