# FaceSort - Product Brief

> Version 1 · 2026-11-11 · Statut : brouillon à valider
> Légende : **[Fait]** vérifié (source ou « exprimé par l'utilisateur ») · **[H#]** hypothèse · **[Reco]** recommandation · **[À vérifier]**

## 1. Concept

FaceSort est une application Rust pour Windows qui analyse un dossier de photos, détecte les visages, les regroupe par ressemblance, laisse l'utilisateur donner un prénom à chaque groupe, puis copie chaque photo dans un dossier par personne. Tout tourne en local, sans compte ni service en ligne.

## 2. Problème

Un particulier accumule des milliers de photos (téléphone, appareil, partages familiaux) dans des dossiers rangés par date ou pas rangés du tout. Retrouver « toutes les photos de Léa » demande de parcourir les dossiers à la main. Les services qui le font automatiquement envoient les photos dans un cloud ou demandent d'installer un serveur. Fréquence et coût réels : non mesurés **[H1]**.

## 3. Utilisateurs

- **Utilisateur principal** : l'auteur du projet, sur son PC Windows, sans GPU requis **[Fait]** (exprimé par l'utilisateur).
- **Utilisateurs secondaires** : aucun au MVP.
- **Hors cible** : diffusion publique, usage professionnel ou commercial, surveillance ou identification de personnes inconnues.

## 4. Alternatives actuelles

| Alternative | Ce qu'elle fait bien | Limite pour notre cible | Statut de l'information |
|---|---|---|---|
| Ranger à la main | Gratuit, contrôle total | Des heures pour quelques milliers de photos | Évident |
| digiKam (bureau, libre) | Détection et reconnaissance locales, tags « People » | Flux « nommer quelques visages puis faire correspondre » ; regroupement des inconnus non documenté dans le manuel actuel ; pas de tri en dossiers ; retours d'utilisateurs sur des correspondances peu fiables | Vérifié le 2026-11-11 (docs.digikam.org, bugs.kde.org, digikam-users) |
| Immich (serveur auto-hébergé) | Regroupement des visages (variante de DBSCAN), nommage, fusion | Exige un serveur (Docker) ; galerie, pas de rangement des fichiers en dossiers | Vérifié le 2026-11-11 (docs.immich.app) |
| Services cloud de photos | Regroupement automatique efficace | Photos et données biométriques chez un tiers ; pas de rangement local | Catégorie, non vérifiée produit par produit **[À vérifier]** |

## 5. Proposition de valeur

Ranger un dossier de photos par personne en une soirée, sans envoyer une seule photo hors de la machine et sans installer de serveur.

- Un exécutable Windows, pas de Docker ni de compte.
- Le résultat est un rangement de **fichiers** (dossiers `Prénom/`), lisible par n'importe quel explorateur, pas une base propre à une galerie.
- Les originaux ne sont jamais modifiés (copie) **[Fait]** (choix de l'utilisateur).
- La base des visages nommés sert aux dossiers suivants : un nouveau dossier se trie sans tout renommer.

## 6. Hypothèses

| ID | Hypothèse | Type | Si elle est fausse | Comment la tester | Coût du test |
|---|---|---|---|---|---|
| H1 | Le rangement par personne a assez de valeur pour l'utilisateur pour justifier l'outil | Problème | Projet d'apprentissage seulement | Usage réel sur un vrai dossier pendant deux semaines | Faible |
| H2 | YuNet (détection) + SFace (reconnaissance), sous licences MIT et Apache 2.0, donnent une qualité suffisante sur des photos de famille | Technique | Passer à RetinaFace/ArcFace (licence non commerciale) ou accepter plus de corrections manuelles | Jeu de 200 photos annotées à la main, mesure de précision du tri (T11) | Moyen |
| H3 | Le traitement CPU tient en moins de 2 s par photo en moyenne sur le PC de l'utilisateur | Technique | Ajouter DirectML (V2) ou réduire la résolution d'analyse | Mesure sur 500 photos réelles (T4) | Faible |
| H4 | Un regroupement simple (seuil de similarité cosinus + liaison) suffit, avec fusion manuelle des groupes scindés | Technique | Implémenter un regroupement plus fin (type DBSCAN/HDBSCAN) | Compter les fusions et retraits nécessaires sur le jeu de 200 photos | Faible |
| H5 | JPEG et PNG couvrent l'essentiel des photos de l'utilisateur | Usage | Ajouter HEIC (photos iPhone) plus tôt | Compter les extensions dans le dossier réel (le scan les rapporte, AC-F1-4) | Nul |
| H6 | La copie (doublement de l'espace disque) est acceptable | Usage | Liens physiques ou déplacement en V2 | L'aperçu affiche l'espace nécessaire avant copie (AC-F5-1) | Nul |
| H7 | Usage strictement personnel et domestique des données biométriques | Légal | Obligations RGPD (données biométriques, art. 9) à traiter avant toute diffusion | Rester en usage personnel ; avis juridique avant de distribuer **[À vérifier]** | - |

## 7. Tester l'intérêt avant d'investir

La tranche 1 (scan CLI qui compte les visages) valide déjà H3 et donne une première idée de H2. Avant la GUI, faire le test H2 sur 200 photos : si moins de 80 % des photos sont rangées correctement sans correction, réévaluer le modèle (D1) avant de poursuivre.

## 8. Objectifs et indicateurs de réussite

| Objectif | Indicateur | Seuil de réussite du MVP | Mesure |
|---|---|---|---|
| Tri fiable | Part des copies correctes (bonne personne) sur le jeu de 200 photos | au moins 95 % de copies correctes, au moins 85 % des apparitions de personnes nommées trouvées | Jeu annoté, T11 |
| Effort de nommage raisonnable | Temps de nommage pour 1 000 photos | moins de 15 minutes | Chronomètre, scénario M3 |
| Performance | Temps de scan moyen par photo (CPU) | moins de 2 s | Sortie de `facesort-cli scan`, H3 |
| Usage réel | Dossiers triés sur deux semaines | au moins 2 dossiers réels | Auto-observation |

## 9. Périmètre du MVP

- **MVP** : F1 scan et analyse, F2 regroupement, F3 nommage et correction en GUI, F4 reconnaissance avec la base, F5 tri par copie, F6 commandes CLI.
- **Version 2** : F7 déplacement ou liens physiques, F8 accélération GPU (DirectML), F9 HEIC et RAW, F10 détecteur RetinaFace en option, F11 export et import de la base, F12 écriture des noms en métadonnées XMP.
- **Hors périmètre** : cloud, comptes, mobile, vidéo, estimation d'âge ou d'émotion.

## 10. Contraintes

- **[Fait]** Langage Rust (exprimé par l'utilisateur).
- **[Fait]** Copie des photos, originaux intacts ; regrouper puis nommer ; GUI egui + CLI ; usage personnel, Windows, CPU (réponses de l'utilisateur du 2026-11-11).
- **[Fait]** Conventions des projets Rust voisins : edition 2024, eframe/egui 0.37, rfd, rusqlite bundled, lib + binaire GUI + binaire CLI (constaté dans `FontLicenseClassifier/Cargo.toml`, `NetPulse/Cargo.toml`).
- **[Reco]** Aucun service externe à l'exécution.

## 11. Questions ouvertes

- D1 (choix du modèle de détection) : YuNet recommandé ; l'utilisateur a cité RetinaFace. Voir IMPLEMENTATION_PLAN.md.
- Faut-il un dossier `_Inconnus/` pour les photos avec visages non nommés ? Recommandation : oui, désactivable (AC-F5-3).

## Sources

Consultées le 2026-11-11 :
- InsightFace model zoo (licence des modèles RetinaFace, SCRFD, ArcFace) : https://github.com/deepinsight/insightface/tree/master/model_zoo
- YuNet, OpenCV Zoo (licence MIT, fichiers ONNX) : https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet
- SFace, OpenCV Zoo (licence Apache 2.0, fichiers ONNX) : https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface
- Crate `ort` : https://crates.io/api/v1/crates/ort et https://docs.rs/crate/ort/2.0.0-rc.15/features
- digiKam People View : https://docs.digikam.org/en/left_sidebar/people_view.html ; demande de regroupement des inconnus : https://bugs.kde.org/show_bug.cgi?id=392024 ; retour utilisateur : https://bugs.kde.org/show_bug.cgi?id=444160
- Immich, reconnaissance faciale : https://docs.immich.app/features/facial-recognition
