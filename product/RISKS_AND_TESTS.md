# FaceSort - Risques et tests

> Version 1 · 2026-11-11

## 1. Risques

| ID | Risque | Type | Probabilité | Impact | Prévention | Repli |
|---|---|---|---|---|---|---|
| R1 | Qualité YuNet/SFace insuffisante (profils, enfants, faible lumière, ressemblances familiales) | Technique | Moyenne | Fort | Évaluation T11 avant la GUI complète, seuils prudents, correction manuelle (F3) | RetinaFace/ArcFace en option pour usage personnel (F10), après relecture de leur licence |
| R2 | API `ort` en version rc qui change | Technique | Moyenne | Moyen | Version exacte épinglée (ADR-2), modèles derrière des traits | Rester sur rc.15 ; adapter `detect.rs`/`embed.rs` seulement |
| R3 | Post-traitement YuNet mal reproduit (cadres décalés) | Technique | Moyenne | Fort | Suivre l'implémentation de référence opencv_zoo, image annotée en T2 | Variante de modèle avec post-traitement intégré (D3) |
| R4 | Scan trop lent sur CPU | Technique | Moyenne | Moyen | Réduction 1 600 px, parallélisme, incrémental | DirectML (F8), résolution plus faible |
| R5 | Erreur de regroupement en chaîne (deux personnes fusionnées) | Fonctionnel | Moyenne | Moyen | DBSCAN `min_samples=2`, seuil calibré, retrait de visage | Fusion et retrait manuels |
| R6 | Perte ou corruption d'originaux | Fonctionnel | Faible | Fort | Lecture seule des sources, copie atomique, destination interdite dans une source | Aucun : c'est pourquoi la copie a été choisie |
| R7 | Espace disque doublé | Fonctionnel | Forte | Faible | Aperçu avec taille et espace libre (AC-F5-1) | Liens physiques (F7) |
| R8 | Données biométriques de tiers (famille, amis) | Légal | Faible en usage personnel | Fort si diffusion | Local uniquement, jamais exporté, suppression simple, pas de diffusion (H7) | Avis juridique avant toute diffusion **[À vérifier]** |
| R9 | Licences des modèles mal interprétées | Légal | Faible | Moyen | ADR-1 ; fichiers LICENSE des modèles copiés dans `models/` | Changer de modèle |
| R10 | Pas de jeu d'images libres adapté pour la CI | Technique | Moyenne | Faible | D2 | Tests d'intégration ignorés hors machine de l'utilisateur, unitaires synthétiques en CI |

## 2. Hypothèses non vérifiées

| ID | Hypothèse | Comment la vérifier | Quand | Ce qu'on fait si elle est fausse |
|---|---|---|---|---|
| H1 | Le besoin est réel | Usage sur deux semaines | Après T12 | Arrêter ou garder comme projet d'apprentissage |
| H2 | Qualité YuNet + SFace suffisante | T11 sur 200 photos annotées | Après T6 | F10, ou seuils plus prudents et plus de nommage manuel |
| H3 | Moins de 2 s par photo sur CPU | Mesure M1 | Fin de tranche 1 | F8, réduction de résolution |
| H4 | DBSCAN simple suffit | Nombre de corrections en T11 et M3 | Après T10 | Regroupement plus fin |
| H5 | JPEG et PNG suffisent | Comptage AC-F1-4 sur le vrai dossier | Fin de tranche 1 | F9 |
| H6 | Copie acceptable | Aperçu AC-F5-1, ressenti en M3 | Après T10 | F7 |
| H7 | Usage personnel uniquement | Décision de l'utilisateur | Avant toute diffusion | Revue RGPD et licences |

## 3. Stratégie de test

| Niveau | Outil | Ce qui est couvert | Quand l'exécuter |
|---|---|---|---|
| Unitaire | `cargo test` (lib) | Noms, alignement, regroupement, reconnaissance, plan de tri, opérations sur les personnes | À chaque modification |
| Intégration | `cargo test` (`tests/`), `tempfile`, `std::process::Command` | Scan des fixtures, tri réel, CLI | À chaque modification (ignorés si modèles absents) |
| Évaluation | `cargo run --release --example eval` | Précision et rappel du tri, seuils | Après un changement de modèle ou de seuil |
| Manuel | Scénarios M1 à M4 | GUI, clavier, vrai dossier, paquet | Avant de déclarer le MVP terminé |

**Commande unique** : `cargo test` (puis `cargo clippy -- -D warnings`).

## 4. Couverture des critères d'acceptation

| Critère | Vérification | Test ou scénario | Tâche |
|---|---|---|---|
| AC-F1-1 | Automatique | `tests/scan.rs::scan_basic` | T4 |
| AC-F1-2 | Automatique | `tests/scan.rs::rescan_unchanged` | T4 |
| AC-F1-3 | Automatique | `tests/scan.rs::truncated_jpeg` | T4 |
| AC-F1-4 | Automatique | `tests/scan.rs::ignored_extensions` | T4 |
| AC-F1-5 | Automatique | `src/detect.rs` test `exif_orientation_6` | T2 |
| AC-F2-1 | Automatique | `src/cluster.rs` + `tests/scan.rs::people3_clusters_pure` | T5 |
| AC-F2-2 | Automatique | `tests/scan.rs::people3_max_two_groups` | T5 |
| AC-F2-3 | Automatique | `src/cluster.rs` test `deterministic` | T5 |
| AC-F2-4 | Automatique | `src/cluster.rs` test `perf_5000` (release, `#[ignore]` en debug) | T5 |
| AC-F3-1 | Manuelle | M2 | T10 |
| AC-F3-2 | Automatique | `store` test `merge_on_same_name` | T7 |
| AC-F3-3 | Automatique | `names` tests | T1 |
| AC-F3-4 | Automatique | `store` test `exclusion_persists` | T7 |
| AC-F3-5 | Manuelle | M2 | T10 |
| AC-F4-1 | Automatique | `tests/scan.rs::recognize_new_photos` | T6 |
| AC-F4-2 | Automatique | `tests/scan.rs::unknown_not_assigned` | T6 |
| AC-F4-3 | Automatique | `src/recognize.rs` test `ambiguous_margin` | T6 |
| AC-F5-1 | Automatique | `tests/sort.rs::dry_run_writes_nothing` | T8 |
| AC-F5-2 | Automatique | `tests/sort.rs::multi_person_copies_identical` | T8 |
| AC-F5-3 | Automatique | `tests/sort.rs::unknown_folder_option` | T8 |
| AC-F5-4 | Automatique | `tests/sort.rs::collisions_and_idempotence` | T8 |
| AC-F5-5 | Automatique + manuelle | `tests/sort.rs::readonly_destination` ; M4 (disque plein) | T8, T12 |
| AC-F5-6 | Automatique | `tests/sort.rs::log_csv` | T8 |
| AC-F6-1 | Automatique + manuelle | `tests/cli.rs::help` ; relecture en M4 | T4, T12 |
| AC-F6-2 | Automatique | `tests/cli.rs::exit_codes` | T4, T8 |
| AC-F6-3 | Automatique | `tests/cli.rs::status_json` | T4 |
| AC-F6-4 | Automatique | `tests/cli.rs::status_sees_named_person` | T10 |

## 5. Scénarios de test manuels

### M1 - Scan d'un vrai dossier (tranche 1)

- **Préparation** : un dossier réel de 500 photos au moins ; modèles téléchargés.
- **Étapes** : `facesort-cli scan <dossier>` ; noter durée et temps par photo ; `facesort-cli status` ; relancer le scan.
- **Résultat attendu** : résumé cohérent, extensions ignorées listées, second scan à 0 photo analysée, temps moyen noté pour H3.
- **Critères couverts** : AC-F1-1, AC-F1-2, AC-F1-4 (en conditions réelles).

### M2 - Nommage dans la GUI

- **Préparation** : base issue de M1.
- **Étapes** : ouvrir FaceSort ; vérifier E1 ; aller sur E2 uniquement au clavier ; nommer trois groupes, dont un avec un nom invalide puis corrigé ; fermer et rouvrir.
- **Résultat attendu** : focus toujours visible, noms conservés après redémarrage, message clair sur le nom invalide.
- **Critères couverts** : AC-F3-1, AC-F3-5.

### M3 - Parcours P1 complet chronométré

- **Préparation** : dossier de 1 000 photos jamais scanné, base vide.
- **Étapes** : scan, nommage de toutes les personnes récurrentes, fusions et retraits nécessaires, tri vers un disque de destination.
- **Résultat attendu** : nommage en moins de 15 minutes, dossiers par personne corrects à la vérification visuelle de 50 photos tirées au hasard.
- **Critères couverts** : P1 de bout en bout (indicateurs du brief).

### M4 - Paquet portable et erreurs disque

- **Préparation** : sortie de `scripts/package.ps1` copiée dans un autre dossier ; clé USB presque pleine.
- **Étapes** : lancer GUI et CLI depuis ce dossier ; lire `--help` ; trier vers la clé presque pleine.
- **Résultat attendu** : application fonctionnelle hors environnement de dev ; arrêt propre, message clair, aucun fichier partiel sur la clé, journal présent.
- **Critères couverts** : AC-F5-5, AC-F6-1.

## 6. Validation du MVP

Le MVP est validé quand :

- [ ] tous les critères d'acceptation MVP sont vérifiés (tableau de la section 4) ;
- [ ] `cargo test` passe sans échec, avec les modèles présents ;
- [ ] M1 à M4 ont été exécutés sur la machine de l'utilisateur ;
- [ ] T11 donne au moins 95 % de précision et 85 % de rappel, ou une décision documentée.

## 7. Après le lancement

Nombre de dossiers réellement triés, nombre de corrections par session de nommage, extensions ignorées, temps de scan, photos mal rangées trouvées après coup (à noter dans un fichier pendant deux semaines).
