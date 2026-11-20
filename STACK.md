# Tube — stack technique

> Client YouTube natif pour Windows, en Rust. Document vérifié dans le code en novembre 2026 (revue EGM : dérive + relecture).

## 1. But et direction

**À quoi sert l'app.** Elle remplace youtube.com dans le navigateur, avec le compte de l'utilisateur :
- accueil personnalisé, abonnements, « À regarder plus tard », playlists, historique et recherche ;
- lecture jusqu'en 4K, directs, mini-lecteur ;
- l'historique remonte au compte, donc les recommandations continuent d'apprendre.

**Prérequis :** être connecté à YouTube dans Opera GX (ou Opera, Chrome, Edge) ; Python 3 ; les codecs VP9/AV1 du Microsoft Store.

**Principes, par ordre de priorité :**
1. **Fluidité avant tout.**
   - Moins de processeur et de mémoire qu'un onglet Chrome (mesures au §6).
   - Démarrage d'une vidéo en ~1 s quand elle a été extraite d'avance.
   - Défilement de grille à 7 ms par image en 2500×1350 (mesuré) ; objectif > 100 images/s en 5120×1440, pas encore mesuré.
2. **Rien d'inutile.**
   - Pas de cartes publicitaires ; segments sponsorisés sautés (SponsorBlock).
   - Pas de Shorts par défaut.
   - Pas de mot de passe : les cookies viennent du navigateur.
3. **Natif Windows.**
   - Décodage GPU via Media Foundation.
   - Touches multimédia et panneau média de Windows.
   - Raccourcis globaux.
   - Une seule instance (`Tube.exe <lien>` ouvre une vidéo).
4. **Même identité visuelle que les autres applis Rust de l'auteur** (Explorateur, Lecteur universel, Vigie, Launcher), grâce au **socle** : une crate commune à ces applis (dépôt GitHub `Desarrh/socle`) qui fournit thème, icônes, réglages, journal de plantage, instance unique et mode test.

**Choix écartés :**
- **Rendu GPU de l'interface (wgpu/glow)** : le rendu CPU (egui → hdrshot → softbuffer) prend moins de mémoire et aucun pilote 3D ; le coût est un rendu proportionnel au nombre de pixels (§8).
- **mpv / ffmpeg** : Media Foundation est intégré à Windows, décode au GPU et dessine directement dans une fenêtre, sans DLL à livrer.
- **Extracteur Rust natif** : yt-dlp suit les changements de YouTube en quelques jours ; un extracteur maison prendrait du retard à chaque changement.
- **Connexion OAuth / mot de passe** : les cookies du navigateur donnent le vrai compte sans rien saisir.
- **Extractions en parallèle** : elles donnaient des adresses bridées (~100 Ko/s) ; elles sont faites une à une, avec une file à priorité.

## 2. Stack

**Langage et compilation.**
- Rust (édition 2024, testé avec 1.99.1 ; version non épinglée).
- Profil release : `lto = "thin"`, `codegen-units = 4`, `opt-level = 3`, `strip = true`. Les dépendances sont compilées en `opt-level = 2` en mode dev.

**Crates principales** (`Cargo.toml`) :

| Crate | Version | Rôle |
|---|---|---|
| egui / egui-winit | 0.37.0 | Interface en mode immédiat (presse-papiers, liens) |
| winit | 0.30.13 | Fenêtre, boucle d'événements |
| softbuffer | 0.4.8 | Affichage de l'image calculée par le CPU |
| hdrshot-egui-backend | 0.2.0 | Rendu logiciel d'egui (tuiles, cache de primitives) |
| windows / windows-core | 0.62 | API Win32 et WinRT |
| ureq | 3 (rustls, gzip) | HTTP : InnerTube, miniatures, flux googlevideo |
| image | 0.25.10 (png, jpeg, webp) | Miniatures, captures |
| serde / serde_json | 1 | Réglages, réponses JSON |
| sha1_smol / sha2 | 1 / 0.10 | SAPISIDHASH, préfixes SponsorBlock, vérification de deno |
| bytemuck | 1 | Conversion des tampons de pixels |
| dirs | 7.0.0 | `%LOCALAPPDATA%`, dossier Images |
| winresource (build) | 0.1.34 | Icône et informations de version de l'exe |
| **socle** | git tag **v0.1.5** (`ui`, `settings`, `crash`, `instance`, `testmode`) | `theme`, `icons`, `ui` (palette Ctrl+K, champ d'autocomplétion), `autocomplete`, `keys` (touches physiques, AZERTY), `paths`, `settings` (écriture atomique), `crash`, `instance`, `testmode`. Développement local : `.cargo/config.toml` (non versionné) pointe vers `../socle` |

**Rendu.** L'interface est rendue par le **CPU** :

```text
egui → hdrshot-egui-backend → softbuffer
```

Le décodage et l'affichage de la vidéo ne passent pas par le CPU. Seules quelques images en sont copiées : captures, image figée en pause, couleurs des bords en mode cinéma.

**Pipeline vidéo :**

| Étape | Choix |
|---|---|
| Adresses des flux | **yt-dlp**, chargé en permanence dans un processus Python (le « worker »). Les défis JavaScript de YouTube sont résolus par **deno** |
| Format | DASH : vidéo et son séparés |
| Codecs | **VP9** préféré. AV1 ou H.264 n'est pris que si VP9 s'arrête sous leur définition et sous 1440p. Son **Opus**, sinon AAC |
| Pistes exclues | Pistes « drc » et HDR. La piste audio **originale** est choisie (doublages ignorés par défaut) |
| Directs | HLS, client `mweb` (vidéo et son dans le même flux) |
| Décodage et affichage | **Media Foundation** `IMFMediaEngine` en mode `MF_MEDIA_ENGINE_PLAYBACK_HWND` : décodage GPU, rendu direct dans une fenêtre enfant |
| Vidéo + son | Source agrégée `MFCreateAggregateSource`. Le moteur ne sait ouvrir qu'une adresse : une extension du moteur résout l'adresse `tubeagg://…` en cette source agrégée |
| Réseau | `IMFByteStream` maison (`http.rs`, `live.rs`) |
| ffmpeg / mpv | **Aucun** |

Définitions testées : 1440p60 et 2160p50/60.

**API Windows utilisées :**
- Media Foundation et Media Engine.
- WASAPI *process loopback* (`Win32_Media_Audio`) : spectre du mode audio seul.
- WinRT `SystemMediaTransportControls` (`Media`, `Storage_Streams`, `Win32_System_WinRT`).
- Restart Manager et ToolHelp : copie des cookies du navigateur, sur clic seulement (§3).
- Shell (`IShellLinkW`) et registre : drapeau d'Opera (§3).
- Crochet clavier bas niveau `WH_KEYBOARD_LL` : raccourcis globaux.
- GDI / `PrintWindow` sur la fenêtre vidéo : captures, image figée en pause, couleurs du mode cinéma.
- DWM.
- `SetWindowRgn` : découpe de la fenêtre vidéo sous les éléments d'interface.

**Dépendances externes et mises à jour** (dans `%LOCALAPPDATA%\Tube\bin`) :
- **yt-dlp** dans un venv Python (`bin\py`), créé au premier lancement (`py -3 -m venv`, sinon `python -m venv`, puis `pip install yt-dlp[default]`). Mis à jour par `pip -U` au démarrage de Tube si la dernière vérification date de plus de 20 h.
- **deno** (`bin\deno.exe`) : version fixe (`DENO_VERSION` dans `worker.rs`), archive GitHub vérifiée par SHA-256 (`DENO_SHA256`) avant extraction. Mise à jour : changer les deux constantes.
- Python est lancé isolé (`python -I -X utf8`).

**Build :**

```powershell
cargo build --release
```

- Exe produit : `target\release\tube.exe` (environ 10 Mo), copié à la main en `dist\Tube.exe`.
- Tests unitaires : `cargo test --release` (index MP4, cookies, choix des formats).

## 3. Architecture

| Fichier | Rôle |
|---|---|
| `main.rs` | Boucle winit, rendu logiciel, opérations de fenêtre (plein écran, demi-écran, mini-lecteur, opacité), branchement de l'instance unique (`socle::instance`), scénarios de test `TUBE_SCRIPT` |
| `app.rs` | État et logique : vues, onglets, file d'attente, lecture, reprise, historique YouTube, SponsorBlock, qualité adaptative, sous-titres, directs, minuterie, import des cookies |
| `ui.rs` | Toute l'interface egui : barre d'outils, onglets, grilles, page vidéo, contrôles, menu ⚙, superpositions plein écran / mini, réglages, aide |
| `api.rs` | InnerTube, l'API interne du site YouTube (`youtubei/v1`) : browse, search, next, player (suivi de lecture), feedback, edit_playlist ; analyse des cartes |
| `cookies.rs` | Cookies au format Netscape (youtube.com seulement), en-tête SAPISIDHASH, copie des fichiers du profil du navigateur |
| `opera.rs` | Drapeau `--disable-features=LockProfileCookieDatabase` sur les raccourcis d'Opera GX et sa clé Run (voir Authentification) |
| `worker.rs` / `worker.py` | Processus Python yt-dlp persistant, JSON ligne à ligne ; déchiffrement des cookies du navigateur ; installation du venv et de deno ; mise à jour |
| `player.rs` | Enveloppe `IMFMediaEngine` : ouverture, saut approximatif, volume, vitesse, statistiques, événements |
| `http.rs` | Flux googlevideo : blocs de 512 Ko ; 3 requêtes de préchargement par flux (`PARALLEL`), de 4 blocs chacune (`FETCH`), jusqu'à 48 blocs (24 Mo) d'avance (`AHEAD`) ; réessai des requêtes bridées ; mesure de débit ; flux préparés au survol ; un fil de lecture par flux |
| `mp4.rs` | Index `mfra` synthétisé depuis le `sidx` (démarrage et sauts rapides des MP4 fragmentés) |
| `live.rs` | Directs HLS : segments TS mis bout à bout, retour arrière par numéro de segment, IPv4 forcé |
| `vidwin.rs` | Fenêtre enfant vidéo (transparente à la souris), découpes, capture de l'image, couleurs des bords |
| `thumbs.rs` | Miniatures : 6 fils, décodage à la taille exacte en pixels, ~400 textures, fichiers d'images gardés jusqu'à 80 Mo |
| `store.rs` | Chemins, réglages (via `socle::settings`), cache disque des extractions, journal `tube.log`, mode test isolé |
| `hotkeys.rs` | Raccourcis globaux Win+Alt |
| `smtc.rs` | Panneau média Windows, touches multimédia |
| `viz.rs` | Capture du son de Tube + FFT : spectre du mode audio seul |
| `sponsor.rs` | SponsorBlock (préfixe sha256 de l'identifiant) |

**Flux de données.**
1. L'interface demande une page : `api` (InnerTube, avec cookies) renvoie une liste de `Item`.
2. Au clic, au survol (≥ 200 ms) ou en préchargement, `worker` (yt-dlp) extrait les formats.
3. `pick()` choisit les flux vidéo et son.
4. `Player::open` crée l'extension `tubeagg://`, qui ouvre deux `ByteStream` (`http::Remote`) et une source agrégée.
5. Media Foundation décode et dessine dans `vidwin`.
6. En parallèle, des fils lancent les pings d'historique, SponsorBlock, les suggestions et les sous-titres.

**Fils d'exécution :**
- fil UI (winit) ;
- tick logique toutes les 250 ms pendant la lecture ;
- 6 fils de miniatures ;
- dans le worker Python, un fil d'extraction avec file à priorité (0 = clic, 1 = survol, 2 = préchargement). Une extraction en cours n'est pas interrompue : un clic peut attendre la fin d'un préchargement ;
- jusqu'à 3 fils de préchargement HTTP par flux, plus un fil de lecture pour Media Foundation ;
- un fil HLS par direct ;
- un fil de capture audio (audio seul, en lecture seulement) ;
- les fils du crochet clavier et du panneau média Windows ;
- écritures disque (réglages, cache des extractions) et lancement de Python dans des fils.

Les messages vers l'UI passent par un canal `mpsc` (`Msg`), suivi d'un `request_repaint`.

**Authentification.**
- Seuls les cookies **youtube.com** du navigateur sont copiés : ils portent toute la session YouTube. Ceux de google.com (accès au reste du compte) ne sont jamais gardés, et un ancien fichier qui en contenait est réécrit sans eux.
- Copie dans `%LOCALAPPDATA%\Tube\cookies.txt`. yt-dlp déchiffre les cookies du profil (nom du navigateur passé à `extract_cookies_from_browser`).
- **Le navigateur n'est jamais interrompu sans clic.** Les réimportations automatiques (copie de plus de 30 min au démarrage, session perdue : au plus une fois toutes les 10 min) ne font qu'une copie directe. Si le fichier est verrouillé, un bandeau propose « Reconnecter » ; ce clic arrête le seul *network service* du navigateur (Restart Manager), qui redémarre aussitôt (~1 s de réseau coupé).
- **Voie préférée : le drapeau d'Opera.** `opera.rs` ajoute `--disable-features=LockProfileCookieDatabase` aux raccourcis d'Opera GX (barre des tâches, menu Démarrer, Bureau) et à sa valeur de lancement automatique (`HKCU\…\CurrentVersion\Run`). Le fichier de cookies n'est alors plus verrouillé. C'est idempotent, et chaque raccourci ou valeur est sauvegardé (`.lnk`, `.reg`) dans `sauvegarde-raccourcis-opera` avant modification. Effet au prochain démarrage d'Opera. Une mise à jour d'Opera peut recréer ses raccourcis : on relance alors le bouton « Réappliquer le réglage d'Opera » du bandeau, `Tube.exe --opera-drapeau` ou `scripts\opera-drapeau.ps1`.
- InnerTube utilise l'en-tête `SAPISIDHASH`, une signature dérivée du cookie SAPISID, comme le site web. yt-dlp n'utilise le compte que si la vidéo l'exige (âge, membres, privée).
- Aucun mot de passe n'est demandé ni lu.

## 4. Direction artistique

**Palette, polices, icônes : celles du socle** (`socle::theme::palette(dark)`, `socle::theme::install_fonts_with`, `socle::icons::Icon`), communes aux applis Rust (thème crème en clair, bleu nuit en sombre). Les valeurs font foi dans le socle.

**Propre à Tube :**
- couleurs d'état (`ui.rs`) : `RED` `#E53B3B` (progression, pastilles « Reprendre », badge 4K/HD, direct), `SB` `#00D400` (segments SponsorBlock) ;
- texte secondaire des cartes (`sub_text`) : `text` mélangé à 40 % de `muted`, pour plus de contraste ;
- polices bold (titres des cartes), emoji et symbol activées ;
- en plein écran et en mini-lecteur, l'interface passe en superposition sombre (noir à 70–75 %) sur la vidéo ;
- en pause, image en noir et blanc assombrie ;
- mode cinéma : la vidéo garde sa place exacte et les bandes noires reprennent, en dégradé, les couleurs de ses bords.

**Logo.**
- `assets/make_icon.py` : carré arrondi en dégradé violet `#7C5CFF` vers bleu `#2884FF`, écran blanc, triangle de lecture, voyant corail `#FF7060`.
- Décliné de 16 à 256 px (en 16 px : triangle seul).

## 5. Conventions

**Raccourcis locaux** (aide complète avec F1) :

| Touche | Action |
|---|---|
| Espace / K, clic sur la vidéo | Lecture / pause |
| J / L, ← / → | −10 / +10 s, −5 / +5 s |
| ↑ / ↓, M | Volume, couper le son |
| F / double-clic | Plein écran |
| Échap | Quitter le plein écran / le mini-lecteur |
| I | Mini-lecteur |
| H | Demi-écran |
| C | Mode cinéma |
| V | Sous-titres |
| S | Capture d'image |
| , / . | Image par image |
| 0 … 9, Début | Aller à 0–90 %, recommencer |
| Maj+, / Maj+. | Vitesse − / + (0,25 à 3×) |
| Maj+N | Vidéo suivante de la file |
| Ctrl+Z | Annuler le saut SponsorBlock |
| Ctrl+C | Copier le lien horodaté |
| Ctrl+← / Ctrl+→ | Chapitre (ou moment fort) précédent / suivant |
| Ctrl+K | Palette de commandes (sections, onglets, lecture, qualité, vitesse, minuterie, affichage) |
| Ctrl+1 … 5 | Sections (touche physique, compatible AZERTY via `socle::keys`) |
| Ctrl+F, / | Recherche |
| Ctrl+T / Ctrl+W / Ctrl+Tab / Ctrl+Maj+Tab / Ctrl+Maj+T | Onglets |
| Alt+←, Retour arrière, bouton arrière de la souris | Retour |
| F5 | Actualiser |
| F2 | Statistiques |
| Ctrl+molette | Taille des vignettes ; opacité du mini-lecteur |
| Double-clic à gauche / à droite | −10 / +10 s (plein écran) |

**Raccourcis globaux** (actifs uniquement quand une vidéo est chargée) :
- **Win+Alt+** Espace, ←/→, ↑/↓, M, H, N ; l'état de Win est revérifié auprès de Windows (après Win+L, son relâchement peut être perdu) ;
- touches multimédia du clavier, via le panneau média Windows.

**Données.**
- `%APPDATA%\Tube` : `settings.json` (socle, écriture atomique ; l'ancien fichier de `%LOCALAPPDATA%` est repris une fois puis renommé `settings.json.migre`) et `crash.log` (socle, signalé au lancement suivant).
- `%LOCALAPPDATA%\Tube` : le reste.

| Fichier | Contenu |
|---|---|
| `cookies.txt` | Cookies youtube.com |
| `extraits.json` | 40 extractions récentes encore valables (démarrage sans nouvelle extraction, même après un redémarrage) |
| `tube.log` / `tube.old.log` | Journal horodaté (rotation à 1 Mo) |
| `worker.py` | Réécrit à chaque lancement du worker |
| `bin\` | Outils (venv yt-dlp, deno) |
| `import-N\` | Copie temporaire du profil du navigateur pendant un import, effacée à la fin et au lancement suivant |

Les captures d'image vont dans `Images\Tube`. Les sauvegardes des raccourcis d'Opera vont dans `sauvegarde-raccourcis-opera` (à côté du projet en développement, sinon dans `%LOCALAPPDATA%\Tube`).

**Format des réglages.**
- JSON plat : thème, volume, son coupé, qualité max, vitesse, vitesse par chaîne, position de reprise par vidéo (2 000 au plus), onglets ouverts et onglet actif, vidéos récentes (40 au plus), historique de recherche, langue des sous-titres, taille des vignettes, taille et position de la fenêtre et du mini-lecteur, options (Shorts, aperçu au survol, mode cinéma, vidéos vues, SponsorBlock et catégories, lecture auto, remontée de l'historique).
- Vérifié toutes les 30 s et écrit seulement s'il a changé (dans un fil) ; écrit aussi à la fermeture.

**Style de code.**
- Commentaires et messages en français ; fonctions courtes nommées en anglais.
- Pas de `unwrap` sur les données réseau : `Option` / `Result<_, String>`, journalisés dans `tube.log`.
- `#[implement]` pour les objets COM.

**Gestion d'erreurs.**
- Rétablissement automatique : adresse expirée, lecture bloquée 12 s, segments de direct refusés, worker relancé. Tube refait l'extraction et reprend à la même position. Le worker réutilise une extraction pendant 120 s.
- Notification en bas de la fenêtre (~3 s).

**Lancement et mise à jour.**
- `Tube.exe [lien YouTube | recherche]` : un lien lance la lecture, un autre texte lance une recherche (ce qui permet à une autre appli d'ouvrir une recherche dans Tube).
- `Tube.exe --opera-drapeau` : (ré)applique le drapeau d'Opera.
- Un second lancement transmet ses arguments à l'instance ouverte (`socle::instance` : mutex + tube nommé, < 1 ms).
- L'exe est remplacé manuellement dans `dist\`.

**Tests.**
- Unitaires : `cargo test --release`.
- Le mode test isolé s'active avec `TUBE_TEST=1`, `TUBE_SCRIPT`, `TUBE_QUIT_AFTER`, `SOCLE_TEST=1` ou `--test-hors-ecran`. Il ne dérange jamais l'utilisateur :
  - fenêtre invisible (`socle::testmode::hide`), sans focus, muette, en priorité basse ;
  - ni raccourcis globaux, ni panneau média, ni import de cookies ;
  - données à part (`%TEMP%\tube-test`, avec une copie des seuls cookies youtube.com, effacée à la sortie) et instance à part ;
  - aucune entrée simulée.
- Scénario scripté : `TUBE_SCRIPT="12:toggle;15:shot:C:\x.png"` (secondes depuis le lancement : commande : argument ; liste des commandes dans `main.rs`), `TUBE_QUIT_AFTER=30` (secondes).
- Journaux détaillés : `TUBE_LOG_EVENTS`, `TUBE_LOG_READS`, `TUBE_LOG_PAINT` ; captures `TUBE_CAPTURE` / `TUBE_CAPTURE_AFTER` ; `TUBE_DUMP_URL` (adresse refusée dans `url403.txt`).

## 6. Fonctionnalités

**Pages et navigation :**
- Accueil personnalisé (avec la rangée « Reprendre »), abonnements, « À regarder plus tard », playlists, historique, pages de chaîne.
- Recherche avec autocomplétion floue (`socle`) : l'historique local d'abord, puis les suggestions YouTube ; Tab complète.
- Palette de commandes Ctrl+K.
- Défilement infini, onglets mémorisés, pages gardées en mémoire avec leur position.
- Menu clic droit : nouvel onglet, lire ensuite, file d'attente, « À regarder plus tard » sur le compte, « Pas intéressé », « Ne pas recommander la chaîne ».

**Lecture :**
- Jusqu'en 4K60 ; qualité adaptative si le débit ne suit pas.
- Menu ⚙ : qualité, vitesse (0,25 à 3×, mémorisée par chaîne), langue audio (originale par défaut), sous-titres avec traduction automatique en français, minuterie, boucle, audio seul avec spectre, mode cinéma, capture.
- Chapitres, courbe « les plus revus », vignette de prévisualisation au survol de la barre, SponsorBlock.
- Reprise locale et reprise depuis la progression YouTube.

**Affichage :**
- Plein écran bord à bord.
- Demi-écran (pensé pour le 5120×1440).
- Mini-lecteur toujours au premier plan, déplaçable, redimensionnable, aimanté aux bords, avec opacité réglable.
- Pause en noir et blanc, attente réseau avec roue de chargement.

**Directs :** lecture HLS, retour arrière d'environ 35 min, bouton « EN DIRECT ».

**Améliorations Y1 à Y4 (faites) :**
- **Y1, enchaînement sans attente.** 30 s avant la fin, la vidéo suivante (file d'attente, sinon suggestion en lecture auto) est extraite en priorité et ses premiers blocs vidéo et son sont téléchargés. Mesure : la suivante démarre **0,18 s** après la fin.
- **Y2, moments forts.** Sur une vidéo sans chapitres, les pics de la courbe « les plus revus » (au plus 8, espacés d'au moins 4 % de la durée, à ≥ 55 % du maximum) servent de repères : losanges au-dessus de la barre, Ctrl+← / Ctrl+→ y sautent, 3 s avant le pic. YouTube ne fournit la courbe que pour une partie des vidéos ; sans courbe, Ctrl+→ avance de 10 s.
- **Y3, audio seul en arrière-plan.** Si la fenêtre est réduite depuis 5 s sur une vidéo de catégorie « Music », Tube passe en audio seul : le flux vidéo n'est plus téléchargé ni décodé. À la restauration, la vidéo revient au même endroit (~2 s). Le mini-lecteur et le plein écran ne sont pas concernés.
- **Y4, recherche locale sans attente réseau.** Pendant la frappe, les vidéos déjà vues (récentes et page Historique, ~400 titres en mémoire) sont proposées avec « ▶ », en tête de liste, avant l'arrivée des suggestions YouTube. Entrée sur l'une d'elles lance la lecture directement.

**Divers :**
- Aperçu vidéo muet au survol des vignettes, après 0,65 s.
- Remontée de l'historique au compte.
- Touches multimédia, panneau média Windows, raccourcis Win+Alt.

**Mesures en usage réel (i9-9900K + RTX 3070 Ti, vidéo 1440p60, fenêtre normale) :**

| | Tube | YouTube dans Chrome |
|---|---|---|
| Processeur | ~7,6 % d'un cœur (Tube + worker) | ~15 % d'un cœur |
| Mémoire | ~450 Mo (2 processus, en lecture 1440p) | ~1 Go (≈ 680 Mo privés, 16 processus) |
| Images perdues | 0 en 1440p60 ; 0 sur 1 472 images en 2160p50 | — |
| Clic → lecture | ~0,8–1,2 s si extraite d'avance ; extraction seule 2,5–4 s | — |
| Saut dans la vidéo | 1,0–1,4 s (5 sauts mesurés) | — |
| Défilement de grille (2500×1350) | 7 ms par image | — |
| Enchaînement (Y1) | vidéo suivante en lecture 0,18 s après la fin | — |
| Direct : retour au direct / recul de 15 min | 0,7 s / 1,4 s | — |

Mode test (1080p60, fenêtre invisible) : première image de l'interface 79–119 ms ; Tube 183–200 Mo, worker ~98 Mo.

## 7. Intégrations

- **Lien en ligne de commande** : `Tube.exe <lien youtube>` ouvre la vidéo, dans l'instance déjà lancée s'il y en a une. N'importe quelle appli peut donc déléguer un lien YouTube à Tube.
- **Lecteur universel** : partage le principe de fenêtre vidéo Media Foundation (`vidwin.rs` en est issu). Pas d'appel croisé à ce jour.
- **Vigie** : partage le thème du socle. Pas d'intégration fonctionnelle.
- **Launcher** : aucune intégration codée. Piste : ajouter Tube et relayer les liens YouTube collés.
- **Opera GX** : Tube modifie ses raccourcis et sa clé Run (drapeau de cookies, §3), avec sauvegardes. C'est le seul effet de Tube hors de ses propres dossiers.

## 8. Limites connues, dette technique, risques

**Risques côté YouTube :**
- **Conditions d'utilisation** : c'est un client non officiel. Les comptes utilisés via cookies peuvent en théorie être limités (avertissement du wiki yt-dlp). Accepté : usage personnel, rythme de requêtes d'un humain.
- **Anti-robot et bridage.**
  - googlevideo bride au hasard certaines requêtes (~100 Ko/s) et plafonne des réponses (~1 Mo). Parade : requêtes parallèles, réessais des requêtes lentes, qualité adaptative. Les débits restent irréguliers.
  - Extraire plusieurs vidéos en parallèle donnait des adresses bridées : les extractions sont en série.
  - yt-dlp n'a pas de fournisseur de *PO token* (jeton anti-robot que YouTube peut exiger pour les flux). Accepté tant que YouTube ne l'exige pas plus largement ; yt-dlp suivra.
- **Dépendance à yt-dlp et deno** : YouTube casse régulièrement l'extraction. La mise à jour de yt-dlp (au démarrage, après 20 h) corrige généralement en quelques jours.
- **Changements d'InnerTube** : l'analyse des cartes et `CLIENT_VERSION` (`2.20261117.01.00`, codé en dur dans `api.rs`) peuvent casser l'accueil, la recherche ou les retours « Pas intéressé ». Parade : changer la constante quand YouTube refuse l'ancienne.
- **Directs** :
  - le client `mweb` refuse parfois des segments (403) ; Tube refait l'extraction tout seul ;
  - la fenêtre de retour arrière (~35 min) est déduite, pas garantie ;
  - les adresses de segments sont liées à l'IPv4.

**Sécurité :**
- **Cookies en clair** dans `%LOCALAPPDATA%\Tube\cookies.txt` (profil Windows de l'utilisateur) : youtube.com seulement, jamais google.com (Gmail, Drive…).
- **yt-dlp** est installé et mis à jour depuis PyPI sans version fixe : c'est le prix des correctifs rapides face à YouTube. Python est lancé isolé (`-I`).
- **deno** : version fixe, SHA-256 vérifié.
- **Crochet clavier global** : n'intercepte que Win+Alt+touche, et seulement quand une vidéo est chargée ; tout le reste passe. Désactivé en mode test.
- **Opera** : raccourcis et clé Run modifiés avec sauvegarde (§3) ; le drapeau réduit la protection du fichier de cookies d'Opera contre la lecture par d'autres programmes. Choix de l'utilisateur.
- Les pings d'historique ne partent que vers `https://www.youtube.com/`.

**Dépendances et environnement :**
- Python 3 doit être installé (lanceur `py -3`, sinon `python`).
- L'audio seul avec spectre (process loopback) demande Windows 11 ; sinon, repli sans spectre.

**Coûts et dette technique :**
- **Rendu CPU** : son coût croît avec le nombre de pixels ; en mode cinéma, les bandes redessinent l'interface 4 fois par seconde. L'interface au-dessus de la vidéo passe par des découpes de fenêtre (`SetWindowRgn`).
- `app.rs` et `ui.rs` font plus de 2 000 lignes chacun : à découper en modules (lecteur, onglets, menus) au prochain gros changement.
- Peu de tests automatisés (index MP4, cookies, choix des formats) ; le reste passe par des scénarios scriptés.
- Les écrans « Connexion » et « Paramètres » n'ont pas eu de travail de mise en page.

## 9. Pistes (non implémentées : validation requise)

Ordre : gain pour l'usage quotidien d'abord. **(I)** = inédite, **(E)** = existe ailleurs. L'effort va de S (petit) à L (gros).

| # | Piste | Type | Effort | Comment elle reste rapide |
|---|---|---|---|---|
| 3 | **DeArrow** : titres et miniatures corrigés par la communauté (anti-clickbait), avec bascule | E (FreeTube, Grayjay) | M | Requête par préfixe de hachage par lot de cartes, en arrière-plan ; titre d'origine affiché tout de suite, remplacé à l'arrivée ; cache disque |
| 4 | **Filtres de recherche** : durée, date, tri, type, en pastilles sous la barre | E (YouTube, FreeTube) | S | Paramètre `params` d'InnerTube ; résultats en ~300 ms comme une recherche normale |
| 5 | **Recherche dans la transcription** : Ctrl+F sur une vidéo cherche dans les sous-titres et liste les passages ; Entrée y saute | E (YouTube, partiel) | S | Sous-titres déjà en mémoire après le premier affichage ; recherche locale sans réseau |
| 6 | **Navigation clavier façon téléviseur** : flèches dans la grille, aperçu muet sur la carte sélectionnée, Entrée pour lire | I | M | Même aperçu que le survol, mais déclenché par la sélection ; extraction prioritaire de la carte focalisée |
| 7 | **Commentaires** (tri, horodatages cliquables qui marquent aussi la barre de progression) | E | M | Chargés en tâche de fond pendant le démarrage ; affichés à la demande ; pagination à la volée |
| 9 | **Badges de nouveautés** : nombre de nouvelles vidéos des abonnements sur l'entrée « Abonnements » | E (Grayjay, YouTube) | S | Interrogation légère toutes les 15 min en arrière-plan (trafic authentifié de plus vers InnerTube) |
| 10 | **Marathon de chaîne** : lire une chaîne dans l'ordre chronologique en sautant les vidéos vues, avec la file prête | I | M | Liste de la chaîne déjà paginée ; file construite localement ; enchaînement sans attente (Y1) |
| 12 | **Return YouTube Dislike** : nombre de « je n'aime pas » estimé sous la vidéo | E (FreeTube, extensions) | S | Une petite requête en parallèle de l'extraction (envoie l'identifiant de la vidéo à ce service) ; affichage à l'arrivée sans rien bloquer |
| 13 | **Vitesse automatique selon le débit de parole** : podcasts et conférences accélérés dans les passages lents | I | L | Analyse du son déjà capturé (Windows 11) ; décision locale, changement de vitesse sans recharger |
| 14 | **Import et export d'abonnements et de playlists locales** (sauvegarde, migration depuis FreeTube) | E (FreeTube, Grayjay) | M | Opération locale sur fichier ; aucune attente réseau |
| 15 | **Python embarqué** (version fixe, vérifiée comme deno) au lieu du Python du système | — | M | Supprime un prérequis ; ~20 Mo au premier lancement |
| 16 | **yt-dlp avec retour arrière** : nouvelle version testée sur une extraction avant de remplacer l'ancienne | — | M | Mise à jour en arrière-plan ; l'ancienne reste en service si la nouvelle échoue |

Les pistes 1, 2, 8 et 11 sont devenues Y1, Y2, Y3 et Y4 (§6).
