# FaceSort - Plan d'implémentation

> Version 1 · 2026-11-11 · Pour confier une tâche à Claude Code : « Implémente T3 de product/IMPLEMENTATION_PLAN.md » ou `/idea-to-product build T3`

## 1. Décisions à prendre avant de coder

| ID | Décision | Options | Recommandation | Bloque |
|---|---|---|---|---|
| D1 | Modèle de détection et de reconnaissance | YuNet + SFace ; YuNet + AuraFace-v1 (Apache 2.0) ; RetinaFace/ArcFace InsightFace (non commercial) | Tranché le 2026-11-11 : YuNet + AuraFace par défaut pour une nouvelle base (moins de confusions, mesure ci-dessous), SFace en repli et pour les bases existantes, conversion sans perte des noms (GUI et `facesort-cli convert`) | T2, T11 |
| D2 | Photos de test | Images libres de droits versionnées ; photos personnelles non versionnées (`fixtures-private/`) ; les deux | Les deux : quelques images libres pour la CI si un jeu adapté existe, photos personnelles pour l'évaluation T11 | T2, T4, T11 |
| D3 | Variante YuNet | `2023mar` (entrée fixe) ; `2026may` (entrée dynamique) | Essai court en T2 : garder celle qui s'exécute avec `ort` et donne les cadres attendus sur `fixtures/basic` | T2 |

## 2. Étapes

1. **Fondations** - projet, outillage de test, schéma de base, validation des noms (T1).
2. **Parcours principal, moteur** - détection, empreintes, scan en CLI (T2 à T4), puis regroupement, reconnaissance, opérations sur les personnes, tri (T5 à T8).
3. **Parcours principal, GUI** - écrans E1 à E3 (T9, T10).
4. **Validation du MVP** - évaluation de la qualité et calibration des seuils, paquet portable, scénarios manuels (T11, T12).

## 3. Tranche 1 - premier résultat testable

- **Ce que l'utilisateur pourra faire à la fin** : lancer `facesort-cli scan D:\Photos\2024` et lire « 812 photos analysées, 1 455 visages, 3 erreurs, 1,1 s/photo », puis `facesort-cli status`. Valide H3 et donne une première lecture de H2.
- **Tâches incluses** : T1, T2, T3, T4.
- **Comment le tester** : `cargo test` ; puis scénario M1 sur un vrai dossier.

## 4. Tâches

### T1 - Fondations du projet

- **Objectif** : squelette compilable et testé.
- **Dépend de** : aucune.
- **Fichiers ou composants** : `Cargo.toml`, `src/lib.rs`, `src/store.rs`, `src/names.rs`, `src/bin/cli.rs`, `src/bin/gui/main.rs`, `README.md`, `.gitignore` (`/target`, `models/*.onnx`, `fixtures-private/`).
- **À faire** :
  1. `cargo init`, `git init`, edition 2024, lib + bins `FaceSort` et `facesort-cli` (noms distincts même sans la casse : sous Windows `FaceSort.exe` et `facesort.exe` seraient le même fichier) (modèle FontLicenseClassifier).
  2. Ajouter les dépendances avec `cargo add` (versions réelles, pas devinées) ; `ort = "=2.0.0-rc.15"`.
  3. `store` : ouverture de la base (`--db` ou `%LOCALAPPDATA%\FaceSort\facesort.db`), WAL, schéma v1, `user_version`.
  4. `names::validate_person_name` (règles F3).
  5. CLI : sous-commandes vides `scan`, `sort`, `status` avec `--help`.
  6. Vérifier que `ort` récupère et copie la DLL ONNX Runtime sous Windows (note dans README).
- **Tests** : unitaires `names` (valides, `Tom/Léa`, `CON`, `com1`, espaces, 65 caractères) ; `store` crée et rouvre une base temporaire.
- **Terminé quand** : `cargo test` et `cargo clippy -- -D warnings` passent ; `facesort-cli --help` liste les trois commandes.
- **Couvre** : F3, F6, AC-F3-3
- **Statut** : fait (2026-11-11 : `cargo test` 47 tests OK, `cargo clippy --all-targets -- -D warnings` OK, `facesort-cli --help` liste scan/sort/status ; CLI renommée `facesort-cli`, voir T9)

### T2 - Modèles et détection

- **Objectif** : détecter les visages d'une image.
- **Dépend de** : T1, D1, D2, D3.
- **Fichiers ou composants** : `scripts/fetch-models.ps1`, `models/SHA256SUMS`, `src/detect.rs`, `src/scan.rs` (décodage).
- **À faire** :
  1. Script de téléchargement depuis opencv_zoo (URL exactes à vérifier dans le dépôt), calcul et écriture des SHA-256.
  2. Contrôle des SHA-256 au chargement.
  3. Décodage `image` avec `Limits`, orientation EXIF, réduction à 1 600 px.
  4. Trait `FaceDetector` ; implémentation YuNet avec `ort` : pré-traitement, lecture des sorties, post-traitement (décodage des cadres si nécessaire, NMS) en suivant l'implémentation de référence opencv_zoo.
- **Tests** : nombre de visages sur `fixtures/basic` contre `expected.json` ; fixture EXIF orientation 6 vs redressée ; modèle au mauvais hash refusé.
- **Terminé quand** : les tests passent ; un exemple `cargo run --example detect -- photo.jpg` écrit une image annotée vérifiée à l'oeil.
- **Couvre** : F1, AC-F1-5
- **Statut** : fait : `tests/reference.rs` (conformité OpenCV) et `tests/scan.rs::exif_orientation_detects_same_faces` (AC-F1-5) OK

### T3 - Alignement et empreintes

- **Objectif** : produire une empreinte comparable par visage.
- **Dépend de** : T2.
- **Fichiers ou composants** : `src/align.rs`, `src/embed.rs`.
- **À faire** :
  1. Lire les métadonnées d'entrée/sortie de SFace avec `ort` (taille, dimension) et les consigner dans le README.
  2. Similarité 5 points vers le gabarit 112x112, warp bilinéaire.
  3. Trait `FaceEmbedder`, implémentation SFace, normalisation L2.
- **Tests** : alignement d'un jeu de points connu (erreur < 0,5 px) ; deux photos d'une même personne plus similaires entre elles qu'avec une autre personne sur `fixtures/people3`.
- **Terminé quand** : tests verts ; la similarité moyenne intra-personne dépasse la moyenne inter-personnes sur `fixtures/people3`.
- **Couvre** : F1, F4
- **Statut** : fait : `tests/reference.rs`, similarité > 0,99 avec les empreintes OpenCV ; SFace = entrée 1x3x112x112, sortie 128 (lu dans le modèle)

### T4 - Scan complet en CLI

- **Objectif** : parcours P1, étape « analyse », en ligne de commande.
- **Dépend de** : T3.
- **Fichiers ou composants** : `src/scan.rs`, `src/store.rs`, `src/bin/cli.rs`.
- **À faire** :
  1. Parcours `walkdir`, filtre d'extensions, comptage des ignorés.
  2. Détection des inchangés (chemin, taille, mtime).
  3. Pipeline parallèle `rayon` (une session `ort` par thread de travail), écriture SQLite par lots en transaction, vignettes 96 px.
  4. Erreurs par fichier collectées sans arrêter le scan ; annulation par drapeau partagé.
  5. `facesort-cli scan`, `facesort-cli status [--json]`, codes de sortie 0/1/2/3, temps moyen par photo.
- **Tests** : intégration `tests/scan.rs` (fixtures basic, rescan, JPEG tronqué, extensions ignorées) ; `tests/cli.rs` (codes de sortie, JSON valide).
- **Terminé quand** : tests verts ; scénario M1 exécuté sur un vrai dossier et le temps par photo noté (H3).
- **Couvre** : F1, F6, AC-F1-1, AC-F1-2, AC-F1-3, AC-F1-4, AC-F6-1, AC-F6-2, AC-F6-3
- **Statut** : en grande partie fait : `tests/scan.rs::scan_basic_rescan_truncated_and_ignored` et `tests/cli.rs` (4 tests) OK ; reste M1 sur un vrai dossier (H3)

### T5 - Regroupement

- **Objectif** : groupes de visages non attribués.
- **Dépend de** : T1 (empreintes synthétiques), T4 pour l'intégration.
- **Fichiers ou composants** : `src/cluster.rs`.
- **À faire** : DBSCAN cosinus, `min_samples = 2`, ordre stable par identifiant de visage, matrice de similarité parallélisée, respect des exclusions, écriture de `cluster_id`.
- **Tests** : synthétiques (3 nuages bien séparés, bruit isolé, déterminisme, 5 000 empreintes < 10 s en release) ; `fixtures/people3`.
- **Terminé quand** : tests verts.
- **Couvre** : F2, AC-F2-1, AC-F2-2, AC-F2-3, AC-F2-4
- **Statut** : en grande partie fait : unitaires + `tests/scan.rs::full_journey_people3` (groupes purs, 2 groupes max par personne) OK ; reste `perf_5000` en release

### T6 - Reconnaissance

- **Objectif** : attribuer les visages des personnes connues.
- **Dépend de** : T5, T7.
- **Fichiers ou composants** : `src/recognize.rs`, appel en fin de scan avant le regroupement.
- **À faire** : similarité max contre visages confirmés, seuil, marge 0,05, marquage `auto`, exclusions respectées.
- **Tests** : synthétiques (attribution, inconnu, ambiguïté) ; intégration `people3` puis `people3_new`.
- **Terminé quand** : tests verts.
- **Couvre** : F4, AC-F4-1, AC-F4-2, AC-F4-3
- **Statut** : fait : unitaires + `tests/scan.rs::full_journey_people3` (3 nouvelles photos reconnues « auto », inconnu non attribué) OK

### T7 - Opérations sur les personnes

- **Objectif** : nommer, fusionner, retirer, ignorer, dans la lib.
- **Dépend de** : T1.
- **Fichiers ou composants** : `src/store.rs` (ou `src/persons.rs`).
- **À faire** : `name_cluster`, `merge_persons`, `remove_face(face, person)` avec exclusion, `ignore_cluster` ; détection de nom existant insensible à la casse.
- **Tests** : unitaires sur base temporaire, dont exclusion persistante après regroupement.
- **Terminé quand** : tests verts.
- **Couvre** : F3, AC-F3-2, AC-F3-4
- **Statut** : fait : tests `store` (AC-F3-2, AC-F3-3, AC-F3-4) OK

### T8 - Tri par copie

- **Objectif** : produire `Destination/Prénom/`.
- **Dépend de** : T7.
- **Fichiers ou composants** : `src/sort.rs`, `src/bin/cli.rs`.
- **À faire** : plan de tri (copies, collisions, taille, espace libre), refus d'une destination dans une source, exécution atomique avec conservation de la mtime, journal CSV, `facesort-cli sort [--dry-run] [--no-unknown]`.
- **Tests** : `tests/sort.rs` dans `TempDir` : multi-personnes avec SHA-256, `_Inconnus/`, collision différente et identique, second tri à 0 copie, destination en lecture seule, journal, dry-run sans écriture.
- **Terminé quand** : tests verts.
- **Couvre** : F5, F6, AC-F5-1, AC-F5-2, AC-F5-3, AC-F5-4, AC-F5-5, AC-F5-6, AC-F6-2
- **Statut** : fait côté code : `sort::tests::execute_copies_identically_and_is_idempotent`, `write_error_stops_cleanly`, tri réel de 22 copies dans `full_journey_people3` OK ; reste M4 (disque plein, manuel)

### T9 - GUI : écran Analyse (E1)

- **Objectif** : scan depuis la fenêtre.
- **Dépend de** : T4.
- **Fichiers ou composants** : `src/bin/gui/`.
- **À faire** : sélection de dossier `rfd`, scan dans un thread, progression via canal, annulation, résumé et liste d'erreurs, état vide.
- **Tests** : logique de progression testée dans la lib ; vérification manuelle M2.
- **Terminé quand** : M2 étape 1 réussie.
- **Couvre** : F1
- **Statut** : fait côté code (2026-11-11), vérification manuelle restante : compilation et clippy OK, progression testée (`scan::tests::progress_fraction_and_eta`), fenêtre lancée 5 s sans erreur ; à vérifier à la main (M2 étape 1) : choix du dossier, barre de progression, Annuler, résumé, liste d'erreurs

### T10 - GUI : écrans Personnes et Tri (E2, E3)

- **Objectif** : nommage et tri à la souris et au clavier.
- **Dépend de** : T6, T7, T8, T9.
- **Fichiers ou composants** : `src/bin/gui/screens.rs`.
- **À faire** : grille de groupes et vignettes (textures chargées à la demande), champ prénom avec messages de validation, fusion, retrait, ignorer ; écran Tri avec aperçu et progression ; navigation clavier et focus visible.
- **Tests** : scénarios manuels M2 et M3 ; AC-F6-4 par test d'intégration (lib nomme, CLI lit).
- **Terminé quand** : M2 et M3 réussis.
- **Couvre** : F3, F5, F6, AC-F3-1, AC-F3-5, AC-F6-4
- **Statut** : fait côté code (2026-11-11), vérification manuelle restante : écrans Personnes (nommer, « existe déjà -> ajouter », ajouter à…, retirer, ignorer, renommer, fusionner, confirmer les auto, relancer la reconnaissance, visages isolés) et Tri (destination, aperçu par dossier, espace libre, copie en arrière-plan, annulation, journal, ouvrir le dossier) ; `tests/cli.rs::status_sees_named_person` (AC-F6-4) OK ; fenêtre lancée sur une base de 30 visages sans erreur ; à faire à la main : M2 (AC-F3-1, AC-F3-5) et M3

### T11 - Évaluation et calibration des seuils

- **Objectif** : valider H2 et H4, fixer les seuils de regroupement et de reconnaissance.
- **Dépend de** : T6, D2.
- **Fichiers ou composants** : `examples/eval.rs` ; source des étiquettes : la base de l'utilisateur (visages nommés et visages retirés, lus sur une copie) ou, à défaut, un dossier `fixtures-private/eval/` (un sous-dossier par personne).
- **À faire** : balayage des seuils, mesure de la précision et du rappel du tri, nombre de corrections nécessaires ; seuils retenus inscrits dans le code avec leur mesure.
- **Tests** : l'exemple lui-même ; résultats notés dans README.
- **Terminé quand** : précision au moins 95 % et rappel au moins 85 % atteints, ou décision consignée (F10, F8, ou seuils plus prudents).
- **Couvre** : F2, F4
- **Statut** : en cours (2026-11-11). Retour utilisateur : des personnes se mélangent, surtout des femmes et des photos mal éclairées. Fait : note de qualité par visage (`src/quality.rs`) ; regroupement où un visage médiocre ne relie jamais deux groupes et doit être plus ressemblant, membres trop loin du centre détachés ; « Séparer ce groupe » adaptatif ; reconnaissance plus exigeante pour les visages médiocres ; seuils par modèle, resserrés (SFace : regroupement 0,50, reconnaissance 0,45) ; base existante migrée sans perte (notes calculées après coup, attributions auto réévaluées une fois) ; AuraFace intégré ; `examples/eval.rs` réécrit (sous-dossier = personne, deux modèles, confusions, groupes mélangés). Banc public (164 visages de 6 femmes dont 48 médiocres) : 0 groupe mélangé et 0 reconnaissance fausse aux nouveaux seuils pour les deux modèles. Reste : `eval` sur les photos de l'utilisateur, puis choix du modèle par défaut. 2026-11-11 : l'utilisateur ne peut pas préparer de dossier de test ; `eval` lit donc par défaut sa base (copie temporaire) : visages nommés = étiquettes, visages retirés = mélanges réels, empreintes recalculées pour les deux modèles. Vérifié sur une base simulée (164 visages, 6 personnes, 3 retraits).

### T12 - Paquet portable et validation

- **Objectif** : dossier utilisable hors de l'environnement de dev.
- **Dépend de** : T10, T11.
- **Fichiers ou composants** : `scripts/package.ps1`, `README.md`.
- **À faire** : assembler exe, DLL, modèles ; lancer depuis un autre dossier ; README (installation, chemin de la base, suppression des données).
- **Tests** : scénario M4 et relecture de `--help`.
- **Terminé quand** : M4 réussi sur la machine de l'utilisateur.
- **Couvre** : F5, F6, AC-F5-5, AC-F6-1
- **Statut** : fait (2026-11-11) : `scripts/package.ps1` construit `dist/FaceSort-<version>-portable/` (exécutables, `DirectML.dll`, runtime Visual C++ en local, modèles, licences), le zip et l'installeur Inno Setup (`installer/FaceSort.iss`) ; contrôle de démarrage de la CLI depuis le dossier portable ; icône `FaceSort.ico` intégrée aux exécutables et à la fenêtre ; README et `THIRD_PARTY_NOTICES.md` ; publié sur https://github.com/Les-Doubles-A/FaceSort. Reste à vérifier à la main : M4 (installation sur un PC sans Visual Studio, disque plein)

## 5. Peut attendre

| Élément | Pourquoi attendre | Déclencheur |
|---|---|---|
| DirectML (F8) | CPU peut suffire | H3 échoue |
| HEIC/RAW (F9) | Proportion inconnue | AC-F1-4 montre plus de 10 % ignorés |
| RetinaFace (F10) | Licence, qualité YuNet non mesurée | T11 insuffisant |
| Installeur, signature | Usage personnel | Diffusion |
