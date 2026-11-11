# FaceSort - Fonctionnalités et critères d'acceptation

> Version 1 · 2026-11-11

## 1. Priorités

| ID | Fonctionnalité | Priorité | Justification |
|---|---|---|---|
| F1 | Scan d'un dossier : détection des visages et calcul de leurs empreintes, stockés en base | MVP | Sans elle, rien à trier |
| F2 | Regroupement automatique des visages non attribués | MVP | Choix de l'utilisateur « regrouper puis nommer » ; nommer visage par visage est impraticable sur 1 000 photos |
| F3 | Nommage et correction des groupes en GUI (nommer, fusionner, retirer un visage) | MVP | Seul moyen d'associer un prénom ; les erreurs de regroupement sont certaines (H4), la correction est donc nécessaire |
| F4 | Reconnaissance des nouveaux visages contre les personnes déjà nommées | MVP | C'est la « base visage + prénom » demandée : un second dossier se trie sans tout renommer |
| F5 | Tri par copie dans `Destination/Prénom/` avec aperçu et journal | MVP | Le résultat promis |
| F6 | Commandes CLI `scan`, `sort`, `status` | MVP | Choix de l'utilisateur (GUI + CLI) ; tri en lot et tests d'intégration simples |
| F7 | Déplacement ou liens physiques au lieu de copie | V2 | Copie choisie pour le MVP ; déplacer impose un journal d'annulation |
| F8 | Accélération GPU (DirectML) | V2 | Seulement si H3 échoue |
| F9 | HEIC et RAW | V2 | Selon le comptage H5 |
| F10 | Détecteur RetinaFace en option | V2 | Licence non commerciale (D1) ; seulement si YuNet est insuffisant |
| F11 | Export et import de la base des personnes | V2 | Utile pour changer de machine |
| F12 | Écriture des prénoms dans les métadonnées XMP | V2 | Modifie les fichiers ou ajoute des fichiers annexes ; hors de la promesse « originaux intacts » |
| - | Cloud, comptes, mobile, vidéo, âge et émotion | Hors périmètre | Voir section 8 |

## 2. Parcours utilisateurs

### P1 - Premier tri d'un dossier

1. L'utilisateur lance FaceSort, choisit un dossier source (E1).
2. Il lance l'analyse ; une barre de progression affiche photos traitées, visages trouvés, temps restant.
3. FaceSort regroupe les visages et ouvre l'écran Personnes (E2).
4. Pour chaque groupe, il tape un prénom et valide ; il fusionne deux groupes de la même personne, retire un visage mal classé, ignore les inconnus.
5. Il ouvre l'écran Tri (E3), choisit un dossier de destination, lit l'aperçu, lance la copie.

**Résultat** : `Destination/Léa/`, `Destination/Tom/`… contiennent des copies des photos où chacun apparaît ; un journal CSV liste chaque copie.

### P2 - Tri d'un nouveau dossier avec la base existante

1. L'utilisateur scanne un nouveau dossier (GUI ou `facesort scan <dossier>`).
2. Les visages proches d'une personne nommée lui sont attribués automatiquement (F4) ; les autres sont regroupés (F2).
3. Il nomme seulement les nouveaux groupes, puis trie (GUI ou `facesort sort <destination>`).

## 3. Écrans et surfaces

| ID | Écran / surface | Rôle | État vide | Chargement | Erreur | Succès |
|---|---|---|---|---|---|---|
| E1 | Accueil / Analyse | Choisir un dossier source, lancer le scan | « Aucun dossier analysé » + bouton « Choisir un dossier » | Barre de progression, compteurs, bouton Annuler | Liste repliable des fichiers en erreur avec la raison | « 1 240 photos, 2 310 visages, 12 erreurs » + bouton « Voir les personnes » |
| E2 | Personnes | Nommer, fusionner, corriger | « Aucun visage à nommer » + lien vers E1 | Vignettes chargées progressivement | Message si une vignette ne peut pas être lue (fichier déplacé) | Groupe nommé affiché dans la colonne « Personnes nommées » |
| E3 | Tri | Choisir la destination, aperçu, copie | « Aucune personne nommée » + lien vers E2 | Progression de copie, bouton Annuler | Erreur d'écriture : message, arrêt propre, chemin du journal | « 1 830 copies, 4,2 Go, journal : … » + bouton « Ouvrir le dossier » |
| E4 | CLI `facesort` | `scan`, `sort`, `status` | `status` sur base vide : « Base vide » | Progression sur stderr | Message sur stderr, code de sortie non nul | Résumé sur stdout, ou JSON avec `--json` |

## 4. Spécifications du MVP

### F1 - Scan et analyse

- **Objectif** : extraire chaque visage d'un dossier et le mémoriser.
- **Comportement attendu** : parcours récursif ; pour chaque JPEG ou PNG, correction de l'orientation EXIF, détection des visages, alignement sur 5 points, calcul de l'empreinte, enregistrement (photo, cadre, score, empreinte, vignette). Analyse incrémentale : une photo déjà analysée et inchangée (même chemin, taille, date de modification) n'est pas réanalysée.
- **Règles et limites** : visages de moins de 40 px de côté dans l'image d'origine ignorés **[H2]** ; score de détection minimal configurable ; image réduite à 1 600 px de grand côté pour la détection **[H3]**.
- **Erreurs et cas limites** : fichier corrompu, format non pris en charge, dossier inaccessible, annulation en cours de scan.
- **Critères d'acceptation** :
  - **AC-F1-1** - Étant donné le dossier de test `fixtures/basic` (10 JPEG dont 7 contiennent au moins un visage), quand l'utilisateur lance le scan, alors le résumé indique « 10 photos analysées » et le nombre de visages attendu par le fichier `fixtures/basic/expected.json` (à 1 près).
  - **AC-F1-2** - Étant donné un dossier déjà scanné et inchangé, quand l'utilisateur relance le scan, alors le résumé indique « 0 photo analysée, 10 inchangées ».
  - **AC-F1-3** - Étant donné un dossier contenant un JPEG tronqué, quand le scan s'exécute, alors ce fichier apparaît dans la liste des erreurs avec la raison « image illisible », et les autres photos sont analysées.
  - **AC-F1-4** - Le résumé du scan indique le nombre de fichiers ignorés par extension (par exemple « ignorés : 34 .heic, 2 .mp4 »).
  - **AC-F1-5** - Étant donné une photo portant l'orientation EXIF 6 (rotation 90°), quand elle est scannée, alors le même nombre de visages est détecté que sur la même photo redressée physiquement.
- **Vérification** : automatique (tests d'intégration sur fixtures) ; AC-F1-5 automatique avec une fixture dédiée.

### F2 - Regroupement

- **Objectif** : proposer des groupes « probablement la même personne » à nommer.
- **Comportement attendu** : après chaque scan, les visages non attribués sont regroupés par similarité cosinus des empreintes (liaison au-dessus d'un seuil, paramètre calibré en T11). Les groupes sont triés par taille décroissante.
- **Règles et limites** : un groupe de moins de 2 visages va dans « Visages isolés » ; un visage retiré d'une personne (F3) n'y est jamais réattribué automatiquement.
- **Erreurs et cas limites** : aucun visage non attribué (état vide E2) ; plusieurs milliers de visages (le regroupement de 5 000 visages se termine en moins de 10 s).
- **Critères d'acceptation** :
  - **AC-F2-1** - Étant donné `fixtures/people3` (3 personnes, 6 photos à visage unique chacune), quand le regroupement s'exécute, alors chaque groupe ne contient les visages que d'une seule personne.
  - **AC-F2-2** - Sur `fixtures/people3`, chaque personne est répartie dans 2 groupes au plus.
  - **AC-F2-3** - Deux regroupements successifs sur les mêmes visages produisent exactement les mêmes groupes.
  - **AC-F2-4** - Le regroupement de 5 000 empreintes synthétiques se termine en moins de 10 s en build release.
- **Vérification** : automatique.

### F3 - Nommage et correction (GUI)

- **Objectif** : associer un prénom à chaque groupe et corriger les erreurs.
- **Comportement attendu** : E2 affiche chaque groupe avec jusqu'à 12 vignettes et son nombre de visages ; champ « Prénom » ; actions « Fusionner avec… », « Retirer ce visage » (sur une vignette), « Ignorer ce groupe ».
- **Règles et limites** : le prénom devient un nom de dossier Windows : caractères `< > : " / \ | ? *` interdits, noms réservés (`CON`, `PRN`, `AUX`, `NUL`, `COM1`…`LPT9`) interdits, 1 à 64 caractères, espaces de début et fin supprimés, accents conservés.
- **Erreurs et cas limites** : prénom déjà utilisé, prénom invalide, vignette illisible.
- **Critères d'acceptation** :
  - **AC-F3-1** - Étant donné un groupe non nommé, quand l'utilisateur tape « Léa » et appuie sur Entrée, alors le groupe passe dans « Personnes nommées » sous le nom « Léa », et c'est encore le cas après redémarrage de l'application.
  - **AC-F3-2** - Étant donné une personne « Léa » existante, quand l'utilisateur nomme un autre groupe « léa », alors l'application propose de fusionner avec « Léa » et, après confirmation, une seule personne « Léa » contient les visages des deux groupes.
  - **AC-F3-3** - Quand l'utilisateur saisit « Tom/Léa » ou « CON », alors le prénom est refusé avec le message « Nom de dossier invalide sous Windows : <raison> » et rien n'est enregistré.
  - **AC-F3-4** - Quand l'utilisateur retire un visage de « Léa », alors ce visage n'apparaît plus chez Léa, ni après un nouveau scan ou regroupement.
  - **AC-F3-5** - Toutes les actions de E2 sont accessibles au clavier (Tab, Entrée, Échap) avec un focus visible.
- **Vérification** : AC-F3-2, AC-F3-3, AC-F3-4 automatiques sur la couche logique ; AC-F3-1 et AC-F3-5 manuelles (scénario M2).

### F4 - Reconnaissance avec la base

- **Objectif** : attribuer automatiquement les visages des personnes déjà connues.
- **Comportement attendu** : chaque nouveau visage est comparé aux empreintes de chaque personne nommée (similarité maximale avec ses visages confirmés). Au-dessus du seuil de reconnaissance, il est attribué et marqué « auto » ; sinon il part au regroupement (F2).
- **Règles et limites** : si les deux meilleures personnes sont à moins de 0,05 l'une de l'autre, le visage reste non attribué (ambiguïté). Seuls les visages confirmés (nommés par l'utilisateur ou fusionnés) servent de référence, pas les visages « auto », pour éviter la dérive.
- **Critères d'acceptation** :
  - **AC-F4-1** - Étant donné « Léa » nommée à partir de `fixtures/people3/a`, quand l'utilisateur scanne `fixtures/people3_new` contenant 3 nouvelles photos de Léa, alors ces 3 visages sont attribués à Léa avec le marqueur « auto ».
  - **AC-F4-2** - Un visage d'une personne inconnue de `fixtures/people3_new` n'est attribué à aucune personne nommée.
  - **AC-F4-3** - Un visage dont les similarités avec deux personnes diffèrent de moins de 0,05 reste non attribué (test unitaire avec empreintes synthétiques).
- **Vérification** : automatique.

### F5 - Tri par copie

- **Objectif** : produire les dossiers par personne.
- **Comportement attendu** : pour chaque photo contenant au moins une personne nommée, copie dans `Destination/<Prénom>/<nom du fichier>` pour chaque personne présente. Option (activée par défaut) : photos contenant uniquement des visages non nommés copiées dans `Destination/_Inconnus/`. Photos sans visage : non copiées.
- **Règles et limites** : la destination ne peut pas être à l'intérieur d'un dossier source scanné ; écriture dans un fichier temporaire puis renommage ; originaux jamais modifiés ; dates de modification conservées sur les copies.
- **Critères d'acceptation** :
  - **AC-F5-1** - Avant toute copie, l'aperçu (E3 ou `sort --dry-run`) affiche le nombre de copies par personne, la taille totale et l'espace libre de la destination, et aucun fichier n'est écrit.
  - **AC-F5-2** - Une photo contenant Léa et Tom est copiée dans `Léa/` et `Tom/` ; les deux copies ont le même SHA-256 que l'original, et l'original garde son SHA-256 et sa date de modification.
  - **AC-F5-3** - Une photo ne contenant que des visages non nommés est copiée dans `_Inconnus/` si l'option est active, et nulle part sinon.
  - **AC-F5-4** - Si `Léa/IMG_0001.jpg` existe avec un contenu différent, la copie est nommée `IMG_0001 (2).jpg` ; avec un contenu identique, elle est sautée et comptée « déjà présente » (un second tri identique fait 0 copie).
  - **AC-F5-5** - En cas d'erreur d'écriture (destination en lecture seule, disque plein), le tri s'arrête, affiche la cause et le chemin du journal, et aucun fichier partiel ne reste dans la destination.
  - **AC-F5-6** - Chaque tri écrit `Destination/facesort-log-AAAAMMJJ-HHMMSS.csv` (UTF-8, colonnes : source, destination, personne, statut).
- **Vérification** : automatique (dossiers temporaires) ; AC-F5-5 pour le disque plein : manuel (M4), lecture seule automatique.

### F6 - CLI

- **Objectif** : piloter scan et tri sans fenêtre, partager la base avec la GUI.
- **Comportement attendu** : `facesort scan <dossier>...`, `facesort sort <destination> [--dry-run] [--no-unknown]`, `facesort status` (personnes, visages, photos), `--help` sur chaque commande, `--json` pour une sortie machine.
- **Critères d'acceptation** :
  - **AC-F6-1** - `facesort --help` et `facesort <commande> --help` décrivent chaque option avec un exemple.
  - **AC-F6-2** - Codes de sortie : 0 succès, 1 erreur fatale (dossier inexistant, base illisible), 2 arguments invalides, 3 succès partiel (au moins un fichier en erreur).
  - **AC-F6-3** - `facesort status --json` produit un JSON valide contenant `persons`, `faces`, `photos`, `unassigned_faces`.
  - **AC-F6-4** - Une personne nommée dans la GUI apparaît dans `facesort status` sans autre action (base partagée).
- **Vérification** : automatique (tests d'intégration du binaire).

## 5. Permissions et données sensibles

| Permission ou donnée | Pourquoi elle est nécessaire | Alternative plus faible envisagée | Ce qui se passe si elle est refusée |
|---|---|---|---|
| Lecture des dossiers source choisis | Analyser les photos | Aucune | Dossier signalé en erreur, scan des autres |
| Écriture dans le dossier destination choisi | Copier | Aucune | Erreur AC-F5-5 |
| Base `%LOCALAPPDATA%\FaceSort\facesort.db` | Empreintes, prénoms, vignettes | Base à côté de l'exécutable (écartée : dossier parfois en lecture seule) | Message au démarrage, chemin alternatif via `--db` |
| Empreintes faciales associées à des prénoms (données biométriques) | Reconnaissance F4 | Ne rien conserver entre deux sessions (perd F4) | Supprimer le fichier de base efface tout ; le chemin est affiché dans `status` et dans « À propos » |

Aucun accès réseau à l'exécution.

## 6. Accessibilité

GUI : navigation au clavier complète, focus visible, libellés sur chaque champ et bouton, vignettes accompagnées du nom du fichier source au survol et au focus, messages d'erreur en texte (pas seulement en couleur). CLI : messages d'erreur sur stderr avec la correction attendue, codes de sortie AC-F6-2.

## 7. Version 2

| ID | Fonctionnalité | Condition pour la lancer |
|---|---|---|
| F7 | Déplacement ou liens physiques | Espace disque devenu gênant (H6) |
| F8 | DirectML | H3 échoue |
| F9 | HEIC et RAW | Plus de 10 % de fichiers ignorés (AC-F1-4) |
| F10 | RetinaFace optionnel | Rappel de détection insuffisant au test T11 et usage resté personnel |
| F11 | Export et import de la base | Changement de machine |
| F12 | Prénoms en XMP | Besoin de retrouver les personnes dans un autre logiciel |

## 8. Hors périmètre

| Fonctionnalité | Raison |
|---|---|
| Cloud, synchronisation, comptes | Usage personnel sur une machine ; données biométriques à garder locales |
| Application mobile | Les photos sont triées sur le PC |
| Vidéo | Coût d'analyse élevé, autre problème |
| Âge, émotion, genre | Sans rapport avec le tri ; données sensibles supplémentaires |
