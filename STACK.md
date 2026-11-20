# Tube — stack technique

> Client YouTube natif pour Windows, en Rust. Document vérifié dans le code (`C:\Dev\tube`, novembre 2026).

## 1. But et direction

**À quoi sert l'app.** Elle remplace youtube.com dans le navigateur, avec le compte de l'utilisateur :
- accueil personnalisé, abonnements, « À regarder plus tard », playlists, historique et recherche ;
- lecture jusqu'en 4K, directs, mini-lecteur ;
- l'historique remonte au compte, donc les recommandations continuent d'apprendre.

**Principes, par ordre de priorité :**
1. **Fluidité avant tout.**
   - Moins de processeur et de mémoire qu'un onglet Chrome.
   - Démarrage d'une vidéo en ~1 s quand elle a été préchargée.
   - Défilement à plus de 100 images/s même sur un écran 32:9.
2. **Rien d'inutile.**
   - Pas de pub (cartes publicitaires filtrées, SponsorBlock).
   - Pas de Shorts par défaut.
   - Pas de mot de passe : les cookies viennent du navigateur.
3. **Natif Windows.**
   - Décodage GPU via Media Foundation.
   - Touches multimédia et panneau média de Windows.
   - Raccourcis globaux.
   - Une seule instance (`Tube.exe <lien>` ouvre une vidéo).
4. **Même identité visuelle que les autres applis Rust** (explorateur, lecteur universel, Vigie).

## 2. Stack

**Langage et compilation.**
- Rust 1.99.1, édition 2024.
- Profil release : `lto = "thin"`, `codegen-units = 4`, `opt-level = 3`, `strip = true`.

**Crates principales** (`Cargo.toml`) :

| Crate | Version | Rôle |
|---|---|---|
| **socle** | git tag v0.1.2 (`ui`, `settings`, `crash`, `instance`, `testmode`) | Thème, icônes, palette Ctrl+K, autocomplétion floue, touches AZERTY, réglages atomiques, journal de plantage, instance unique, mode test. Développement local : `.cargo/config.toml` (non versionné) pointe vers `../socle` |
| egui / egui-winit | 0.37.0 | Interface en mode immédiat (presse-papiers, liens) |
| winit | 0.30.13 | Fenêtre, boucle d'événements |
| softbuffer | 0.4.8 | Affichage de l'image calculée par le CPU |
| hdrshot-egui-backend | 0.2.0 | Rendu logiciel d'egui (tuiles, cache de primitives) |
| windows / windows-core | 0.62 | API Win32 et WinRT |
| ureq | 3 (rustls, gzip) | HTTP : InnerTube, miniatures, flux googlevideo |
| image | 0.25.10 (png, jpeg, webp) | Miniatures, captures |
| serde / serde_json | 1 | Réglages, réponses JSON |
| sha1_smol / sha2 | 1 / 0.10 | SAPISIDHASH (compte), préfixes SponsorBlock |
| dirs | 7.0.0 | `%LOCALAPPDATA%`, dossier Images |
| winresource (build) | 0.1.34 | Icône et informations de version de l'exe |

**Rendu.** L'interface est rendue par le **CPU** :

```
egui → hdrshot-egui-backend → softbuffer
```

C'est un choix assumé : peu de mémoire, aucun pilote 3D pour l'interface. La vidéo, elle, ne passe jamais par le CPU.

**Pipeline vidéo :**

| Étape | Choix |
|---|---|
| URLs des flux | **yt-dlp**, chargé en permanence dans un worker Python. Les défis JS de YouTube sont résolus par **deno** |
| Format | DASH : vidéo et son séparés |
| Codecs | **VP9** préféré. AV1 ou H.264 seulement s'ils montent plus haut (jusqu'à 1440p au moins). Son **Opus**, sinon AAC |
| Pistes exclues | Pistes « drc » et HDR. La piste audio **originale** est choisie (doublages ignorés par défaut) |
| Directs | HLS, client `mweb` (vidéo et son mêlés, itags 91 à 96) |
| Décodage et affichage | **Media Foundation** `IMFMediaEngine` en mode `MF_MEDIA_ENGINE_PLAYBACK_HWND` : décodage GPU, rendu direct dans une fenêtre enfant |
| Vidéo + son | Source agrégée `MFCreateAggregateSource`, via une extension `tubeagg://` |
| Réseau | `IMFByteStream` maison (`http.rs`, `live.rs`) |
| ffmpeg / mpv | **Aucun** |

- Prérequis des codecs : extensions *VP9 Video* et *AV1 Video* du Microsoft Store.
- Définitions testées : 1440p60 et 2160p50/60.

**API Windows utilisées :**
- Media Foundation et Media Engine.
- WASAPI *process loopback* (`Win32_Media_Audio`) : spectre du mode musique.
- WinRT `SystemMediaTransportControls` (`Media`, `Storage_Streams`, `Win32_System_WinRT`).
- Restart Manager et ToolHelp : copie des cookies du navigateur.
- Crochet clavier bas niveau `WH_KEYBOARD_LL` : raccourcis globaux.
- GDI / `PrintWindow` : captures, image figée en pause, couleurs du mode cinéma.
- DWM.
- `SetWindowRgn` : découpe de la fenêtre vidéo sous les éléments d'interface.

**Dépendances externes et mises à jour** (dans `%LOCALAPPDATA%\Tube\bin`) :
- **yt-dlp** dans un venv Python (`bin\py`). Créé au premier lancement (`py -3 -m venv` puis `pip install yt-dlp[default]`), puis **mis à jour une fois par jour** (`pip -U`).
- **deno** (`bin\deno.exe`), téléchargé au premier lancement.
- Python 3 doit être installé sur la machine.

**Build :**

```
cargo build --release --bin tube
```

- Exe produit : `target\release\tube.exe` (environ 10 Mo).
- Copie distribuée : `dist\Tube.exe`.
- `proto/proto.rs` (binaire `tube-proto`) est le prototype de faisabilité d'origine.

## 3. Architecture

| Fichier | Rôle |
|---|---|
| `main.rs` | Boucle winit, rendu logiciel, opérations de fenêtre (plein écran, demi-écran, mini-lecteur, opacité), instance unique, scénarios de test `TUBE_SCRIPT` |
| `app.rs` | État et logique : vues, onglets, file d'attente, lecture, reprise, historique YouTube, SponsorBlock, qualité adaptative, sous-titres, directs, minuterie |
| `ui.rs` | Toute l'interface egui : barre d'outils, onglets, grilles, page vidéo, contrôles, menu ⚙, superpositions plein écran / mini, réglages, aide |
| `api.rs` | InnerTube (`youtubei/v1`) : browse, search, next, player (tracking), feedback, edit_playlist ; analyse des cartes (lockupViewModel, videoRenderer…) |
| `cookies.rs` | Lecture et écriture des cookies (format Netscape), SAPISIDHASH, déverrouillage du fichier du navigateur (Restart Manager) |
| `worker.rs` / `worker.py` | Processus Python yt-dlp persistant, JSON ligne à ligne ; installation du venv et de deno ; mise à jour quotidienne |
| `player.rs` | Enveloppe `IMFMediaEngine` : ouverture, saut approximatif, volume, vitesse, statistiques, événements |
| `http.rs` | Flux googlevideo : blocs de 512 Ko, 4 connexions, réessai des requêtes bridées, cache borné, mesure de débit, flux « réchauffés » au survol |
| `mp4.rs` | Index `mfra` synthétisé depuis le `sidx` (démarrage et sauts rapides des MP4 fragmentés) |
| `live.rs` | Directs HLS : segments TS mis bout à bout, retour arrière par numéro de segment `sq`, IPv4 forcé |
| `vidwin.rs` | Fenêtre enfant vidéo (transparente à la souris), découpes, capture de l'image, couleurs des bords |
| `thumbs.rs` | Miniatures : 6 fils, décodage à la taille exacte en pixels, cache d'environ 400 textures |
| `store.rs` | Chemins, réglages (via `socle::settings`), journal `tube.log`, mode test isolé |
| *(socle)* | `socle::theme` (palette, polices), `socle::icons` (icônes vectorielles), `socle::ui` (palette Ctrl+K, champ d'autocomplétion), `socle::keys`, `socle::instance`, `socle::crash`, `socle::testmode` |
| `hotkeys.rs` | Raccourcis globaux Win+Alt |
| `smtc.rs` | Panneau média Windows, touches multimédia |
| `viz.rs` | Capture du son de Tube + FFT : spectre du mode musique |
| `sponsor.rs` | SponsorBlock (préfixe sha256 de l'identifiant) |

**Flux de données.**
1. L'interface demande une page : `api` (InnerTube, avec cookies) renvoie une liste de `Item`.
2. Au clic, au survol (≥ 200 ms) ou en préchargement, `worker` (yt-dlp) extrait les formats.
3. `pick()` choisit les flux vidéo et son.
4. `Player::open` crée l'extension `tubeagg://`, qui ouvre deux `ByteStream` (`http::Remote`) et une source agrégée.
5. Media Foundation décode et dessine dans `vidwin`.
6. En parallèle, des fils lancent les pings d'historique (`playbackTracking`), SponsorBlock, les suggestions et les sous-titres.

**Fils d'exécution :**
- fil UI (winit) ;
- tick logique toutes les 250 ms pendant la lecture ;
- 6 fils de miniatures ;
- dans le worker Python, un fil d'extraction avec file à priorité (0 = clic, 1 = survol, 2 = préchargement). Les extractions en parallèle donnaient des URLs bridées ;
- jusqu'à 4 fils de préchargement HTTP par flux ;
- un fil par lecture asynchrone demandée par Media Foundation ;
- un fil HLS par direct ;
- un fil de capture audio (mode musique) ;
- les fils du crochet clavier et du panneau média Windows.

Les messages vers l'UI passent par un canal `mpsc` (`Msg`), suivi d'un `request_repaint`.

**Authentification.**
- Les cookies YouTube/Google sont copiés depuis Opera GX (Opera, Chrome et Edge sont aussi détectés).
- Le fichier verrouillé du navigateur est débloqué avec le Restart Manager, en ne tuant que le *network service* du navigateur.
- Copie dans `%LOCALAPPDATA%\Tube\cookies.txt`, réimportée automatiquement si elle a plus de 30 min ou si la session est perdue.
- InnerTube utilise `SAPISIDHASH`. yt-dlp n'utilise le compte que si la vidéo l'exige (âge, membres, privée).
- Aucun mot de passe n'est demandé ni lu.

## 4. Direction artistique

**Palette.** Elle vient du socle (`socle::theme::palette(dark)`), identique à l'explorateur. L'ancien `src/theme.rs` est retiré (gardé en `theme.rs.migre`).

| Rôle | Clair (crème) | Sombre (bleu nuit) |
|---|---|---|
| `bg` | `#EFE6D3` | `#0F1B2D` |
| `chrome` (barres) | `#E4D8BF` | `#0B1525` |
| `surface` (contenu) | `#F5EDDC` | `#13223A` |
| `side` (volet) | `#EEE4CF` | `#101D31` |
| `code` (champs) | `#F7F0E1` | `#0D182A` |
| `border` | `#D6C8AA` | `#243756` |
| `soft` | `#E8DCC3` | `#1C2D48` |
| `text` | `#2A2620` | `#E7ECF4` |
| `muted` | `#625846` | `#9FB0C8` |
| `accent` | `#2E5E9E` | `#6FA8FF` |
| `sel_bg` / `hover_bg` | `#D5DDE6` / `#EBE1CB` | `#23406B` / `#182A44` |
| `live` / `add` / `del` | `#A9561A` / `#2F7346` / `#A83434` | `#F2A65A` / `#7FD49C` / `#F0A7A7` |

**Couleurs propres à Tube** (`ui.rs`) :
- `RED` `#E53B3B` : progression, pastilles « Reprendre », badge 4K/HD, direct ;
- `SB` `#00D400` : segments SponsorBlock ;
- texte secondaire des cartes (`sub_text`) : `text` mélangé à 40 % de `muted`, pour plus de contraste.

**Polices** (`socle::theme::install_fonts`) : Segoe UI (avec Bold, famille `bold`, pour les titres des cartes), Consolas, Segoe UI Emoji et Segoe UI Symbol en secours.

**Tailles de texte** (en points egui) :

| Élément | Taille |
|---|---|
| Titres de page | 20 |
| Titre de la vidéo | 19 |
| Logo « Tube » | 17 |
| Titres de carte | 14, ×1 à 1,3 selon la largeur de la carte |
| Chaîne / méta | 12,5 / 12 |
| Barre d'état | 11,5 |
| En-têtes de section | 11, majuscules |

**Arrondis :**

| Élément | Rayon |
|---|---|
| Widgets egui | 6 |
| Menus et champ de recherche | 8 |
| Fenêtres et menu ⚙ | 10 |
| Miniatures | 8 (coins masqués par maillage) |
| Boutons d'icône | 7 |
| Onglets | 7 |
| Pastilles | 4 à 5 |

**Espacements :**
- marges de grille 22 px, écart entre cartes 18 px ;
- barre d'outils `Margin::symmetric(12, 8)`, volet `(10, 14)`, rangées du volet 32 px ;
- lignes de menu 30 px ;
- barre de contrôles 46 px (50 en plein écran, 38 en mini-lecteur).

**Icônes.**
- Elles viennent de `socle::icons::Icon`, sur une grille de 24 avec un trait de 1,8, sans aucune police d'icônes.
- Renommages depuis l'ancien `Ic` de Tube : `Full` → `Fullscreen`, `ExitFull` → `Unfullscreen`, `Vol(n)` → `VolumeLevel(n)`, `Gear` → `Settings`, `List` → `Playlist`.
- Elles sont environ 18 % plus grandes qu'avant ; la mise en page a été vérifiée en capture.

**Logo.**
- `assets/make_icon.py` : carré arrondi en dégradé violet `#7C5CFF` vers bleu `#2884FF`, écran blanc, triangle de lecture, voyant corail `#FF7060`.
- Décliné de 16 à 256 px (en 16 px : triangle seul).

**Différences avec l'explorateur :**
- couleurs d'état propres à la vidéo (rouge, vert SponsorBlock) ;
- polices bold, emoji et symbol en plus ;
- icônes dessinées au lieu de glyphes ;
- en plein écran et mini-lecteur, l'interface passe en superposition sombre (noir à 70–75 %) sur la vidéo ;
- en pause, image en noir et blanc assombrie ;
- mode cinéma : bandes aux couleurs de l'image.

## 5. Conventions

**Raccourcis locaux** (aide complète avec F1) :

| Touche | Action |
|---|---|
| Espace / K, clic sur la vidéo | Lecture / pause |
| J / L, ← / → | −10 / +10 s, −5 / +5 s |
| ↑ / ↓, M | Volume, couper le son |
| F / double-clic | Plein écran |
| I | Mini-lecteur |
| H | Demi-écran |
| C | Mode cinéma |
| V | Sous-titres |
| S | Capture d'image |
| , / . | Image par image |
| 0 … 9, Début | Aller à 0–90 %, recommencer |
| Maj+, / Maj+. | Vitesse − / + |
| Maj+N | Vidéo suivante de la file |
| Ctrl+Z | Annuler le saut SponsorBlock |
| Ctrl+C | Copier le lien horodaté |
| Ctrl+← / Ctrl+→ | Chapitre précédent / suivant |
| Ctrl+K | Palette de commandes : tout Tube au clavier (sections, onglets, lecture, qualité, vitesse, minuterie, affichage) |
| Ctrl+1 … 5 | Sections (touche physique, compatible AZERTY via `socle::keys`) |
| Ctrl+F | Recherche |
| Ctrl+T / Ctrl+W / Ctrl+Tab / Ctrl+Maj+T | Onglets |
| Alt+← / bouton arrière de la souris | Retour |
| F5 | Actualiser |
| F2 | Statistiques |
| Ctrl+molette | Taille des vignettes ; opacité du mini-lecteur |
| Double-clic à gauche / à droite | −10 / +10 s (plein écran) |

**Raccourcis globaux** (actifs uniquement quand une vidéo est chargée) :
- **Win+Alt+** Espace, ←/→, ↑/↓, M, H, N ;
- touches multimédia du clavier, via le panneau média Windows.

**Données.**
- `%APPDATA%\Tube` : `settings.json` (socle, écriture atomique ; l'ancien fichier de `%LOCALAPPDATA%` est repris une fois puis renommé `settings.json.migre`) et `crash.log` (socle, signalé au lancement suivant).
- `%LOCALAPPDATA%\Tube` : le reste.

| Fichier | Contenu |
|---|---|
| `cookies.txt` | Cookies |
| `tube.log` | Journal horodaté |
| `worker.py` | Réécrit à chaque lancement |
| `bin\` | Outils (venv yt-dlp, deno) |

Les captures d'image vont dans `Images\Tube`.

**Format des réglages.**
- JSON plat : thème, volume, qualité max, vitesse, vitesse par chaîne, position de reprise par vidéo (2 000 au plus), onglets ouverts, vidéos récentes, langue des sous-titres, taille des vignettes, options (Shorts, aperçu au survol, mode cinéma, vidéos vues, SponsorBlock et catégories, lecture auto).
- Sauvegarde toutes les 30 s et à la fermeture.

**Style de code.**
- Commentaires et messages en français ; fonctions courtes nommées en anglais.
- Pas de `unwrap` sur les données réseau : `Option` / `Result<_, String>`, journalisés dans `tube.log`.
- `#[implement]` pour les objets COM.

**Gestion d'erreurs.**
- Rétablissement automatique : URL expirée, lecture bloquée 12 s, segments de direct refusés, worker relancé. Tube refait l'extraction et reprend à la même position.
- Toast discret pour l'utilisateur.

**Lancement et mise à jour.**
- `Tube.exe [lien YouTube | recherche]` : un lien lance la lecture, un autre texte lance une recherche (intégration « Guide » d'Ultimate).
- Un second lancement transmet ses arguments à l'instance ouverte (`socle::instance` : mutex + tube nommé, < 1 ms).
- yt-dlp se met à jour tout seul chaque jour.
- L'exe est remplacé manuellement dans `dist\`.

**Tests (règle n° 0).**
- Le mode test isolé s'active avec `TUBE_TEST`, `TUBE_SCRIPT`, `TUBE_QUIT_AFTER`, `SOCLE_TEST` ou `--test-hors-ecran`.
- Il est invisible (`socle::testmode::hide`), sans focus, muet, en priorité basse.
- Il n'installe ni raccourcis globaux ni panneau média, n'importe pas de cookies, et utilise ses propres données (`%TEMP%	ube-test`) et sa propre instance.
- Aucune entrée simulée.
- Scénarios scriptés : `TUBE_SCRIPT="t:commande;…"`, `TUBE_QUIT_AFTER`.
- Journaux détaillés : `TUBE_LOG_EVENTS`, `TUBE_LOG_READS`, `TUBE_LOG_PAINT`.

## 6. Fonctionnalités

**Pages et navigation :**
- Accueil personnalisé (avec la rangée « Reprendre »), abonnements, « À regarder plus tard », playlists, historique, pages de chaîne.
- Recherche avec autocomplétion floue (`socle`) : l'historique local d'abord, puis les suggestions YouTube ; Tab complète.
- Palette de commandes Ctrl+K.
- Défilement infini, onglets mémorisés, pages gardées en mémoire avec leur position.
- Menu clic droit : nouvel onglet, lire ensuite, file d'attente, « À regarder plus tard » sur le compte, « Pas intéressé », « Ne pas recommander la chaîne ».

**Lecture :**
- Jusqu'en 4K60 ; qualité adaptative si le débit ne suit pas.
- Menu ⚙ : qualité, vitesse (mémorisée par chaîne), langue audio (originale par défaut), sous-titres avec traduction automatique en français, minuterie, boucle, audio seul avec spectre, mode cinéma, capture.
- Chapitres, courbe « les plus revus », vignette de prévisualisation au survol de la barre, SponsorBlock.
- Reprise locale et reprise depuis la progression YouTube.

**Affichage :**
- Plein écran bord à bord.
- Demi-écran (pensé pour le 5120×1440).
- Mini-lecteur toujours au premier plan, déplaçable, redimensionnable, aimanté aux bords, avec opacité réglable.
- Pause en noir et blanc, attente réseau avec roue de chargement.

**Directs :** lecture HLS, retour arrière d'environ 35 min, bouton « EN DIRECT ».

**Vague QoL validée (Y1 à Y4) :**
- **Y1, enchaînement sans attente.** 30 s avant la fin, la vidéo suivante (file d'attente, sinon suggestion en lecture auto) est extraite en priorité et ses premiers blocs vidéo et son sont téléchargés. Mesure : la suivante démarre **0,18 s** après la fin.
- **Y2, moments forts.** Sur une vidéo sans chapitres, les pics de la courbe « les plus revus » (au plus 8, espacés d'au moins 4 % de la durée, à ≥ 55 % du maximum) servent de repères. Ils sont marqués par des losanges au-dessus de la barre, et Ctrl+← / Ctrl+→ y sautent, 3 s avant le pic. Calcul local en quelques µs. YouTube ne fournit la courbe que pour une partie des vidéos ; sans courbe, Ctrl+→ avance de 10 s.
- **Y3, arrière-plan intelligent.** Si la fenêtre est réduite depuis 5 s sur une vidéo de catégorie « Music », Tube passe en audio seul ; à la restauration, la vidéo revient au même endroit (~2 s). Ça économise le réseau et le GPU. Le mini-lecteur et le plein écran ne sont pas concernés.
- **Y4, recherche locale instantanée.** Pendant la frappe, les vidéos déjà vues (récentes et page Historique, ~400 titres en mémoire) sont proposées avec « ▶ », en tête de liste, avant l'arrivée des suggestions YouTube. Entrée sur l'une d'elles lance la lecture directement.

**Divers :**
- Aperçu vidéo muet au survol des vignettes, après 0,65 s.
- Remontée de l'historique au compte.
- Touches multimédia, panneau média Windows, raccourcis Win+Alt.

**Mesures (i9-9900K + RTX 3070 Ti, 1440p60) :**

| | Tube | YouTube dans Chrome |
|---|---|---|
| Processeur | ~7,6 % d'un cœur (Tube + worker) | ~15 % d'un cœur |
| Mémoire | ~450 Mo (2 processus) | ~1 Go (≈ 680 Mo privés, 16 processus) |
| Images perdues | 0 en 1440p60 ; 0 sur 1 472 images en 2160p50 | — |
| Clic → lecture | ~0,8–1,2 s si extraite d'avance ; extraction seule 2,5–4 s | — |
| Saut dans la vidéo | 1,0–1,4 s (5 sauts mesurés) | — |
| Défilement de grille (2500×1350) | 7 ms par image (61 ms avant les miniatures au pixel près) | — |
| Migration sur le socle (mode test, 1080p60) | 1re image 133–136 ms (avant : 131–135), Tube ~200 Mo, worker ~98 Mo : aucune régression ; exe +260 Ko | — |
| Avec Y1 à Y4 | 1re image 79–119 ms, Tube 183–200 Mo, worker 98 Mo, lecture 1,0–1,4 s après l'extraction | — |
| Enchaînement (Y1) | vidéo suivante en lecture 0,18 s après la fin | — |
| Direct : retour au direct / recul de 15 min | 0,7 s / 1,4 s | — |

## 7. Intégrations

- **Aujourd'hui** : `Tube.exe <lien youtube>` ouvre la vidéo, dans l'instance déjà lancée s'il y en a une. N'importe quelle appli peut donc déléguer un lien YouTube à Tube.
- **Lecteur universel** : partage le principe de fenêtre vidéo Media Foundation (`vidwin.rs` en est issu). Pas d'appel croisé à ce jour.
- **Vigie** : partage le même `theme.rs`. Pas d'intégration fonctionnelle.
- **Launcher** : aucune intégration codée. Piste : ajouter Tube et relayer les liens YouTube collés.
- Aucune modification des autres projets n'a été faite : il faut l'accord de l'utilisateur.

## 8. Limites connues, dette technique, risques

**Risques côté YouTube :**
- **Conditions d'utilisation** : c'est un client non officiel. Les comptes utilisés via cookies peuvent en théorie être limités (avertissement du wiki yt-dlp).
- **Anti-robot et bridage.**
  - googlevideo bride au hasard certaines requêtes (~100 Ko/s) et plafonne des réponses (~1 Mo). C'est contourné par le parallélisme, les réessais et la qualité adaptative, mais les débits restent irréguliers.
  - Extraire plusieurs vidéos en parallèle donnait des URLs bridées : les extractions sont donc en série.
  - yt-dlp n'a pas de fournisseur de *PO token* (« PO Token Providers: none »). C'est un risque si YouTube l'exige plus largement.
- **Dépendance à yt-dlp et deno** : YouTube casse régulièrement l'extraction. La mise à jour quotidienne corrige généralement en heures ou en jours.
- **Changements d'InnerTube** : l'analyse des cartes (lockupViewModel…) et `CLIENT_VERSION` (`2.20261117.01.00`) peuvent casser l'accueil, la recherche ou les retours « Pas intéressé ».
- **Directs** :
  - le client `mweb` refuse parfois des segments (403) ; Tube refait l'extraction tout seul ;
  - la fenêtre de retour arrière (~35 min) est déduite, pas garantie ;
  - les URLs de segments sont liées à l'IPv4.

**Dépendances et environnement :**
- Python 3 doit être installé.
- Les codecs VP9/AV1 du Store sont nécessaires.
- Le mode musique (process loopback) demande Windows 11 ; sinon il y a un repli sans spectre.

**Coûts et dette technique :**
- **Rendu CPU** : une grande fenêtre en mouvement coûte du processeur (le mode cinéma redessine 4 fois par seconde). L'interface au-dessus de la vidéo passe par des découpes de fenêtre (`SetWindowRgn`).
- `app.rs` et `ui.rs` font plus de 2 000 lignes chacun : un découpage en modules (lecteur, onglets, menus) serait souhaitable.
- Aucun test automatisé : seulement des scénarios scriptés.
- Les écrans « Connexion » et « Paramètres » sont fonctionnels, sans plus.

## 9. Pistes QoL (non implémentées : validation requise)

Elles sont classées par utilité. **(I)** = inédite, **(E)** = existe ailleurs. L'effort va de S (petit) à L (gros).

| # | Piste | Type | Effort | Comment elle reste instantanée |
|---|---|---|---|---|
| 1 | ✅ fait (Y1). **Enchaînement sans attente** : 30 s avant la fin, la vidéo suivante (file ou suggestion) est extraite et ses premiers blocs téléchargés ; le passage est immédiat | I | S | Réutilise le préchargement et les flux réchauffés : tout est prêt avant la fin |
| 2 | ✅ fait (Y2). **Moments forts automatiques** : sur une vidéo sans chapitres, les pics de la courbe « les plus revus » deviennent des repères, et Ctrl+→ saute au prochain moment fort | I | S | La courbe arrive avec l'extraction ; calcul local de quelques microsecondes |
| 3 | **DeArrow** : titres et miniatures corrigés par la communauté (anti-clickbait), avec bascule | E (FreeTube, Grayjay) | M | Requête par préfixe de hachage par lot de cartes, en arrière-plan ; titre d'origine affiché tout de suite, remplacé à l'arrivée ; cache disque |
| 4 | **Filtres de recherche** : durée, date, tri, type, en pastilles sous la barre | E (YouTube, FreeTube) | S | Paramètre `params` d'InnerTube ; résultats en ~300 ms comme une recherche normale |
| 5 | **Recherche dans la transcription** : Ctrl+F sur une vidéo cherche dans les sous-titres et liste les passages ; Entrée y saute | E (YouTube, partiel) | S | Sous-titres déjà en mémoire après le premier affichage ; recherche locale sans réseau |
| 6 | **Navigation clavier façon téléviseur** : flèches dans la grille, aperçu muet sur la carte sélectionnée, Entrée pour lire | I | M | Même aperçu que le survol, mais déclenché par la sélection ; extraction prioritaire de la carte focalisée |
| 7 | **Commentaires** (tri, horodatages cliquables qui marquent aussi la barre de progression) | E (YouTube) / repères sur la barre I | M | Chargés en tâche de fond pendant le démarrage ; affichés à la demande ; pagination à la volée |
| 8 | ✅ fait (Y3). **Arrière-plan intelligent** : fenêtre réduite et vidéo musicale → bascule en audio seul, puis retour à la vidéo à la restauration | I | S | Bascule à la même position ; flux vidéo réchauffé avant le retour |
| 9 | **Badges de nouveautés** : nombre de nouvelles vidéos des abonnements sur l'entrée « Abonnements » | E (Grayjay, YouTube) | S | Interrogation légère toutes les 15 min en arrière-plan ; aucun coût à l'ouverture |
| 10 | **Marathon de chaîne** : lire une chaîne dans l'ordre chronologique en sautant les vidéos vues, avec la file prête | I | M | Liste de la chaîne déjà paginée ; file construite localement ; enchaînement sans attente (piste 1) |
| 11 | ✅ fait (Y4). **Recherche locale instantanée** dans l'historique et les vidéos récentes, pendant la frappe, avant les résultats YouTube | I | S | Index en mémoire (titres, chaînes) ; résultats affichés à la frappe, avant toute requête |
| 12 | **Return YouTube Dislike** : nombre de « je n'aime pas » estimé sous la vidéo | E (FreeTube, extensions) | S | Une petite requête en parallèle de l'extraction ; affichage à l'arrivée sans rien bloquer |
| 13 | **Vitesse automatique selon le débit de parole** : podcasts et conférences accélérés dans les passages lents, sans toucher à la musique | I | L | Analyse du son déjà capturé (mode musique) ; décision locale, changement de vitesse sans recharger |
| 14 | **Import et export d'abonnements et de playlists locales** (sauvegarde, migration depuis FreeTube) | E (FreeTube, Grayjay) | M | Opération locale sur fichier ; aucune attente réseau |
