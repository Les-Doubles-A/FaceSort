<p align="center"><img src="assets/icon-256.png" width="128" alt="Icône FaceSort"></p>

# FaceSort

Range tes photos par personne : FaceSort analyse un dossier, détecte les visages, les regroupe par ressemblance, te laisse donner un prénom à chaque groupe, puis **copie** chaque photo dans `Destination\<Prénom>\`. Tout se passe sur ton ordinateur : aucune photo ni aucune donnée n'est envoyée en ligne.

## Installation

Télécharge la dernière version dans [Releases](https://github.com/Les-Doubles-A/FaceSort/releases) :

- **FaceSort-Setup-x.y.z.exe** : installeur (utilisateur courant, sans droits administrateur, raccourcis menu Démarrer et bureau en option).
- **FaceSort-x.y.z-portable.zip** : à décompresser où tu veux, puis lancer `FaceSort.exe`.

Windows 10 ou 11, 64 bits. Les modèles de reconnaissance sont inclus (environ 300 Mo). L'installeur n'est pas signé : si SmartScreen s'affiche, clique sur « Informations complémentaires » puis « Exécuter quand même ».

## Utilisation

1. **Analyse** : choisis un ou plusieurs dossiers de photos (JPEG, PNG), puis « Analyser ». Seules les photos nouvelles ou modifiées sont réanalysées les fois suivantes.
2. **Personnes** : donne un prénom à chaque groupe de visages.
   - « Retirer » sous une vignette : ce visage n'est pas cette personne (il ne lui sera plus attribué).
   - « Séparer ce groupe » : coupe un groupe où deux personnes se sont mélangées.
   - « Ajouter à… », « Fusionner avec… » : réunit deux groupes de la même personne.
   - La mention « médiocre » signale une photo sombre, floue, peu contrastée ou un visage petit : la reconnaissance y est moins fiable.
   - « Relancer la reconnaissance » attribue aux personnes nommées les visages qui leur ressemblent.
3. **Tri** : choisis un dossier de destination, regarde l'aperçu (copies par personne, taille, espace libre), puis « Copier ». Une photo avec plusieurs personnes est copiée dans le dossier de chacune ; les photos sans personne nommée vont dans `_Inconnus\` (désactivable). Un journal CSV est écrit dans la destination.

   - **Dossiers spéciaux** : crée tes propres dossiers selon une règle, par exemple « Parents » = les photos où Léa et Tom sont **ensemble**, « Cousins » = les photos avec **au moins un** des cousins, « Léa seule » = **uniquement** Léa. Ils sont enregistrés et suivent les renommages. La case « Créer un dossier par personne » peut être décochée pour n'avoir que tes dossiers spéciaux.

Les originaux ne sont jamais modifiés. Relancer un tri ne recopie pas les photos déjà présentes. Chaque onglet défile si son contenu dépasse la fenêtre.

## Modèles de reconnaissance

| Modèle | Rôle | Licence | Remarque |
|---|---|---|---|
| [YuNet](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet) (OpenCV Zoo) | Détection des visages | MIT | |
| [AuraFace-v1](https://huggingface.co/fal/AuraFace-v1) (fal.ai) | Reconnaissance (défaut pour une nouvelle base) | Apache 2.0 | Confond moins les personnes, plus lent |
| [SFace](https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface) (OpenCV Zoo) | Reconnaissance (repli) | Apache 2.0 | Plus rapide, confond davantage |

Mesures du 2026-11-11 sur 164 visages de 6 femmes, dont 48 photos volontairement dégradées (sombres, ternes, brûlées, floues) : plus forte similarité entre deux personnes différentes 0,33 avec AuraFace contre 0,40 avec SFace sur les bonnes photos (0,42 contre 0,54 sur les photos médiocres). Aucun groupe mélangé ni aucune reconnaissance fausse aux seuils retenus, pour les deux modèles.

Une base remplie avec SFace se convertit en AuraFace sans perdre les noms : bouton « Passer à AuraFace » de l'onglet Analyse, ou `facesort-cli convert --model auraface`.

## Vie privée

Les empreintes faciales associées à des prénoms sont des données biométriques. FaceSort les garde uniquement dans `%LOCALAPPDATA%\FaceSort\facesort.db` ; supprimer ce fichier efface tout (la désinstallation ne le supprime pas, pour ne pas perdre tes noms par erreur). Utilise FaceSort pour tes propres photos, dans un cadre personnel et familial.

## Ligne de commande

`facesort-cli.exe` (installé à côté de `FaceSort.exe`) partage la même base :

```
facesort-cli scan D:\Photos\2024          analyse un dossier
facesort-cli status                       photos, visages, personnes, modèle
facesort-cli sort E:\Tri --dry-run        aperçu du tri, rien n'est écrit
facesort-cli sort E:\Tri                  copie (dossiers par personne et dossiers spéciaux)
facesort-cli sort E:\Tri --no-person-folders   dossiers spéciaux seulement
facesort-cli convert --model auraface     change de modèle sans perdre les noms
```

Options communes : `--db <fichier>`, `--models <dossier>`, `--json`. Codes de sortie : 0 succès, 1 erreur, 2 arguments invalides, 3 succès partiel.

Variables d'environnement : `FACESORT_DB` (base de la fenêtre), `FACESORT_MODELS` (dossier des modèles), `FACESORT_EMBEDDER` (`sface` ou `auraface`, force le modèle).

## Mesurer la qualité sur tes photos

```
cargo run --release --example eval
```

Lit ta base (sur une copie) : les visages que tu as nommés servent d'étiquettes, ceux que tu as retirés d'une personne sont des exemples réels de mélange. Affiche, pour SFace et AuraFace, les personnes les plus confondues, les reconnaissances fausses et les groupes mélangés selon le seuil. Variante : `eval <dossier>` avec un sous-dossier par personne.

## Construire depuis les sources

Prérequis : Rust (édition 2024), Visual Studio Build Tools (MSVC), Python n'est pas nécessaire.

```
powershell -ExecutionPolicy Bypass -File scripts\fetch-models.ps1 -AuraFace
cargo test
cargo build --release
powershell -ExecutionPolicy Bypass -File scripts\package.ps1     # installeur + version portable (Inno Setup 6)
```

ONNX Runtime est lié statiquement par la crate [`ort`](https://crates.io/crates/ort) (2.0.0-rc.15). Les exécutables ont besoin, à côté d'eux, de `DirectML.dll` et du runtime Visual C++ (`msvcp140*.dll`, `vcruntime140*.dll`) ; `scripts\package.ps1` les copie.

Organisation : `src/` (bibliothèque : analyse, base SQLite, regroupement, reconnaissance, tri), `src/bin/gui` (fenêtre egui), `src/bin/cli.rs`, `tests/` (dont parcours complet avec les vrais modèles), `fixtures/` (photos du domaine public, sources dans `fixtures/SOURCES.md`), `product/` (cahier des charges, plan, risques et tests).

## Composants tiers

Voir [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
