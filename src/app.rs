//! État de l'application et logique (navigation, lecture, historique, reprise, SponsorBlock).
use crate::api::{Api, Item, Kind, Page, Tracking};
use crate::cookies::Jar;
use crate::player::Player;
use crate::sponsor::Seg;
use crate::store::{Quality, Settings};
use crate::vidwin::VideoWindow;
use crate::worker::Worker;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum View {
    Home,
    Subs,
    Later,
    Playlists,
    History,
    Search(String),
    Playlist(String, String),
    /// page d'une chaîne : (UC…, nom)
    Channel(String, String),
    Watch,
    Settings,
}

impl View {
    /// Clé de flux (requête InnerTube) de la vue.
    pub fn feed_key(&self) -> Option<String> {
        Some(match self {
            View::Home => "FEwhat_to_watch".into(),
            View::Subs => "FEsubscriptions".into(),
            View::Later => "VLWL".into(),
            View::Playlists => "FEplaylist_aggregation".into(),
            View::History => "FEhistory".into(),
            View::Search(q) => format!("search:{q}"),
            View::Playlist(id, _) => format!("VL{id}"),
            View::Channel(id, _) => format!("chan:{id}"),
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Normal,
    Full,
    /// demi-écran : toute la hauteur, la moitié de la largeur (écrans très larges)
    Half,
    Mini,
}

pub enum WinOp {
    Fullscreen(bool),
    Half(bool),
    Mini(bool),
    Opacity(f32),
    Drag,
    Resize(winit::window::ResizeDirection),
    Title(String),
    Focus,
}

/// Onglet : une vue avec son propre historique (et la vidéo qu'il affiche, s'il en a une).
pub struct Tab {
    pub view: View,
    pub back: Vec<View>,
    pub video: Option<Item>,
}

#[derive(Default)]
pub struct Feed {
    pub items: Vec<Item>,
    pub cont: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub loaded: Option<Instant>,
}

pub enum Msg {
    /// Image de fond du mode audio seul (identifiant de la vidéo)
    Frame(String, Option<egui::ColorImage>),
    Feed(String, Result<Page, String>, bool),
    Extracted(String, Result<Value, String>),
    Related(String, Vec<Item>),
    Tracking(String, Tracking),
    Sponsor(String, Vec<Seg>),
    Cookies(Result<String, String>),
    SetupStep(String),
    SetupDone(Result<(), String>),
    Updated(Result<Option<String>, String>),
    Toast(String),
    /// sous-titres chargés : (vidéo, repères (début, fin, texte))
    Subs(String, Vec<(f64, f64, String)>),
    /// suggestions de recherche : (texte tapé, propositions)
    Suggest(String, Vec<String>),
}

/// Minuterie de mise en veille.
#[derive(Clone, Copy, PartialEq)]
pub enum Sleep {
    At(Instant),
    EndOfVideo,
}

#[derive(PartialEq)]
pub enum Setup {
    Running(String),
    Failed(String),
    Done,
}

#[derive(PartialEq, Clone, Copy)]
pub enum Login {
    Need,
    Importing,
    Ok,
    Skipped,
}

/// Piste audio proposée : (code langue, libellé, originale ?)
#[derive(Clone, PartialEq)]
pub struct Lang {
    pub code: String,
    pub label: String,
    pub original: bool,
}

#[derive(Clone)]
pub struct Fmt {
    pub id: String,
    pub url: String,
    pub ext: String,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub tbr: f64,
    pub lang: String,
    pub original: bool,
    pub label: String,
}

pub struct Watch {
    pub item: Item,
    pub info: Option<Value>,
    pub player: Option<Player>,
    pub video_fmt: Option<Fmt>,
    pub audio_fmt: Option<Fmt>,
    pub heights: Vec<u32>,
    pub forced_height: Option<u32>,
    /// pistes audio (vide s'il n'y en a qu'une) et langue choisie à la main
    pub langs: Vec<Lang>,
    pub forced_lang: Option<String>,
    /// qualité plafonnée automatiquement (connexion trop lente), et début de l'attente en cours
    pub auto_cap: Option<u32>,
    pub started_at: Instant,
    /// images affichées au dernier relevé : si elles n'avancent plus alors que le son joue,
    /// la vidéo manque de données (débit insuffisant pour la définition)
    pub frames_at: (u64, Instant),
    /// saut en cours : instant de la demande (mesure du temps de reprise, journal)
    pub seek_at: Option<(Instant, u64, f64)>,
    /// Y1 : vidéo suivante déjà préparée (extraite, flux réchauffés) avant la fin
    pub next_prepared: Option<String>,
    /// Y3 : passée en audio seul automatiquement (fenêtre réduite sur une musique)
    pub auto_audio: bool,
    pub stalled_since: Option<Instant>,
    pub last_abr: Option<Instant>,
    /// lecture du son seul (pas de flux vidéo téléchargé)
    pub audio_only: bool,
    /// sous-titre choisi (index dans info["subs"]) et ses repères
    pub sub_sel: Option<usize>,
    pub cues: Vec<(f64, f64, String)>,
    /// direct : retard voulu sur le bord du direct au (re)démarrage, et repère (instant, temps lecteur)
    /// pris à la première image pour suivre le retard réel (pauses comprises)
    pub live_back: f64,
    /// retard accumulé depuis (pauses et attentes réseau), et dernier relevé
    pub live_lag: f64,
    pub live_tick: Instant,
    pub error: Option<String>,
    pub related: Vec<Item>,
    pub sponsors: Vec<Seg>,
    pub skipped: HashSet<usize>,
    pub tracking: Option<Tracking>,
    pub cpn: String,
    pub pinged_start: bool,
    pub last_ping: (f64, Instant),
    pub watched: f64,
    pub recovered: bool,
    pub desc_open: bool,
    pub opened_at: Instant,
    pub resume_from: Option<f64>,
    pub undo_skip: Option<f64>,
    pub last_save: Instant,
    /// garde-fou : dernière position qui a avancé (lecture bloquée ⇒ réouverture)
    pub progress_at: (f64, Instant),
    /// lecture demandée mais bloquée (mise en mémoire tampon)
    pub stalled: bool,
}

/// Aperçu au survol d'une vignette : petite vidéo muette jouée dans la vignette.
pub struct Preview {
    pub id: String,
    /// déclaré avant la fenêtre : le moteur s'arrête avant que sa fenêtre soit détruite
    pub player: Option<Player>,
    pub win: VideoWindow,
    pub touched: bool,
    pub shown: bool,
}

pub struct App {
    pub ctx: egui::Context,
    pub hwnd: isize,
    pub pal: socle::theme::Palette,
    pub s: Settings,
    pub api: Arc<Api>,
    pub worker: Arc<Worker>,
    pub thumbs: crate::thumbs::Thumbs,
    pub tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub view: View,
    pub back: Vec<View>,
    pub feeds: HashMap<String, Feed>,
    pub search: String,
    /// recherche : état du champ et moteur d'autocomplétion du socle (historique local + suggestions
    /// YouTube comme candidats), dernière saisie envoyée aux suggestions
    pub search_state: socle::ui::AutoField,
    pub ac: socle::autocomplete::Autocomplete,
    pub sugg_q: String,
    /// palette de commandes Ctrl+K (socle)
    pub palette: socle::ui::Palette,
    pub focus_search: bool,
    pub watch: Option<Watch>,
    pub cache: HashMap<String, (Value, Instant)>,
    pub extracting: HashSet<String>,
    pub extracting_prio: HashMap<String, u8>,
    pub hover: Option<(String, Instant)>,
    pub setup: Setup,
    pub login: Login,
    pub login_msg: Option<String>,
    pub reimported: Option<Instant>,
    /// session expirée et copie silencieuse impossible : bandeau « Reconnecter »
    pub reconnect: bool,
    pub toast: Option<(String, Instant)>,
    pub mode: Mode,
    pub vidwin: Option<VideoWindow>,
    pub video_px: Option<(i32, i32, i32, i32)>,
    pub video_cuts: Vec<(i32, i32, i32, i32)>,
    pub overlay_until: Instant,
    pub ops: Vec<WinOp>,
    pub hk_failed: Vec<String>,
    hk_rx: Option<Receiver<Vec<String>>>,
    pub queue: Vec<Item>,
    pub qpos: usize,
    pub stats: bool,
    pub help: bool,
    pub ytdlp_version: Option<String>,
    pub occluded: bool,
    /// fenêtre réduite (Y3), et depuis quand
    pub minimized: Option<Instant>,
    last_disk_save: Instant,
    pub scroll_to_top: bool,
    pub menu: Option<(crate::ui::Menu, egui::Rect)>,
    pub output_dirty: bool,
    /// onglets ; ceux de l'onglet actif (vue, historique) vivent dans `view` / `back`
    pub tabs: Vec<Tab>,
    pub tab: usize,
    /// onglet qui a lancé la lecture en cours
    pub play_tab: usize,
    auto_next: bool,
    /// image figée affichée (grisée) pendant une pause ou une attente réseau : (vidéo, texture)
    pub still: Option<(String, egui::TextureHandle, egui::Vec2, bool)>,
    /// Mode audio seul : une vraie image de la vidéo en fond (identifiant de la vidéo).
    pub audio_bg: Option<(String, egui::TextureHandle)>,
    audio_bg_asked: String,
    /// pas de capture avant cet instant (le temps que l'image d'un saut s'affiche)
    pub still_after: Instant,
    /// test : défilement automatique de la grille (points par image)
    pub dbg_scroll: (f32, f32),
    pub preview: Option<Preview>,
    /// mode cinéma : couleurs lissées des deux bords de l'image (bandes), instant du dernier relevé
    pub ambient: Option<(Vec<[f32; 3]>, Vec<[f32; 3]>, Instant)>,
    /// mode audio seul : capture du son de Tube pour la visualisation, et hauteurs lissées des barres
    pub viz: Option<crate::viz::Viz>,
    pub viz_bars: Vec<f32>,
    pub viz_failed: bool,
    /// panneau multimédia de Windows (touches multimédia, titre et miniature)
    pub smtc: Option<crate::smtc::Smtc>,
    /// logo de la barre d'outils (même dessin que l'icône de l'application)
    pub logo: Option<egui::TextureHandle>,
    /// lecture en boucle de la vidéo courante
    pub loop_on: bool,
    /// minuterie : pause à un instant donné ou à la fin de la vidéo
    pub sleep: Option<Sleep>,
    /// onglets fermés (Ctrl+Maj+T pour les rouvrir)
    pub closed_tabs: Vec<(View, Option<Item>)>,
    /// vidéos à extraire d'avance (premières cartes de la page affichée)
    prefetch_q: std::collections::VecDeque<String>,
    /// dernière vidéo dont le flux a été réchauffé au survol
    warmed: Option<String>,
}

fn cookies_age() -> u64 {
    std::fs::metadata(crate::store::cookies_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}

/// Plus haute définition (≤ `cap`) dont le débit tient dans `speed` (octets/s), avec marge.
/// `below` : strictement sous la définition actuellement choisie pour `cap`.
pub fn fit_cap(info: &Value, cap: u32, lang: Option<&str>, speed: f64, below: bool) -> Option<u32> {
    let (cur, a, heights, _) = pick(info, cap, lang);
    let audio = a.map(|a| a.tbr * 125.0).unwrap_or(16_000.0);
    let cur_h = cur.as_ref().map(|v| v.height).unwrap_or(cap);
    // le débit suffit pour la définition actuelle : l'attente vient d'ailleurs, on ne baisse pas
    let cur_need = cur.as_ref().map(|v| v.tbr * 125.0).unwrap_or(0.0) + audio;
    if below && speed > 0.0 && cur_need * 1.3 < speed {
        return None;
    }
    for h in heights.into_iter().filter(|h| *h <= cap && (!below || *h < cur_h)) {
        let (v, _, _, _) = pick(info, h, lang);
        let need = v.map(|v| v.tbr * 125.0).unwrap_or(0.0) + audio;
        if need * 1.3 < speed {
            return if h >= cur_h && !below { None } else { Some(h) };
        }
    }
    Some(if below { 360 } else { cur_h.min(360) })
}

/// Sous-titres YouTube au format JSON3 → (début, fin, texte). Les ajouts des sous-titres
/// automatiques (« aAppend ») sont ignorés : chaque événement porte déjà sa phrase.
pub fn parse_json3(v: &Value) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    for e in v["events"].as_array().into_iter().flatten() {
        if e["aAppend"].as_u64() == Some(1) {
            continue;
        }
        let Some(segs) = e["segs"].as_array() else { continue };
        let text: String = segs.iter().filter_map(|s| s["utf8"].as_str()).collect();
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let a = e["tStartMs"].as_f64().unwrap_or(0.0) / 1000.0;
        let d = e["dDurationMs"].as_f64().unwrap_or(2000.0) / 1000.0;
        out.push((a, a + d, text));
    }
    out
}

trait OrDefaultTo {
    fn or_default_to(self, other: &str) -> String;
}
impl OrDefaultTo for String {
    fn or_default_to(self, other: &str) -> String {
        if self.is_empty() { other.to_string() } else { self }
    }
}

/// Y2 « moments forts » : pics de la courbe « les plus revus » (au plus 8, espacés d'au moins
/// 4 % de la durée, au-dessus de 55 % du maximum). Calcul local instantané (~100 points).
pub fn highlights(w: &Watch) -> Vec<f64> {
    let Some(hm) = w.info.as_ref().and_then(|i| i["heatmap"].as_array()) else { return Vec::new() };
    let pts: Vec<(f64, f64)> = hm.iter().filter_map(|h| Some(((h["start"].as_f64()? + h["end"].as_f64()?) / 2.0, h["value"].as_f64()?))).collect();
    if pts.len() < 5 {
        return Vec::new();
    }
    let dur = pts.last().map(|p| p.0).unwrap_or(0.0).max(1.0);
    // le tout début est souvent haut (tout le monde le voit) : on l'ignore
    let skip = (dur * 0.03).max(10.0);
    let mut peaks: Vec<(f64, f64)> = (1..pts.len() - 1).filter(|&i| pts[i].1 >= pts[i - 1].1 && pts[i].1 >= pts[i + 1].1 && pts[i].0 > skip).map(|i| pts[i]).collect();
    let max = peaks.iter().map(|p| p.1).fold(0.0, f64::max);
    peaks.retain(|p| p.1 >= max * 0.55);
    peaks.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out: Vec<f64> = Vec::new();
    for (t, _) in peaks {
        if out.len() >= 8 {
            break;
        }
        if out.iter().all(|o| (o - t).abs() > dur * 0.04) {
            out.push(t);
        }
    }
    out.sort_by(|a, b| a.total_cmp(b));
    out
}

/// Titre du chapitre en cours à l'instant `t`.
pub fn chapter_name(w: Option<&Watch>, t: f64) -> Option<String> {
    let ch = w?.info.as_ref()?["chapters"].as_array()?.clone();
    ch.iter().rev().find(|c| c["start"].as_f64().is_some_and(|s| s <= t)).and_then(|c| c["title"].as_str().map(|s| s.to_string()))
}

pub fn fmt_time(t: f64) -> String {
    let t = t.max(0.0) as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

/// Choix des flux : VP9/WebM de préférence (sauts rapides), AV1 seulement s'il apporte nettement plus,
/// SDR uniquement ; son Opus (WebM), sinon AAC.
/// `lang` : piste audio voulue (code langue) ; par défaut, la piste d'origine de la vidéo
/// (YouTube propose souvent des doublages automatiques, parfois servis en premier).
pub fn pick(info: &Value, cap: u32, lang: Option<&str>) -> (Option<Fmt>, Option<Fmt>, Vec<u32>, Vec<Lang>) {
    let mut vids = Vec::new();
    let mut auds = Vec::new();
    for f in info["formats"].as_array().into_iter().flatten() {
        let g = |k: &str| f[k].as_str().unwrap_or("").to_string();
        let fmt = Fmt {
            id: g("format_id"),
            url: g("url"),
            ext: g("ext"),
            height: f["height"].as_u64().unwrap_or(0) as u32,
            fps: f["fps"].as_f64().unwrap_or(0.0) as u32,
            codec: if g("vcodec") != "none" && !g("vcodec").is_empty() { g("vcodec") } else { g("acodec") },
            tbr: f["tbr"].as_f64().unwrap_or(0.0),
            lang: g("language"),
            original: f["language_preference"].as_i64().unwrap_or(0) >= 10 || g("format_note").to_lowercase().contains("original"),
            label: {
                // « French (FR) original (default), medium » → « French (FR) »
                let n = g("format_note");
                let n = n.split(',').next().unwrap_or("").replace("original", "").replace("(default)", "");
                n.trim().to_string()
            },
        };
        if fmt.url.is_empty() || fmt.id.contains("drc") {
            continue;
        }
        let (v, a) = (g("vcodec"), g("acodec"));
        let sdr = g("dynamic_range").is_empty() || g("dynamic_range") == "SDR";
        if v != "none" && !v.is_empty() && (a == "none" || a.is_empty()) && sdr {
            vids.push(fmt);
        } else if (v == "none" || v.is_empty()) && a != "none" && !a.is_empty() {
            let note = g("format_note").to_lowercase();
            // pistes doublées : garder l'originale
            if note.contains("dubbed") || (note.contains("descriptive")) {
                continue;
            }
            auds.push(fmt);
        }
    }
    let rank = |c: &str| if c.starts_with("vp9") || c.starts_with("vp09.00") { 3 } else if c.starts_with("av01") { 2 } else { 1 };
    let best = |list: &Vec<&Fmt>| list.iter().max_by(|a, b| (a.height, a.fps).cmp(&(b.height, b.fps)).then(a.tbr.total_cmp(&b.tbr))).map(|f| (*f).clone());
    let under: Vec<&Fmt> = vids.iter().filter(|f| f.height <= cap).collect();
    let vp9: Vec<&Fmt> = under.iter().copied().filter(|f| rank(&f.codec) == 3).collect();
    let other: Vec<&Fmt> = under.iter().copied().filter(|f| rank(&f.codec) < 3).collect();
    let bv = best(&vp9);
    let bo = best(&other);
    let video = match (&bv, &bo) {
        (Some(v), Some(o)) => {
            if v.height >= o.height.min(1440) { bv.clone() } else { bo.clone() }
        }
        (Some(_), None) => bv.clone(),
        _ => bo.clone(),
    };
    // langues disponibles (une entrée par code), l'originale en premier
    let mut langs: Vec<Lang> = Vec::new();
    for a in &auds {
        if !langs.iter().any(|l| l.code == a.lang) {
            langs.push(Lang { code: a.lang.clone(), label: if a.label.is_empty() { a.lang.clone() } else { a.label.clone() }, original: a.original });
        } else if a.original {
            if let Some(l) = langs.iter_mut().find(|l| l.code == a.lang) {
                l.original = true;
            }
        }
    }
    langs.sort_by(|a, b| b.original.cmp(&a.original).then(a.label.cmp(&b.label)));
    let want = lang.filter(|l| langs.iter().any(|x| x.code == *l)).map(|s| s.to_string()).or_else(|| langs.first().map(|l| l.code.clone()));
    let audio = auds
        .iter()
        .filter(|a| want.as_ref().is_none_or(|w| a.lang == *w))
        .max_by(|a, b| {
            let ra = a.codec.starts_with("opus") as u8;
            let rb = b.codec.starts_with("opus") as u8;
            ra.cmp(&rb).then(a.tbr.total_cmp(&b.tbr))
        })
        .cloned();
    if langs.len() < 2 {
        langs.clear();
    }
    let mut heights: Vec<u32> = vids.iter().map(|f| f.height).filter(|h| *h >= 144).collect();
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();
    (video, audio, heights, langs)
}

impl App {
    pub fn new(ctx: &egui::Context, s: Settings, hwnd: isize, wake_hk: impl Fn(crate::hotkeys::Hk) + Send + Sync + 'static) -> App {
        // gras (titres des cartes) et emoji (fréquents dans les titres YouTube : sans eux, des carrés vides)
        socle::theme::install_fonts_with(ctx, socle::theme::FontOptions { bold: true, emoji: true, symbols: true });
        let pal = socle::theme::palette(s.dark);
        socle::theme::apply(ctx, &pal);
        // dossiers d'import laissés par un arrêt brutal (copies brutes des cookies du navigateur)
        std::thread::spawn(|| {
            for e in std::fs::read_dir(crate::store::dir()).into_iter().flatten().flatten() {
                if e.file_name().to_string_lossy().starts_with("import") && e.path().is_dir() {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        });
        let jar = Arc::new(Mutex::new(Jar::load(&crate::store::cookies_path())));
        let logged = jar.lock().unwrap().logged_in();
        let api = Arc::new(Api::new(jar));
        let c2 = ctx.clone();
        let worker = Worker::new(Arc::new(move || c2.request_repaint()));
        let thumbs = crate::thumbs::Thumbs::new(api.clone(), ctx.clone());
        let (tx, rx) = channel();
        let vidwin = VideoWindow::new(hwnd);
        // test : pas de crochet clavier global (il intercepterait les Win+Alt de l'utilisateur)
        let hk_rx = if crate::store::test_mode() { None } else { Some(crate::hotkeys::start(wake_hk)) };
        let mut app = App {
            ctx: ctx.clone(),
            hwnd,
            pal,
            s,
            api,
            worker,
            thumbs,
            tx,
            rx,
            view: View::Home,
            back: Vec::new(),
            feeds: HashMap::new(),
            search: String::new(),
            focus_search: false,
            watch: None,
            cache: HashMap::new(),
            extracting: HashSet::new(),
            extracting_prio: HashMap::new(),
            hover: None,
            setup: if crate::worker::ready() { Setup::Done } else { Setup::Running("Préparation…".into()) },
            login: if logged { Login::Ok } else { Login::Need },
            login_msg: None,
            reimported: None,
            reconnect: false,
            toast: None,
            mode: Mode::Normal,
            vidwin,
            video_px: None,
            video_cuts: Vec::new(),
            overlay_until: Instant::now(),
            ops: Vec::new(),
            hk_failed: Vec::new(),
            hk_rx,
            queue: Vec::new(),
            qpos: 0,
            stats: false,
            help: false,
            ytdlp_version: None,
            occluded: false,
            minimized: None,
            last_disk_save: Instant::now(),
            scroll_to_top: false,
            menu: None,
            output_dirty: false,
            tabs: vec![Tab { view: View::Home, back: Vec::new(), video: None }],
            // (onglets de la session précédente : restaurés par restore_tabs)
            tab: 0,
            play_tab: 0,
            auto_next: false,
            still: None,
            audio_bg: None,
            audio_bg_asked: String::new(),
            still_after: Instant::now(),
            dbg_scroll: (0.0, 0.0),
            preview: None,
            ambient: None,
            viz: None,
            viz_bars: Vec::new(),
            viz_failed: false,
            smtc: None,
            logo: None,
            loop_on: false,
            sleep: None,
            closed_tabs: Vec::new(),
            prefetch_q: Default::default(),
            search_state: Default::default(),
            ac: Default::default(),
            sugg_q: String::new(),
            palette: Default::default(),
            warmed: None,
        };
        if app.setup != Setup::Done {
            let tx = app.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let c3 = ctx.clone();
                let tx2 = tx.clone();
                let r = crate::worker::setup(&move |s: &str| {
                    let _ = tx2.send(Msg::SetupStep(s.to_string()));
                    c3.request_repaint();
                });
                let _ = tx.send(Msg::SetupDone(r));
                ctx.request_repaint();
            });
        } else {
            app.after_setup();
        }
        // (jamais en test : l'import relance le service réseau du navigateur de l'utilisateur)
        if app.login == Login::Ok && app.setup == Setup::Done && cookies_age() > 30 * 60 && !crate::store::test_mode() {
            // le navigateur fait tourner les jetons de session : on repart de sa copie fraîche
            // copie silencieuse (aucun arrêt) ; si le fichier est verrouillé : bandeau « Reconnecter »
            if let Some((_, p)) = crate::cookies::browser_profiles().into_iter().next() {
                app.reimported = Some(Instant::now());
                app.import_cookies(p, false);
                app.login = Login::Ok;
                return app;
            }
        }
        if app.login != Login::Need {
            app.load_feed(&View::Home, false);
        }
        app
    }

    fn after_setup(&mut self) {
        // préchauffe le worker (charge yt-dlp) et vérifie la version
        let rx = self.worker.send(serde_json::json!({"cmd": "version"}));
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            if let Ok(Ok(v)) = rx.recv() {
                let _ = tx.send(Msg::Toast(format!("__version:{}", v["version"].as_str().unwrap_or("?"))));
                ctx.request_repaint();
            }
        });
        // mise à jour quotidienne de yt-dlp
        if crate::store::now().saturating_sub(self.s.last_update_check) > 20 * 3600 {
            self.s.last_update_check = crate::store::now();
            self.check_update();
        }
    }

    pub fn check_update(&mut self) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updated(crate::worker::update()));
            ctx.request_repaint();
        });
    }

    /// Ajoute (ou remet après une mise à jour d'Opera) le drapeau qui laisse ses cookies lisibles.
    /// Sauvegardes à côté du projet en développement, sinon dans le dossier de Tube.
    pub fn opera_flag(&mut self) {
        let dev = std::env::current_exe().ok().and_then(|e| e.parent()?.parent().map(|p| p.to_path_buf())).filter(|p| p.join("Cargo.toml").exists());
        let backup = dev.unwrap_or_else(crate::store::real_dir).join("sauvegarde-raccourcis-opera");
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        // raccourcis et registre : dans un fil (accès disque et COM)
        std::thread::spawn(move || {
            let r = crate::opera::apply(&backup);
            let added = r.iter().filter(|l| l.contains("ajouté")).count();
            let err = r.iter().any(|l| l.contains("erreur") || l.starts_with("clé Run"));
            let _ = tx.send(Msg::Toast(if err {
                "Opera : une modification a échoué (voir tube.log)".to_string()
            } else if added > 0 {
                format!("Opera : verrou des cookies réduit sur {added} lancement(s), effet au prochain démarrage d'Opera")
            } else {
                "Opera : réglage déjà en place".into()
            }));
            ctx.request_repaint();
        });
    }

    pub fn toast(&mut self, t: impl Into<String>) {
        self.toast = Some((t.into(), Instant::now()));
    }

    // ---------- Connexion ----------
    /// Import des cookies. `allow_kill` : seulement sur action de l'utilisateur (bouton) ; sinon
    /// copie silencieuse, possible uniquement si le fichier du navigateur n'est pas verrouillé.
    pub fn import_cookies(&mut self, profile: std::path::PathBuf, allow_kill: bool) {
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
        static BUSY: AtomicBool = AtomicBool::new(false);
        static N: AtomicU32 = AtomicU32::new(0);
        // un seul import à la fois (bouton cliqué deux fois, réimport auto en même temps…)
        if BUSY.swap(true, Ordering::SeqCst) {
            return;
        }
        crate::store::log(&format!("import des cookies depuis {}{}", profile.display(), if allow_kill { "" } else { " (sans rien arrêter)" }));
        if self.login != Login::Ok {
            self.login = Login::Importing;
        }
        self.login_msg = None;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let worker = self.worker.clone();
        // nom yt-dlp du navigateur (déchiffrement propre à chacun) ; Opera GX = « opera »
        let p = profile.to_string_lossy().to_lowercase();
        let browser = if p.contains(r"\google\chrome") { "chrome" } else if p.contains(r"\microsoft\edge") { "edge" } else { "opera" };
        std::thread::spawn(move || {
            let tmp = crate::store::dir().join(format!("import-{}", N.fetch_add(1, Ordering::Relaxed)));
            let r = crate::cookies::copy_profile(&profile, &tmp, allow_kill).and_then(|note| {
                let rx = worker.send(serde_json::json!({"cmd": "import", "browser": browser, "src": tmp.to_string_lossy(), "out": crate::store::cookies_path().to_string_lossy()}));
                let res = rx.recv().map_err(|e| e.to_string()).and_then(|r| r);
                let _ = std::fs::remove_dir_all(&tmp);
                res.map(|v| format!("{} cookies importés{}", v["count"], if note.is_empty() { String::new() } else { format!(" ({note})") }))
            });
            let _ = std::fs::remove_dir_all(&tmp);
            BUSY.store(false, Ordering::SeqCst);
            let _ = tx.send(Msg::Cookies(r));
            ctx.request_repaint();
        });
    }

    // ---------- Flux ----------
    pub fn go(&mut self, v: View) {
        if self.view != v {
            let prev = std::mem::replace(&mut self.view, v.clone());
            if prev != View::Watch || v != View::Watch {
                self.back.push(prev);
                if self.back.len() > 50 {
                    self.back.remove(0);
                }
            }
        }
        if v == View::Watch {
            if let Some(w) = &self.watch {
                self.tabs[self.tab].video = Some(w.item.clone());
                self.play_tab = self.tab;
            }
        }
        self.ensure_feed(&v);
    }

    fn ensure_feed(&mut self, v: &View) {
        // une page déjà chargée est gardée telle quelle (et à la même position de défilement) ;
        // seulement rafraîchie si elle date de plus de 30 min (ou à la demande : F5, logo)
        let stale = v.feed_key().and_then(|k| self.feeds.get(&k)).is_none_or(|f| f.error.is_some() || f.loaded.is_some_and(|t| t.elapsed() > Duration::from_secs(1800)));
        if v.feed_key().is_some() && stale {
            self.load_feed(v, false);
        }
    }

    // ---------- Onglets ----------
    /// Ouvre un onglet (en arrière-plan sauf si `activate`).
    pub fn new_tab(&mut self, view: View, video: Option<Item>, activate: bool) {
        if let Some(v) = &video {
            let id = v.id.clone();
            self.extract(&id);
        } else {
            self.ensure_feed(&view);
        }
        self.tabs.push(Tab { view, back: Vec::new(), video });
        if activate {
            self.switch_tab(self.tabs.len() - 1);
        } else {
            self.toast("Ouvert dans un nouvel onglet");
        }
    }

    /// Ouvre un élément (vidéo ou playlist) dans un nouvel onglet en arrière-plan.
    pub fn open_in_tab(&mut self, it: &Item) {
        if it.kind == Kind::Playlist {
            self.new_tab(View::Playlist(it.id.clone(), it.title.clone()), None, false);
        } else {
            self.new_tab(View::Watch, Some(it.clone()), false);
        }
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i == self.tab || i >= self.tabs.len() {
            return;
        }
        let t = &mut self.tabs[self.tab];
        t.view = self.view.clone();
        t.back = std::mem::take(&mut self.back);
        self.load_tab(i);
    }

    fn load_tab(&mut self, i: usize) {
        self.tab = i;
        self.view = self.tabs[i].view.clone();
        self.back = std::mem::take(&mut self.tabs[i].back);
        self.menu = None;
        if self.view == View::Watch {
            // une seule lecture à la fois : afficher l'onglet relance sa vidéo (reprise à la même position)
            if let Some(v) = self.tabs[i].video.clone() {
                self.play_tab = i;
                if self.watch.as_ref().is_none_or(|w| w.item.id != v.id) {
                    self.open(&v, None);
                }
            }
        } else {
            let v = self.view.clone();
            self.ensure_feed(&v);
        }
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let view = if i == self.tab { self.view.clone() } else { self.tabs[i].view.clone() };
        // fermer l'onglet de la vidéo en cours l'arrête
        let playing_here = self.watch.as_ref().is_some_and(|w| self.tabs[i].video.as_ref().is_some_and(|v| v.id == w.item.id));
        if view == View::Watch && i == self.play_tab && playing_here {
            self.close_watch();
            if self.mode != Mode::Normal {
                self.ops.push(WinOp::Mini(false));
                self.ops.push(WinOp::Half(false));
                self.ops.push(WinOp::Fullscreen(false));
            }
        }
        if self.tabs.len() == 1 {
            self.tabs[0].video = None;
            self.back.clear();
            self.view = View::Home;
            self.ensure_feed(&View::Home);
            return;
        }
        let t = &self.tabs[i];
        self.closed_tabs.push((view.clone(), t.video.clone()));
        if self.closed_tabs.len() > 20 {
            self.closed_tabs.remove(0);
        }
        self.tabs.remove(i);
        if self.play_tab > i {
            self.play_tab -= 1;
        }
        if i < self.tab {
            self.tab -= 1;
        } else if i == self.tab {
            self.load_tab(i.min(self.tabs.len() - 1));
        }
    }

    /// Onglets de la session précédente (sans relancer de lecture).
    pub fn restore_tabs(&mut self) {
        self.ac = socle::autocomplete::Autocomplete::new(self.s.search_history.clone());
        self.load_disk_cache();
        let saved = std::mem::take(&mut self.s.tabs);
        let tabs: Vec<Tab> = saved
            .into_iter()
            .filter(|(v, it)| *v != View::Watch || it.is_some())
            .map(|(view, video)| Tab { view, back: Vec::new(), video })
            .collect();
        if tabs.is_empty() {
            return;
        }
        self.tabs = tabs;
        self.tab = self.s.tab.min(self.tabs.len() - 1);
        self.view = self.tabs[self.tab].view.clone();
        // un onglet vidéo actif ne relance pas la lecture au démarrage : on montre l'accueil à la place
        if self.view == View::Watch {
            self.view = View::Home;
            self.tabs[self.tab].video = None;
        }
        let v = self.view.clone();
        self.ensure_feed(&v);
    }

    /// Extractions encore valables enregistrées au lancement précédent : remises dans le cache
    /// (avec leur âge réel), donc pas de nouvelle extraction pour ces vidéos.
    pub fn load_disk_cache(&mut self) {
        let now = crate::store::now();
        for (id, info) in crate::store::load_extracts() {
            let left = crate::store::extract_expiry(&info).unwrap_or(now).saturating_sub(now + 1800).min(3 * 3600);
            let born = Instant::now().checked_sub(Duration::from_secs(3 * 3600 - left)).unwrap_or_else(Instant::now);
            self.cache.entry(id).or_insert((info, born));
        }
    }


    /// Copie les onglets (et l'historique de recherche) dans les réglages (sauvegardés avec eux).
    pub fn store_tabs(&mut self) {
        self.s.search_history = self.ac.history.clone();
        let mut out = Vec::new();
        for i in 0..self.tabs.len() {
            let view = if i == self.tab { self.view.clone() } else { self.tabs[i].view.clone() };
            if view == View::Settings {
                continue;
            }
            let video = if view == View::Watch { self.tabs[i].video.clone() } else { None };
            out.push((view, video));
        }
        self.s.tabs = out;
        self.s.tab = self.tab;
    }

    /// Rouvre le dernier onglet fermé (Ctrl+Maj+T).
    pub fn reopen_tab(&mut self) {
        if let Some((v, video)) = self.closed_tabs.pop() {
            self.new_tab(v, video, true);
        }
    }

    /// Ctrl+C : lien de la vidéo à l'instant courant.
    pub fn copy_time_link(&mut self) {
        let Some(w) = &self.watch else { return };
        let t = w.player.as_ref().map(|p| p.time()).unwrap_or(0.0) as u64;
        let url = if t > 0 { format!("https://youtu.be/{}?t={t}", w.item.id) } else { format!("https://youtu.be/{}", w.item.id) };
        self.ctx.copy_text(url);
        self.toast(format!("Lien copié (à {})", fmt_time(t as f64)));
    }

    /// , / . : image précédente / suivante (met en pause).
    pub fn frame_step(&mut self, dir: f64) {
        let fps = self.watch.as_ref().and_then(|w| w.video_fmt.as_ref()).map(|f| f.fps).filter(|f| *f > 0).unwrap_or(30) as f64;
        if let Some(p) = self.player() {
            if p.wants_play() {
                p.toggle();
            }
            let t = p.time() + dir / fps;
            p.seek(t.max(0.0));
        }
        self.unfreeze();
    }

    pub fn tab_title(&self, i: usize) -> String {
        let view = if i == self.tab { &self.view } else { &self.tabs[i].view };
        match view {
            View::Home => "Accueil".into(),
            View::Subs => "Abonnements".into(),
            View::Later => "À regarder plus tard".into(),
            View::Playlists => "Playlists".into(),
            View::History => "Historique".into(),
            View::Search(q) => format!("« {q} »"),
            View::Playlist(_, t) => t.clone(),
            View::Channel(_, n) => n.clone(),
            View::Settings => "Réglages".into(),
            View::Watch => {
                let v = self.tabs[i].video.as_ref();
                let live = self.watch.as_ref().filter(|w| v.is_some_and(|v| v.id == w.item.id)).map(|w| w.item.title.clone());
                live.or_else(|| v.map(|v| v.title.clone())).filter(|t| !t.is_empty()).unwrap_or_else(|| "Vidéo".into())
            }
        }
    }

    // ---------- File d'attente / « À regarder plus tard » ----------
    /// Ajoute à la file : juste après la vidéo en cours (`next`) ou à la fin.
    pub fn enqueue(&mut self, it: &Item, next: bool) {
        if self.queue.is_empty() {
            if let Some(w) = &self.watch {
                self.queue = vec![w.item.clone()];
                self.qpos = 0;
            }
        }
        if self.queue.is_empty() {
            self.open(it, None);
            return;
        }
        self.queue.retain(|x| x.id != it.id);
        self.qpos = self.qpos.min(self.queue.len() - 1);
        if next {
            self.queue.insert(self.qpos + 1, it.clone());
        } else {
            self.queue.push(it.clone());
        }
        let id = it.id.clone();
        self.extract(&id);
        self.toast(if next { "Lue juste après la vidéo en cours" } else { "Ajoutée à la file d'attente" });
    }

    /// Argument de ligne de commande ou d'une 2ᵉ instance : lien YouTube → lecture, sinon
    /// recherche (intégration « Guide » d'Ultimate : `Tube.exe "nom du jeu guide"`).
    pub fn open_arg(&mut self, arg: &str) {
        let a = arg.trim();
        // commandes d'entretien (transmises à l'instance en cours par `Tube.exe --…`)
        if a == "--opera-drapeau" {
            return self.opera_flag();
        }
        if let Some(id) = crate::ui::video_id_from(a) {
            let it = Item { kind: Kind::Video, id, ..Default::default() };
            self.open(&it, None);
        } else if !a.is_empty() {
            self.search = a.to_string();
            self.go(View::Search(a.to_string()));
        }
    }

    /// Ouvre la page d'une chaîne (dans l'onglet courant, ou un nouvel onglet).
    pub fn open_channel(&mut self, id: &str, name: &str, new_tab: bool) {
        if id.is_empty() {
            return;
        }
        let v = View::Channel(id.to_string(), name.to_string());
        if new_tab {
            self.new_tab(v, None, false);
        } else {
            self.go(v);
        }
    }

    /// « Pas intéressé » (`channel` = faux) ou « Ne pas recommander la chaîne » : retour envoyé à YouTube
    /// et carte(s) retirée(s) de l'accueil.
    pub fn not_interested(&mut self, it: &Item, channel: bool) {
        let tok = if channel { it.no_channel.clone() } else { it.not_interested.clone() };
        if tok.is_empty() {
            return;
        }
        for f in self.feeds.values_mut() {
            f.items.retain(|x| if channel && !it.channel_id.is_empty() { x.channel_id != it.channel_id } else { x.id != it.id });
        }
        let api = self.api.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let m = match api.send_feedback(&tok) {
                Ok(()) => if channel { "Cette chaîne ne vous sera plus recommandée".to_string() } else { "Vidéo masquée · YouTube en tiendra compte".to_string() },
                Err(e) => format!("Retour non envoyé : {e}"),
            };
            let _ = tx.send(Msg::Toast(m));
            ctx.request_repaint();
        });
    }

    pub fn add_watch_later(&mut self, it: &Item) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let id = it.id.clone();
        self.feeds.remove("VLWL");
        std::thread::spawn(move || {
            let m = match api.watch_later(&id) {
                Ok(()) => "Ajoutée à « À regarder plus tard »".to_string(),
                Err(e) => format!("Échec de l'ajout : {e}"),
            };
            let _ = tx.send(Msg::Toast(m));
            ctx.request_repaint();
        });
    }

    pub fn go_back(&mut self) {
        if matches!(self.mode, Mode::Full | Mode::Half) {
            self.leave_big();
            return;
        }
        if let Some(v) = self.back.pop() {
            self.view = v;
        }
    }

    pub fn load_feed(&mut self, v: &View, more: bool) {
        let Some(key) = v.feed_key() else { return };
        let f = self.feeds.entry(key.clone()).or_default();
        if f.loading {
            return;
        }
        let token = if more {
            match &f.cont {
                Some(t) => Some(t.clone()),
                None => return,
            }
        } else {
            None
        };
        f.loading = true;
        f.error = None;
        let api = self.api.clone();
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = match (&token, key.strip_prefix("search:")) {
                (Some(t), Some(_)) => api.search_more(t),
                (Some(t), None) => api.browse_more(t),
                (None, Some(q)) => api.search(q),
                (None, None) => match key.strip_prefix("chan:") {
                    Some(id) => api.channel(id),
                    None => api.browse(&key),
                },
            };
            let _ = tx.send(Msg::Feed(key, r, token.is_some()));
            ctx.request_repaint();
        });
    }

    pub fn refresh(&mut self) {
        self.scroll_to_top = true;
        let v = self.view.clone();
        if let Some(k) = v.feed_key() {
            self.feeds.remove(&k);
            self.load_feed(&v, false);
        }
    }

    // ---------- Extraction / préchargement ----------
    pub fn cached(&self, id: &str) -> Option<Value> {
        self.cache.get(id).filter(|c| c.1.elapsed() < Duration::from_secs(3 * 3600)).map(|c| c.0.clone())
    }

    pub fn extract(&mut self, id: &str) {
        self.extract_prio(id, 0);
    }

    /// `prio` : 0 = clic, 1 = survol, 2 = préchargement. Une demande urgente pour une vidéo déjà
    /// en file avec une priorité plus basse est renvoyée en tête (le worker réutilise le résultat).
    pub fn extract_prio(&mut self, id: &str, prio: u8) {
        if self.cached(id).is_some() || self.setup != Setup::Done {
            return;
        }
        if let Some(p) = self.extracting_prio.get(id) {
            if *p <= prio {
                return;
            }
        }
        self.extracting.insert(id.to_string());
        self.extracting_prio.insert(id.to_string(), prio);
        let rx = self.worker.extract(id, prio);
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            let r = rx.recv().map_err(|e| e.to_string()).and_then(|r| r);
            let _ = tx.send(Msg::Extracted(id, r));
            ctx.request_repaint();
        });
    }

    /// Survol d'une miniature : on prépare la vidéo pour un démarrage instantané au clic.
    pub fn hovering(&mut self, id: Option<&str>) {
        match (id, &self.hover) {
            (Some(i), Some((h, t))) if h == i => {
                let i = i.to_string();
                let held = t.elapsed();
                if held > Duration::from_millis(200) {
                    self.extract_prio(&i, 1);
                }
                // survol prolongé : on réchauffe aussi le flux (premiers blocs vidéo et son téléchargés)
                if held > Duration::from_millis(500) && self.warmed.as_deref() != Some(&i) {
                    if let Some(info) = self.cached(&i) {
                        let (v, a, _, _) = pick(&info, self.s.quality.cap(), None);
                        if info["is_live"].as_bool() != Some(true) {
                            crate::http::prewarm(v.as_ref().map(|f| f.url.as_str()).unwrap_or(""));
                            crate::http::prewarm(a.as_ref().map(|f| f.url.as_str()).unwrap_or(""));
                        }
                        self.warmed = Some(i);
                    }
                }
            }
            (Some(i), _) => {
                self.hover = Some((i.to_string(), Instant::now()));
                self.ctx.request_repaint_after(Duration::from_millis(400));
            }
            (None, _) => self.hover = None,
        }
    }

    // ---------- Lecture ----------
    pub fn open(&mut self, item: &Item, queue: Option<(Vec<Item>, usize)>) {
        let mut item = item.clone();
        if item.thumb.is_empty() && item.kind == Kind::Video {
            item.thumb = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", item.id);
        }
        let item = &item;
        if item.kind == Kind::Playlist {
            self.go(View::Playlist(item.id.clone(), item.title.clone()));
            return;
        }
        if !std::mem::take(&mut self.auto_next) {
            self.play_tab = self.tab;
        }
        // historique local (carte « Reprendre » de l'accueil)
        if item.kind == Kind::Video && !item.title.is_empty() {
            self.s.recent.retain(|x| x.id != item.id);
            self.s.recent.insert(0, item.clone());
            self.s.recent.truncate(40);
        }
        if self.play_tab < self.tabs.len() {
            self.tabs[self.play_tab].video = Some(item.clone());
        }
        if let Some((q, i)) = queue {
            self.queue = q;
            self.qpos = i;
        } else if !self.queue.iter().any(|x| x.id == item.id) {
            self.queue.clear();
        }
        if self.watch.as_ref().is_some_and(|w| w.item.id == item.id) {
            if self.play_tab == self.tab {
                self.go(View::Watch);
            }
            return;
        }
        self.close_watch();
        let saved = self.s.positions.get(&item.id).map(|p| p.0);
        self.watch = Some(Watch {
            item: item.clone(),
            info: None,
            player: None,
            video_fmt: None,
            audio_fmt: None,
            heights: vec![],
            forced_height: None,
            langs: vec![],
            forced_lang: None,
            auto_cap: None,
            started_at: Instant::now(),
            frames_at: (0, Instant::now()),
            seek_at: None,
            next_prepared: None,
            auto_audio: false,
            stalled_since: None,
            last_abr: None,
            audio_only: false,
            sub_sel: None,
            cues: Vec::new(),
            live_back: 0.0,
            live_lag: 0.0,
            live_tick: Instant::now(),
            error: None,
            related: vec![],
            sponsors: vec![],
            skipped: HashSet::new(),
            tracking: None,
            cpn: crate::api::cpn(),
            pinged_start: false,
            last_ping: (0.0, Instant::now()),
            watched: 0.0,
            recovered: false,
            desc_open: false,
            opened_at: Instant::now(),
            resume_from: saved,
            undo_skip: None,
            last_save: Instant::now(),
            progress_at: (0.0, Instant::now()),
            stalled: false,
        });
        if !item.title.is_empty() {
            self.ops.push(WinOp::Title(format!("{} — Tube", item.title)));
        }
        if self.mode != Mode::Mini && self.play_tab == self.tab {
            self.go(View::Watch);
        }
        let id = item.id.clone();
        if let Some(info) = self.cached(&id) {
            self.start(info);
        } else {
            self.extract(&id);
        }
        let (api, tx, ctx) = (self.api.clone(), self.tx.clone(), self.ctx.clone());
        let cats = self.s.sb_categories.clone();
        let (sb, hist) = (self.s.sponsorblock, self.s.report_history && self.login == Login::Ok);
        std::thread::spawn(move || {
            if hist {
                if let Ok(t) = api.tracking(&id) {
                    let _ = tx.send(Msg::Tracking(id.clone(), t));
                }
            }
            if sb {
                let _ = tx.send(Msg::Sponsor(id.clone(), crate::sponsor::fetch(&api, &id, &cats)));
            }
            if let Ok(p) = api.related(&id) {
                let _ = tx.send(Msg::Related(id.clone(), p.items));
            }
            ctx.request_repaint();
        });
    }

    fn start(&mut self, info: Value) {
        let cap = self.s.quality.cap();
        let Some(w) = self.watch.as_mut() else { return };
        // qualité adaptative : au premier démarrage, la plus haute que le débit mesuré peut tenir
        if w.forced_height.is_none() && w.auto_cap.is_none() && w.player.is_none() {
            if let Some(sp) = crate::http::speed() {
                w.auto_cap = fit_cap(&info, cap, w.forced_lang.as_deref(), sp, false);
            }
        }
        let cap = w.auto_cap.map(|c| c.min(cap)).unwrap_or(cap);
        let (v, a, heights, langs) = pick(&info, w.forced_height.unwrap_or(cap), w.forced_lang.as_deref());
        w.heights = heights;
        w.langs = langs;
        if let Some(t) = info["title"].as_str() {
            if w.item.title.is_empty() {
                w.item.title = t.to_string();
            }
        }
        if w.item.channel.is_empty() {
            w.item.channel = info["channel"].as_str().unwrap_or("").to_string();
        }
        let dur = info["duration"].as_f64().unwrap_or(0.0);
        let title = format!("{} — Tube", w.item.title);
        w.info = Some(info);
        self.ops.push(WinOp::Title(title));
        let is_live = w.info.as_ref().is_some_and(|i| i["is_live"].as_bool() == Some(true));
        // direct : flux HLS (TS vidéo + son), la meilleure définition sous le plafond
        let v = if is_live {
            let cap = w.forced_height.unwrap_or(cap);
            let hls = w.info.as_ref().and_then(|i| i["hls"].as_array().cloned()).unwrap_or_default();
            let mut hs: Vec<u32> = hls.iter().filter_map(|f| f["height"].as_u64()).map(|h| h as u32).collect();
            hs.sort_unstable();
            hs.dedup();
            hs.reverse();
            w.heights = hs;
            let muxed = |f: &&Value| f["acodec"].as_str().is_some_and(|c| c != "none");
            let any_muxed = hls.iter().any(|f| muxed(&f) && !f["height"].is_null());
            hls.iter()
                .filter(|f| !any_muxed || muxed(f))
                .filter(|f| f["height"].as_u64().is_some_and(|h| h as u32 <= cap))
                .max_by(|a, b| (a["height"].as_u64(), a["fps"].as_f64().map(|x| x as u64)).cmp(&(b["height"].as_u64(), b["fps"].as_f64().map(|x| x as u64))))
                .map(|f| Fmt {
                    id: f["format_id"].as_str().unwrap_or("").to_string(),
                    url: format!("hls:{}-{}|{}|{}", w.cpn, w.live_back as u64, w.live_back, f["url"].as_str().unwrap_or("")),
                    ext: "ts".into(),
                    height: f["height"].as_u64().unwrap_or(0) as u32,
                    fps: f["fps"].as_f64().unwrap_or(0.0) as u32,
                    codec: f["vcodec"].as_str().unwrap_or("").to_string(),
                    tbr: f["tbr"].as_f64().unwrap_or(0.0),
                    lang: String::new(),
                    original: true,
                    label: String::new(),
                })
        } else {
            v
        };
        let live_muxed = v.as_ref().is_some_and(|f| f.url.starts_with("hls:")) && w.info.as_ref().and_then(|i| i["hls"].as_array()).is_some_and(|h| h.iter().any(|f| f["format_id"].as_str() == v.as_ref().map(|v| v.id.as_str()) && f["acodec"].as_str().is_some_and(|c| c != "none")));
        let a = if is_live && live_muxed {
            None
        } else if is_live {
            // son du direct : liste HLS sans vidéo, le meilleur débit
            let hls = w.info.as_ref().and_then(|i| i["hls"].as_array().cloned()).unwrap_or_default();
            hls.iter()
                .filter(|f| f["height"].is_null() && f["vcodec"].as_str().is_none_or(|c| c == "none"))
                .max_by(|a, b| a["tbr"].as_f64().unwrap_or(0.0).total_cmp(&b["tbr"].as_f64().unwrap_or(0.0)).then(a["format_id"].as_str().cmp(&b["format_id"].as_str())))
                .map(|f| Fmt {
                    id: f["format_id"].as_str().unwrap_or("").to_string(),
                    url: format!("hls:{}-{}|{}|{}", w.cpn, w.live_back as u64, w.live_back, f["url"].as_str().unwrap_or("")),
                    // son des directs : AAC brut (ADTS), pas du TS
                    ext: "aac".into(),
                    height: 0,
                    fps: 0,
                    codec: "aac".into(),
                    tbr: f["tbr"].as_f64().unwrap_or(0.0),
                    lang: String::new(),
                    original: true,
                    label: String::new(),
                })
        } else {
            a
        };
        // audio seul : le flux son est ouvert à la place de la vidéo ; une image de la vidéo vers
        // 1 s est décodée à part pour le fond, sans retarder le son (VP9 choisi, sinon H.264)
        if w.audio_only && !is_live && a.is_some() && self.audio_bg.as_ref().is_none_or(|b| b.0 != w.item.id) && self.audio_bg_asked != w.item.id {
            self.audio_bg_asked = w.item.id.clone();
            let mut cands: Vec<(String, String)> = v.iter().map(|f| (f.url.clone(), f.ext.clone())).collect();
            let h264 = w.info.as_ref().and_then(|i| i["formats"].as_array()).into_iter().flatten()
                .filter(|f| f["vcodec"].as_str().is_some_and(|c| c.starts_with("avc1")) && f["height"].as_u64().unwrap_or(0) <= 1080)
                .max_by_key(|f| f["height"].as_u64().unwrap_or(0));
            if let Some(f) = h264 {
                cands.push((f["url"].as_str().unwrap_or("").to_string(), f["ext"].as_str().unwrap_or("mp4").to_string()));
            }
            let (tx, ctx, id) = (self.tx.clone(), self.ctx.clone(), w.item.id.clone());
            std::thread::spawn(move || {
                let t = Instant::now();
                let img = cands.iter().find_map(|(u, e)| crate::player::grab_frame(u, e, 1.0, 2560));
                crate::store::log(&format!("fond audio seul {id} : {} en {} ms", if img.is_some() { "image décodée" } else { "échec" }, t.elapsed().as_millis()));
                let _ = tx.send(Msg::Frame(id, img));
                ctx.request_repaint();
            });
        }
        let (v, a) = if w.audio_only && !is_live && a.is_some() { (a.clone(), None) } else { (v, a) };
        let Some(v) = v else {
            w.error = Some("Aucun flux lisible pour cette vidéo.".into());
            return;
        };
        // reprise : position locale, sinon progression YouTube (vue sur un autre appareil)
        let mut start = w.player.as_ref().map(|p| p.time()).unwrap_or(0.0);
        if is_live {
            start = 0.0;
            w.resume_from = None;
        } else if w.player.is_none() {
            if let Some(r) = w.resume_from {
                start = r;
            } else if w.item.progress > 0.03 && w.item.progress < 0.95 && dur > 0.0 {
                start = w.item.progress as f64 * dur;
            }
            if dur > 0.0 && (start < 15.0 || start > dur - 20.0) {
                start = 0.0;
            }
            w.resume_from = if start > 0.0 { Some(start) } else { None };
        }
        let play = w.player.as_ref().is_none_or(|p| p.wants_play());
        let (audio_url, audio_ext) = match &a {
            Some(a) => (a.url.clone(), a.ext.clone()),
            None => (String::new(), String::new()),
        };
        if let Some(a) = &a {
            crate::store::log(&format!("audio : {} « {} »{} ({} pistes)", a.lang, a.label, if a.original { " originale" } else { "" }, w.langs.len()));
        }
        w.video_fmt = Some(v.clone());
        // audio seul : le flux son est ouvert comme « vidéo », il reste la piste audio courante
        w.audio_fmt = if w.audio_only && a.is_none() { Some(v.clone()) } else { a };
        let Some(vw) = &self.vidwin else {
            w.error = Some("fenêtre vidéo indisponible".into());
            return;
        };
        // test : toujours muet
        let vol = if self.s.muted || crate::store::test_mode() { 0.0 } else { self.s.volume };
        let c2 = self.ctx.clone();
        w.player = None;
        // vitesse mémorisée pour cette chaîne
        let ch_key = w.item.channel_id.clone().or_default_to(&w.item.channel);
        if let Some(r) = self.s.channel_speed.get(&ch_key) {
            self.s.speed = *r;
        }
        let rate = if is_live { 1.0 } else { self.s.speed };
        w.started_at = Instant::now();
        match Player::open(vw.hwnd.0 as isize, (&v.url, &v.ext), (&audio_url, &audio_ext), start, play, vol, rate, Arc::new(move || c2.request_repaint())) {
            Ok(p) => {
                let (pw, ph) = vw.size();
                if pw > 0 {
                    p.set_output(pw, ph);
                }
                w.player = Some(p);
                w.error = None;
                self.output_dirty = true;
            }
            Err(e) => w.error = Some(format!("Media Foundation : {e}")),
        }
        if let Some(r) = w.resume_from.take() {
            self.toast(format!("Reprise à {} · touche Début pour recommencer", fmt_time(r)));
        }
        // sous-titres : la langue choisie la dernière fois, si la vidéo la propose
        if self.watch.as_ref().is_some_and(|w| w.sub_sel.is_none()) {
            if let Some(l) = self.s.subs_lang.clone() {
                let subs = self.watch.as_ref().and_then(|w| w.info.as_ref()).and_then(|i| i["subs"].as_array().cloned()).unwrap_or_default();
                let pick = subs.iter().position(|s| s["lang"].as_str().is_some_and(|x| x == l) && s["auto"].as_bool() != Some(true)).or_else(|| subs.iter().position(|s| s["lang"].as_str().is_some_and(|x| x.split('-').next() == l.split('-').next())));
                if let Some(i) = pick {
                    self.load_subs(Some(i), false);
                }
            }
        }
    }

    /// Change la piste audio (langue) en gardant la position.
    pub fn set_lang(&mut self, code: &str) {
        let Some(w) = self.watch.as_mut() else { return };
        w.forced_lang = Some(code.to_string());
        if let Some(info) = w.info.clone() {
            self.start(info);
        }
    }

    /// Choisit les sous-titres (`None` = désactivés) et les télécharge. `remember` : devient la langue par défaut.
    pub fn load_subs(&mut self, i: Option<usize>, remember: bool) {
        let Some(w) = self.watch.as_mut() else { return };
        w.sub_sel = i;
        w.cues.clear();
        let entry = i.and_then(|i| w.info.as_ref().map(|inf| inf["subs"][i].clone()));
        if remember {
            self.s.subs_lang = entry.as_ref().and_then(|e| e["lang"].as_str().map(|s| s.to_string()));
        }
        let Some(e) = entry else { return };
        let Some(url) = e["url"].as_str().map(|s| s.to_string()) else { return };
        let (api, tx, ctx, id) = (self.api.clone(), self.tx.clone(), self.ctx.clone(), w.item.id.clone());
        std::thread::spawn(move || {
            let cues = api.get_json(&url).map(|v| parse_json3(&v)).unwrap_or_default();
            let _ = tx.send(Msg::Subs(id, cues));
            ctx.request_repaint();
        });
    }

    /// Y4 : titres des vidéos déjà vues (récentes + page Historique), proposés pendant la frappe,
    /// préfixés « ▶ ». Index local en mémoire : réponse immédiate, avant toute requête.
    pub fn local_videos(&self) -> Vec<(String, Item)> {
        let mut seen = HashSet::new();
        let hist = self.feeds.get("FEhistory").map(|f| f.items.clone()).unwrap_or_default();
        self.s
            .recent
            .iter()
            .chain(hist.iter())
            .filter(|i| i.kind == Kind::Video && !i.title.is_empty() && seen.insert(i.id.clone()))
            .take(400)
            .map(|i| (format!("▶ {}", i.title), i.clone()))
            .collect()
    }

    /// Candidats de l'autocomplétion : vidéos vues (locales) + dernières suggestions YouTube.
    pub fn refresh_candidates(&mut self, remote: Vec<String>) {
        let mut c: Vec<String> = self.local_videos().into_iter().map(|x| x.0).collect();
        c.extend(remote);
        self.ac.set_candidates(c);
        self.search_state.suggestions = self.ac.suggest(&self.search, 8);
        self.local_first();
    }

    /// Y4 : les vidéos déjà vues (« ▶ ») passent en tête des suggestions (3 au plus).
    pub fn local_first(&mut self) {
        let v = std::mem::take(&mut self.search_state.suggestions);
        let (mut local, rest): (Vec<_>, Vec<_>) = v.into_iter().partition(|s| s.text.starts_with("▶ "));
        let extra: Vec<_> = if local.len() > 3 { local.split_off(3) } else { Vec::new() };
        local.extend(rest);
        local.extend(extra);
        self.search_state.suggestions = local;
    }

    /// Suggestions de recherche (comme sur le site) pour le texte tapé.
    pub fn suggest(&mut self) {
        if self.sugg_q.is_empty() {
            // première frappe : l'historique local est disponible tout de suite
            self.refresh_candidates(Vec::new());
        }
        let q = self.search.trim().to_string();
        if q.is_empty() || q == self.sugg_q || q.starts_with("http") {
            return;
        }
        self.sugg_q = q.clone();
        let (api, tx, ctx) = (self.api.clone(), self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let url = format!("https://suggestqueries.google.com/complete/search?client=firefox&ds=yt&hl=fr&oe=utf-8&ie=utf-8&q={}", crate::api::urlenc(&q));
            let list = api.get_json(&url).ok().and_then(|v| v[1].as_array().cloned()).unwrap_or_default().iter().filter_map(|x| x.as_str().map(|s| s.to_string())).take(8).collect();
            let _ = tx.send(Msg::Suggest(q, list));
            ctx.request_repaint();
        });
    }

    /// Chapitre précédent (`dir` < 0) ou suivant. Sans chapitres : moments forts (Y2).
    pub fn chapter_jump(&mut self, dir: i32) {
        let Some(w) = self.watch.as_ref() else { return };
        let t = w.player.as_ref().map(|p| p.time()).unwrap_or(0.0);
        let mut starts: Vec<f64> = w.info.as_ref().and_then(|i| i["chapters"].as_array().cloned()).unwrap_or_default().iter().filter_map(|c| c["start"].as_f64()).collect();
        let moments = starts.is_empty();
        if moments {
            starts = highlights(w);
        }
        if starts.is_empty() {
            self.seek_by(if dir < 0 { -10.0 } else { 10.0 });
            return;
        }
        if moments {
            // moments forts : on saute 3 s avant le pic, pour voir arriver le passage
            let target = if dir < 0 { starts.iter().rev().find(|s| **s < t - 5.0).copied() } else { starts.iter().find(|s| **s > t + 4.0).copied() };
            let Some(target) = target else {
                self.toast(if dir < 0 { "Pas de moment fort avant" } else { "Pas d'autre moment fort" });
                return;
            };
            let n = starts.iter().position(|s| *s == target).unwrap_or(0) + 1;
            let total = starts.len();
            self.seek_to((target - 3.0).max(0.0));
            self.toast(format!("Moment fort {n}/{total} (les plus revus)"));
            return;
        }
        let target = if dir < 0 {
            // au début du chapitre courant, ou au précédent si on vient d'y entrer
            starts.iter().rev().find(|s| **s < t - 3.0).copied().unwrap_or(0.0)
        } else {
            match starts.iter().find(|s| **s > t + 0.5) {
                Some(s) => *s,
                None => return,
            }
        };
        self.seek_to(target);
        if let Some(name) = chapter_name(self.watch.as_ref(), target + 0.1) {
            self.toast(name);
        }
    }

    /// Touche V / bouton CC : sous-titres oui / non (langue par défaut : la dernière choisie,
    /// sinon le français, sinon la langue d'origine).
    pub fn toggle_subs(&mut self) {
        let Some(w) = self.watch.as_ref() else { return };
        if w.sub_sel.is_some() {
            self.load_subs(None, true);
            self.toast("Sous-titres désactivés");
            return;
        }
        let subs = w.info.as_ref().and_then(|i| i["subs"].as_array().cloned()).unwrap_or_default();
        let want = self.s.subs_lang.clone().unwrap_or_else(|| "fr".into());
        let i = subs
            .iter()
            .position(|s| s["lang"].as_str().is_some_and(|l| l.split('-').next() == want.split('-').next()))
            .or_else(|| subs.iter().position(|s| s["translated"].as_bool() != Some(true)));
        match i {
            Some(i) => {
                let name = subs[i]["name"].as_str().unwrap_or("").to_string();
                self.load_subs(Some(i), true);
                self.toast(format!("Sous-titres : {name}"));
            }
            None => self.toast("Pas de sous-titres pour cette vidéo"),
        }
    }

    /// Direct : secondes de retard sur le bord du direct (None si ce n'est pas un direct).
    pub fn live_behind(&self) -> Option<f64> {
        let w = self.watch.as_ref()?;
        if w.info.as_ref()?["is_live"].as_bool() != Some(true) {
            return None;
        }
        w.player.as_ref()?;
        // un direct avance au rythme du temps réel : le retard ne grandit que pendant les pauses et attentes
        Some(w.live_back + w.live_lag)
    }

    /// Direct : reprend `back` secondes avant le bord du direct (0 = au direct).
    pub fn live_seek(&mut self, back: f64) {
        let win = crate::live::window();
        let Some(w) = self.watch.as_mut() else { return };
        w.live_back = back.clamp(0.0, (win - 20.0).max(0.0));
        w.live_lag = 0.0;
        w.live_tick = Instant::now();
        if let Some(info) = w.info.clone() {
            self.start(info);
        }
        self.unfreeze();
    }

    /// Capture de l'image affichée, enregistrée dans Images\Tube.
    pub fn screenshot(&mut self) {
        let Some(w) = &self.watch else { return };
        let Some(img) = self.vidwin.as_ref().and_then(|v| v.snapshot(8192)) else {
            self.toast("Capture impossible (vidéo non affichée)");
            return;
        };
        let t = w.player.as_ref().map(|p| p.time()).unwrap_or(0.0);
        let name: String = w.item.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).take(60).collect();
        let dir = dirs::picture_dir().unwrap_or_else(|| crate::store::dir()).join("Tube");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{} {}.png", name.trim(), fmt_time(t).replace(':', "-")));
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| [p.r(), p.g(), p.b(), 255]).collect();
        match image::save_buffer(&path, &rgba, img.size[0] as u32, img.size[1] as u32, image::ColorType::Rgba8) {
            Ok(()) => self.toast(format!("Capture enregistrée : {}", path.display())),
            Err(e) => self.toast(format!("Capture impossible : {e}")),
        }
    }

    pub fn set_sleep(&mut self, s: Option<Sleep>) {
        self.sleep = s;
        self.toast(match s {
            None => "Minuterie désactivée".to_string(),
            Some(Sleep::EndOfVideo) => "Pause à la fin de cette vidéo".to_string(),
            Some(Sleep::At(t)) => format!("Pause dans {} min", (t.saturating_duration_since(Instant::now()).as_secs() + 30) / 60),
        });
    }

    pub fn set_audio_only(&mut self, on: bool) {
        // en venant de la vidéo : sa dernière image affichée sert de fond
        if on && self.watch.as_ref().is_some_and(|w| !w.audio_only) {
            if let (Some(img), Some(id)) = (self.vidwin.as_ref().and_then(|v| v.snapshot(2560)), self.watch.as_ref().map(|w| w.item.id.clone())) {
                self.audio_bg = Some((id.clone(), self.ctx.load_texture("audio_bg", img, egui::TextureOptions::LINEAR)));
                self.audio_bg_asked = id;
            }
        }
        let Some(w) = self.watch.as_mut() else { return };
        w.audio_only = on;
        if let Some(info) = w.info.clone() {
            self.start(info);
        }
    }

    pub fn set_quality(&mut self, h: Option<u32>) {
        let Some(w) = self.watch.as_mut() else { return };
        w.forced_height = h;
        w.auto_cap = None;
        w.audio_only = false;
        if let Some(info) = w.info.clone() {
            self.start(info);
        }
    }

    pub fn close_watch(&mut self) {
        if let Some(mut w) = self.watch.take() {
            if let Some(p) = &w.player {
                let (r, d) = p.stats();
                crate::store::log(&format!("fin {} : {} · {}x{} · {} images, {} perdues · t={:.0}/{:.0}", w.item.id, self.quality_label_of(&w), p.size.0, p.size.1, r, d, p.time(), p.duration));
                let t = p.time();
                self.remember(&w.item.id, t, p.duration);
            }
            self.final_ping(&mut w);
        }
        if let Some(v) = &mut self.vidwin {
            v.hide();
        }
        self.video_px = None;
        self.ops.push(WinOp::Title("Tube".into()));
    }

    pub fn stop(&mut self) {
        self.close_watch();
        if self.mode != Mode::Normal {
            self.ops.push(WinOp::Mini(false));
            self.ops.push(WinOp::Half(false));
            self.ops.push(WinOp::Fullscreen(false));
        }
        if self.view == View::Watch {
            self.go_back();
        }
    }

    fn remember(&mut self, id: &str, t: f64, dur: f64) {
        if dur > 0.0 && dur.is_finite() && t > 15.0 && t < dur - 20.0 {
            self.s.positions.insert(id.to_string(), (t, dur, crate::store::now()));
        } else {
            self.s.positions.remove(id);
        }
    }

    pub fn player(&mut self) -> Option<&mut Player> {
        self.watch.as_mut().and_then(|w| w.player.as_mut())
    }

    pub fn toggle(&mut self) {
        if let Some(b) = self.live_behind() {
            let paused = self.player().is_some_and(|p| !p.wants_play());
            if paused && b - self.watch.as_ref().map(|w| w.live_back).unwrap_or(0.0) > 60.0 {
                // longue pause sur un direct : on repart au même retard (les segments en mémoire ne suffisent plus)
                self.live_seek(b);
                return;
            }
        }
        if let Some(p) = self.player() {
            p.toggle();
        }
        let paused = self.player().is_some_and(|p| !p.wants_play());
        crate::store::log(if paused { "pause" } else { "lecture" });
        if paused {
            if let Some(mut w) = self.watch.take() {
                self.final_ping(&mut w);
                self.watch = Some(w);
            }
        }
    }

    pub fn seek_by(&mut self, d: f64) {
        if let Some(b) = self.live_behind() {
            self.live_seek(b - d);
            return;
        }
        if let Some(p) = self.player() {
            let t = p.time() + d;
            p.seek(t);
        }
        self.unfreeze();
        self.overlay_until = Instant::now() + Duration::from_secs(2);
    }

    pub fn seek_to(&mut self, t: f64) {
        if let Some(p) = self.player() {
            p.seek(t);
        }
        if let Some(w) = self.watch.as_mut() {
            let f0 = w.player.as_ref().map(|p| p.stats().0).unwrap_or(0);
            w.seek_at = Some((Instant::now(), f0, t));
        }
        self.unfreeze();
    }

    /// Oublie l'image figée (la vraie vidéo réapparaît), recapture plus tard si besoin.
    pub fn unfreeze(&mut self) {
        self.still = None;
        self.still_after = Instant::now() + Duration::from_millis(600);
    }

    pub fn change_volume(&mut self, d: f64) {
        self.s.volume = (self.s.volume + d).clamp(0.0, 1.0);
        self.s.muted = false;
        let v = self.s.volume;
        if let Some(p) = self.player() {
            p.set_volume(v);
        }
        self.toast(format!("Volume {:.0} %", v * 100.0));
    }

    pub fn toggle_mute(&mut self) {
        self.s.muted = !self.s.muted;
        let v = if self.s.muted { 0.0 } else { self.s.volume };
        if let Some(p) = self.player() {
            p.set_volume(v);
        }
        self.toast(if self.s.muted { "Son coupé" } else { "Son rétabli" });
    }

    pub fn set_speed(&mut self, r: f64) {
        self.s.speed = r;
        if let Some(w) = &self.watch {
            let k = w.item.channel_id.clone().or_default_to(&w.item.channel);
            if !k.is_empty() {
                self.s.channel_speed.insert(k, r);
            }
        }
        if let Some(p) = self.player() {
            p.set_rate(r);
        }
        self.toast(format!("Vitesse ×{r}"));
    }

    pub fn toggle_mini(&mut self) {
        if self.watch.is_none() {
            return;
        }
        let on = self.mode != Mode::Mini;
        self.ops.push(WinOp::Mini(on));
    }

    pub fn toggle_full(&mut self) {
        if self.watch.is_none() {
            return;
        }
        self.ops.push(WinOp::Fullscreen(self.mode != Mode::Full));
    }

    /// Demi-écran : toute la hauteur et la moitié de la largeur du moniteur, côté où se trouve la fenêtre.
    pub fn toggle_half(&mut self) {
        if self.watch.is_none() {
            return;
        }
        self.ops.push(WinOp::Half(self.mode != Mode::Half));
    }

    /// Quitte le plein écran ou le demi-écran.
    pub fn leave_big(&mut self) {
        match self.mode {
            Mode::Full => self.ops.push(WinOp::Fullscreen(false)),
            Mode::Half => self.ops.push(WinOp::Half(false)),
            _ => {}
        }
    }

    pub fn next_in_queue(&mut self) -> bool {
        if self.qpos + 1 < self.queue.len() {
            self.qpos += 1;
            let it = self.queue[self.qpos].clone();
            let q = std::mem::take(&mut self.queue);
            let pos = self.qpos;
            self.auto_next = true;
            self.open(&it, Some((q, pos)));
            return true;
        }
        false
    }

    // ---------- Historique YouTube ----------
    fn ping_params(w: &Watch, cmt: f64, st: f64, et: f64, state: &str, final_: bool) -> Vec<(&'static str, String)> {
        let len = w.player.as_ref().map(|p| p.duration).unwrap_or(0.0);
        let mut v = vec![
            ("ver", "2".to_string()),
            ("cpn", w.cpn.clone()),
            ("c", "WEB".into()),
            ("cver", crate::api::CLIENT_VERSION.into()),
            ("cplayer", "UNIPLAYER".into()),
            ("cmt", format!("{cmt:.3}")),
            ("st", format!("{st:.3}")),
            ("et", format!("{et:.3}")),
            ("len", format!("{len:.3}")),
            ("rt", format!("{:.3}", w.opened_at.elapsed().as_secs_f64())),
            ("state", state.into()),
            ("muted", "0".into()),
            ("fmt", w.video_fmt.as_ref().map(|f| f.id.clone()).unwrap_or_default()),
            ("afmt", w.audio_fmt.as_ref().map(|f| f.id.clone()).unwrap_or_default()),
            ("ns", "yt".into()),
            ("el", "detailpage".into()),
        ];
        if final_ {
            v.push(("final", "1".into()));
        }
        v
    }

    fn final_ping(&self, w: &mut Watch) {
        let (Some(tr), Some(p)) = (&w.tracking, &w.player) else { return };
        if !w.pinged_start {
            return;
        }
        let t = p.time();
        let params = Self::ping_params(w, t, w.last_ping.0, t, "paused", true);
        let (api, url) = (self.api.clone(), tr.watchtime.clone());
        w.last_ping = (t, Instant::now());
        std::thread::spawn(move || api.ping(&url, &params));
    }

    fn tick_tracking(&mut self) {
        let api = self.api.clone();
        let Some(w) = self.watch.as_mut() else { return };
        let (Some(tr), Some(p)) = (&w.tracking, &w.player) else { return };
        if !p.playing() || p.buffering {
            return;
        }
        let t = p.time();
        if !w.pinged_start {
            w.pinged_start = true;
            w.last_ping = (t, Instant::now());
            let params = Self::ping_params(w, t, t, t, "playing", false);
            let (a, u) = (api.clone(), tr.playback.clone());
            std::thread::spawn(move || a.ping(&u, &params));
            return;
        }
        if w.last_ping.1.elapsed() >= Duration::from_secs(20) {
            let st = w.last_ping.0;
            let params = Self::ping_params(w, t, st, t, "playing", false);
            w.last_ping = (t, Instant::now());
            let (a, u) = (api, tr.watchtime.clone());
            std::thread::spawn(move || a.ping(&u, &params));
        }
    }

    // ---------- Boucle ----------
    /// Logique périodique (messages des fils, lecteur, SponsorBlock, sauvegardes). Renvoie le délai souhaité avant le prochain passage.
    pub fn tick(&mut self) -> Option<Duration> {
        if let Some(rx) = &self.hk_rx {
            if let Ok(f) = rx.try_recv() {
                crate::store::log(&format!("raccourcis indisponibles : {f:?}"));
                self.hk_failed = f;
                self.hk_rx = None;
            }
        }
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }
        crate::hotkeys::set_active(self.watch.is_some());
        if let Some(s) = self.smtc.as_mut() {
            let now = self.watch.as_ref().map(|w| (w.item.id.as_str(), w.item.title.as_str(), w.item.channel.as_str(), w.item.thumb.as_str(), w.player.as_ref().is_some_and(|p| p.wants_play())));
            s.update(now);
        }
        if self.viz.is_some() && !self.watch.as_ref().is_some_and(|w| w.audio_only) {
            self.viz = None;
        }
        // préchargement : une extraction d'avance à la fois, une place reste libre pour le clic
        if self.extracting.is_empty() {
            while let Some(id) = self.prefetch_q.pop_front() {
                if self.cached(&id).is_none() && !self.extracting.contains(&id) {
                    self.extract_prio(&id, 2);
                    break;
                }
            }
        }
        let mut next = None;
        let mut ended = false;
        let mut err_recover = None;
        if let Some(w) = self.watch.as_mut() {
            if let Some(p) = w.player.as_mut() {
                p.poll();
                if let Some(e) = &p.error {
                    // un direct peut être relancé autant de fois que nécessaire (adresses refusées en cours de route)
                    let live = w.info.as_ref().is_some_and(|i| i["is_live"].as_bool() == Some(true));
                    if !w.recovered || live {
                        w.recovered = true;
                        err_recover = Some(p.time());
                    } else if w.error.is_none() {
                        w.error = Some(e.clone());
                        crate::store::log(&format!("lecteur : {e}"));
                    }
                }
                ended = p.ended;
                let t = p.time();
                if (t - w.progress_at.0).abs() > 0.05 || !p.wants_play() || !p.ready {
                    w.progress_at = (t, Instant::now());
                } else if w.progress_at.1.elapsed() > Duration::from_secs(12) && err_recover.is_none() {
                    crate::store::log(&format!("lecture bloquée à {t:.1} s : réouverture"));
                    w.progress_at = (t, Instant::now());
                    err_recover = Some(t);
                }
                if w.info.as_ref().is_some_and(|i| i["is_live"].as_bool() == Some(true)) {
                    let dt = w.live_tick.elapsed().as_secs_f64();
                    w.live_tick = Instant::now();
                    if !p.wants_play() || !p.ready || w.stalled {
                        w.live_lag += dt;
                    }
                }
                // attente réseau (lecture demandée mais l'image n'avance plus) : l'interface l'affiche
                let stalled = p.wants_play() && p.ready && w.progress_at.1.elapsed() > Duration::from_millis(900);
                let frames = p.stats().0;
                if let Some((t0, f0, target)) = w.seek_at {
                    let _ = f0;
                    if p.time() > target + 0.3 && p.time() < target + 6.0 {
                        crate::store::log(&format!("saut : lecture reprise en {} ms", t0.elapsed().as_millis()));
                        w.seek_at = None;
                    }
                }
                if frames != w.frames_at.0 || !p.wants_play() || !p.ready {
                    w.frames_at = (frames, Instant::now());
                }
                // images qui n'avancent plus : seulement si la vidéo est réellement affichée (fenêtre
                // réduite, autre page ou autre onglet : aucune image n'est dessinée, ce n'est pas un manque de débit)
                let shown = self.video_px.is_some() && !self.occluded;
                let starving = shown && !w.audio_only && w.frames_at.1.elapsed() > Duration::from_secs(2);
                if stalled || starving {
                    w.stalled_since.get_or_insert_with(Instant::now);
                } else {
                    w.stalled_since = None;
                }
                if stalled != w.stalled {
                    w.stalled = stalled;
                    self.ctx.request_repaint();
                }
                if p.wants_play() {
                    next = Some(Duration::from_millis(250));
                }
                if p.playing() {
                    next = Some(Duration::from_millis(250));
                    // SponsorBlock
                    for (i, s) in w.sponsors.iter().enumerate() {
                        if t >= s.start && t < s.end - 0.4 && !w.skipped.contains(&i) {
                            w.skipped.insert(i);
                            w.undo_skip = Some(s.start);
                            let lbl = crate::sponsor::label(&s.category);
                            let end = s.end;
                            p.seek(end);
                            self.toast = Some((format!("Passage « {lbl} » ignoré · Ctrl+Z pour revenir"), Instant::now()));
                            break;
                        }
                    }
                    if w.last_save.elapsed() > Duration::from_secs(5) {
                        w.last_save = Instant::now();
                        let (id, d) = (w.item.id.clone(), p.duration);
                        if d > 0.0 && t > 15.0 && t < d - 20.0 {
                            self.s.positions.insert(id, (t, d, crate::store::now()));
                        } else if d > 0.0 && t >= d - 20.0 {
                            self.s.positions.remove(&id);
                        }
                    }
                }
            }
        }
        // qualité adaptative : attente réseau de plus de 3 s → définition inférieure, même position
        let abr = self.watch.as_ref().and_then(|w| {
            // attente en cours de lecture, ou démarrage qui n'aboutit pas
            let not_starting = w.player.as_ref().is_some_and(|p| !p.ready) && w.started_at.elapsed() > Duration::from_secs(6);
            let long = w.stalled_since.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) || not_starting;
            let calm = w.last_abr.is_none_or(|t| t.elapsed() > Duration::from_secs(15));
            let live = w.info.as_ref().is_some_and(|i| i["is_live"].as_bool() == Some(true));
            if !long || !calm || w.forced_height.is_some() || w.audio_only || live {
                return None;
            }
            let info = w.info.as_ref()?;
            let cap = w.auto_cap.unwrap_or(self.s.quality.cap());
            let sp = crate::http::recent_rate().max(crate::http::speed().unwrap_or(0.0) * 0.5);
            fit_cap(info, cap, w.forced_lang.as_deref(), sp, true)
        });
        if let Some(h) = abr {
            if let Some(w) = self.watch.as_mut() {
                crate::store::log(&format!("connexion lente ({:.0} Ko/s) : passage en {h}p", crate::http::speed().unwrap_or(0.0) / 1024.0));
                w.auto_cap = Some(h);
                w.last_abr = Some(Instant::now());
                w.stalled_since = None;
                if let Some(info) = w.info.clone() {
                    self.start(info);
                }
                self.toast(format!("Connexion lente : qualité réduite à {h}p (⚙ pour choisir)"));
            }
        }
        if let Some(t) = err_recover {
            // URL expirée ou coupure : nouvelle extraction puis reprise au même endroit
            if let Some(w) = self.watch.as_mut() {
                let id = w.item.id.clone();
                w.resume_from = Some(t);
                w.player = None;
                self.cache.remove(&id);
                self.extract(&id);
            }
        }
        // Y1 : 30 s avant la fin, la vidéo suivante (file d'attente, sinon suggestion en lecture auto)
        // est extraite et ses premiers blocs téléchargés : l'enchaînement est immédiat
        let next_vid = self.watch.as_ref().and_then(|w| {
            let p = w.player.as_ref()?;
            if w.next_prepared.is_some() || !p.wants_play() || p.duration <= 0.0 || p.duration - p.time() > 30.0 {
                return None;
            }
            let it = self.queue.get(self.qpos + 1).cloned().or_else(|| self.s.autoplay_next.then(|| w.related.first().cloned()).flatten())?;
            Some(it.id)
        });
        if let Some(id) = next_vid {
            if let Some(w) = self.watch.as_mut() {
                w.next_prepared = Some(id.clone());
            }
            crate::store::log(&format!("enchaînement : préparation de {id}"));
            self.extract_prio(&id, 0);
        }
        if let Some(id) = self.watch.as_ref().and_then(|w| w.next_prepared.clone()) {
            if self.warmed.as_deref() != Some(&id) {
                if let Some(info) = self.cached(&id) {
                    let (v, a, _, _) = pick(&info, self.s.quality.cap(), None);
                    crate::http::prewarm(v.as_ref().map(|f| f.url.as_str()).unwrap_or(""));
                    crate::http::prewarm(a.as_ref().map(|f| f.url.as_str()).unwrap_or(""));
                    self.warmed = Some(id);
                }
            }
        }
        // Y3 : fenêtre réduite depuis 5 s sur une musique → audio seul (moins de réseau et de GPU) ;
        // restaurée → retour à la vidéo, au même endroit
        let music = self.watch.as_ref().is_some_and(|w| w.info.as_ref().is_some_and(|i| i["categories"].as_array().is_some_and(|c| c.iter().any(|x| x.as_str() == Some("Music")))));
        let playing = self.watch.as_ref().and_then(|w| w.player.as_ref()).is_some_and(|p| p.wants_play());
        let auto = self.watch.as_ref().is_some_and(|w| w.auto_audio);
        let audio = self.watch.as_ref().is_some_and(|w| w.audio_only);
        if music && playing && !audio && self.mode == Mode::Normal && self.minimized.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            crate::store::log("arrière-plan : musique, fenêtre réduite → audio seul");
            self.set_audio_only(true);
            if let Some(w) = self.watch.as_mut() {
                w.auto_audio = true;
            }
        } else if auto && self.minimized.is_none() {
            crate::store::log("arrière-plan : fenêtre restaurée → retour à la vidéo");
            if let Some(w) = self.watch.as_mut() {
                w.auto_audio = false;
            }
            self.set_audio_only(false);
        }
        // minuterie : pause à l'heure dite
        if let Some(Sleep::At(t)) = self.sleep {
            if Instant::now() >= t {
                self.sleep = None;
                if let Some(p) = self.player() {
                    if p.wants_play() {
                        p.toggle();
                    }
                }
                self.toast("Minuterie : lecture mise en pause");
            }
        }
        if ended && self.loop_on {
            if let Some(p) = self.player() {
                p.ended = false;
                p.seek(0.0);
                p.play();
            }
            ended = false;
        }
        if ended && self.sleep == Some(Sleep::EndOfVideo) {
            self.sleep = None;
            if let Some(p) = self.player() {
                p.ended = false;
                p.pause();
            }
            self.toast("Minuterie : fin de la vidéo, lecture arrêtée");
            ended = false;
        }
        if ended {
            if let Some(w) = self.watch.as_mut() {
                let id = w.item.id.clone();
                self.s.positions.remove(&id);
                if let Some(p) = w.player.as_mut() {
                    p.ended = false;
                    p.pause();
                }
            }
            if !self.next_in_queue() && self.s.autoplay_next {
                if let Some(it) = self.watch.as_ref().and_then(|w| w.related.first().cloned()) {
                    self.auto_next = true;
                    self.open(&it, None);
                }
            }
        }
        self.tick_tracking();
        if self.last_disk_save.elapsed() > Duration::from_secs(30) {
            self.last_disk_save = Instant::now();
            self.store_tabs();
            self.s.save_bg();
        }
        if let Some(e) = crate::store::take_save_error() {
            self.toast(format!("Réglages non enregistrés : {e}"));
        }
        if let Some((_, t)) = &self.toast {
            let left = Duration::from_millis(3200).saturating_sub(t.elapsed());
            if left.is_zero() {
                self.toast = None;
            } else {
                next = Some(next.map_or(left, |n: Duration| n.min(left)));
            }
        }
        next
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Feed(key, r, more) => {
                crate::store::log(&match &r {
                    Ok(p) => format!("flux {key} : {} éléments, suite {}, connecté {:?}", p.items.len(), p.cont.is_some(), p.logged_in),
                    Err(e) => format!("flux {key} : erreur {e}"),
                });
                let hide = self.s.hide_shorts;
                let f = self.feeds.entry(key.clone()).or_default();
                f.loading = false;
                match r {
                    Ok(mut p) => {
                        if p.logged_in == Some(false) && self.login == Login::Ok && !key.starts_with("search:") {
                            self.session_lost();
                        }
                        if hide {
                            p.items.retain(|i| !i.short);
                        }
                        if self.s.hide_watched && !key.starts_with("VL") && key != "FEhistory" {
                            let pos = &self.s.positions;
                            p.items.retain(|i| i.progress < 0.9 && pos.get(&i.id).is_none_or(|x| x.0 / x.1.max(1.0) < 0.9));
                        }
                        // les premières vidéos de la page affichée sont extraites d'avance
                        if !more && self.view.feed_key().as_deref() == Some(key.as_str()) {
                            self.prefetch_q = p.items.iter().filter(|i| i.kind == Kind::Video && !i.live && !i.short).take(2).map(|i| i.id.clone()).collect();
                        }
                        let f = self.feeds.entry(key).or_default();
                        if more {
                            let seen: HashSet<String> = f.items.iter().map(|i| i.id.clone()).collect();
                            f.items.extend(p.items.into_iter().filter(|i| !seen.contains(&i.id)));
                        } else {
                            f.items = p.items;
                        }
                        f.cont = p.cont;
                        f.loaded = Some(Instant::now());
                    }
                    Err(e) => f.error = Some(e),
                }
            }
            Msg::Extracted(id, r) => {
                crate::store::log(&match &r {
                    Ok(v) => format!("extraction {id} : {} formats", v["formats"].as_array().map(|a| a.len()).unwrap_or(0)),
                    Err(e) => format!("extraction {id} : erreur {e}"),
                });
                self.extracting.remove(&id);
                self.extracting_prio.remove(&id);
                let playing_this = self.watch.as_ref().is_some_and(|w| w.item.id == id && w.player.is_none());
                match r {
                    Ok(v) => {
                        // extractions expirées retirées au passage (adresses valables ~6 h, gardées 3 h)
                        self.cache.retain(|_, (_, t)| t.elapsed() < Duration::from_secs(3 * 3600));
                        self.cache.insert(id.clone(), (v.clone(), Instant::now()));
                        crate::store::add_extract(id.clone(), v.clone());
                        if playing_this {
                            self.start(v);
                        }
                    }
                    // worker relancé pendant l'extraction (mise à jour, nouveaux cookies) : on recommence
                    Err(e) if playing_this && (e.contains("s'est arrêté") || e.contains("ne répond pas")) => {
                        self.extract(&id);
                    }
                    Err(e) => {
                        if playing_this {
                            // le worker traduit déjà ses erreurs (`human` dans worker.py)
                            let msg = if e.contains("anti-robot") || e.contains("Sign in") {
                                "YouTube demande une vérification anti-robot. Ouvre youtube.com dans ton navigateur, puis Paramètres › Reconnecter.".to_string()
                            } else {
                                e
                            };
                            if let Some(w) = self.watch.as_mut() {
                                w.error = Some(msg);
                            }
                        }
                    }
                }
            }
            Msg::Related(id, items) => {
                let hide = self.s.hide_shorts;
                if let Some(w) = self.watch.as_mut().filter(|w| w.item.id == id) {
                    w.related = items.into_iter().filter(|i| !(hide && i.short)).collect();
                }
            }
            Msg::Tracking(id, t) => {
                if let Some(w) = self.watch.as_mut().filter(|w| w.item.id == id) {
                    w.tracking = Some(t);
                }
            }
            Msg::Sponsor(id, s) => {
                if let Some(w) = self.watch.as_mut().filter(|w| w.item.id == id) {
                    w.sponsors = s;
                }
            }
            Msg::Cookies(r) => match r {
                Ok(note) => {
                    crate::store::log(&format!("import : {note}"));
                    let jar = Jar::load(&crate::store::cookies_path());
                    let ok = jar.logged_in();
                    *self.api.jar.lock().unwrap() = jar;
                    self.reconnect = false;
                    // nouveaux cookies pris sans tuer le worker (les extractions en cours continuent)
                    let _ = self.worker.send(serde_json::json!({"cmd": "reload"}));
                    if ok {
                        self.login = Login::Ok;
                        self.toast(format!("Connecté · {note}"));
                        self.feeds.clear();
                        self.cache.clear();
                        let v = if self.view.feed_key().is_some() { self.view.clone() } else { View::Home };
                        self.load_feed(&v, false);
                    } else {
                        self.login = Login::Need;
                        self.login_msg = Some("Aucune session YouTube trouvée dans ce navigateur : connecte-toi à youtube.com dedans, puis réessaie.".into());
                    }
                }
                Err(e) => {
                    crate::store::log(&format!("import : {e}"));
                    if e == crate::cookies::LOCKED {
                        self.reconnect = true;
                    }
                    let had = self.api.jar.lock().unwrap().logged_in();
                    self.login = if had { Login::Ok } else { Login::Need };
                    if had && self.feeds.is_empty() {
                        self.load_feed(&View::Home, false);
                    }
                    self.login_msg = Some(e.clone());
                    self.toast(format!("Connexion impossible : {e}"));
                }
            },
            Msg::SetupStep(s) => self.setup = Setup::Running(s),
            Msg::SetupDone(r) => match { crate::store::log(&format!("installation : {r:?}")); r } {
                Ok(()) => {
                    self.setup = Setup::Done;
                    self.after_setup();
                    if self.login == Login::Ok {
                        self.load_feed(&View::Home, false);
                    }
                }
                Err(e) => self.setup = Setup::Failed(e),
            },
            Msg::Updated(r) => match r {
                Ok(Some(v)) => {
                    self.worker.restart();
                    self.ytdlp_version = Some(v.clone());
                    self.toast(format!("yt-dlp mis à jour ({v})"));
                }
                Ok(None) => {}
                Err(e) => self.toast(format!("Mise à jour de yt-dlp impossible : {e}")),
            },
            Msg::Suggest(q, list) => {
                if std::env::var_os("TUBE_SCRIPT").is_some() {
                    crate::store::log(&format!("suggestions « {q} » : {}", list.len()));
                }
                if self.search.trim() == q {
                    // suggestions YouTube = candidats ; l'historique local reste prioritaire (socle)
                    self.refresh_candidates(list);
                }
            }
            Msg::Subs(id, cues) => {
                if let Some(w) = self.watch.as_mut().filter(|w| w.item.id == id) {
                    if cues.is_empty() {
                        self.toast = Some(("Sous-titres indisponibles".into(), Instant::now()));
                    }
                    w.cues = cues;
                }
            }
            Msg::Frame(id, img) => {
                if let Some(img) = img.filter(|_| self.watch.as_ref().is_some_and(|w| w.item.id == id)) {
                    self.audio_bg = Some((id, self.ctx.load_texture("audio_bg", img, egui::TextureOptions::LINEAR)));
                }
            }
            Msg::Toast(t) => {
                if let Some(v) = t.strip_prefix("__version:") {
                    self.ytdlp_version = Some(v.to_string());
                } else {
                    self.toast(t);
                }
            }
        }
    }

    /// Google a invalidé la session : réimport automatique (une fois par lancement).
    fn session_lost(&mut self) {
        // (appelée quand YouTube répond « déconnecté »)
        if crate::store::test_mode() {
            return;
        }
        if self.reimported.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) {
            self.toast("Session YouTube expirée : Réglages › Reconnecter");
            return;
        }
        self.reimported = Some(Instant::now());
        // jamais d'arrêt du navigateur sans clic : copie silencieuse, sinon bandeau « Reconnecter »
        if let Some((_, p)) = crate::cookies::browser_profiles().into_iter().next() {
            self.import_cookies(p, false);
        } else {
            self.reconnect = true;
        }
    }

    pub fn set_dark(&mut self, d: bool) {
        self.s.dark = d;
        self.pal = socle::theme::palette(d);
        socle::theme::apply(&self.ctx, &self.pal);
    }

    pub fn quality_label(&self) -> String {
        self.watch.as_ref().map(|w| self.quality_label_of(w)).unwrap_or_default()
    }

    pub fn quality_label_of(&self, w: &Watch) -> String {
        if w.audio_only {
            return "Audio seul".into();
        }
        w.video_fmt
            .as_ref()
            .map(|f| {
                let c = if f.codec.starts_with("vp") { "VP9" } else if f.codec.starts_with("av01") { "AV1" } else { "H.264" };
                format!("{}p{} {c}", f.height, if f.fps > 30 { f.fps.to_string() } else { String::new() })
            })
            .unwrap_or_default()
    }

    pub fn _quality_default(&self) -> Quality {
        self.s.quality
    }
}

#[cfg(test)]
mod tests {
    use super::pick;
    use serde_json::json;

    #[test]
    fn pick_prefere_vp9_et_ignore_drc_et_doublage() {
        let info = json!({"formats": [
            {"format_id": "400", "url": "u", "vcodec": "av01.0.12M.08", "acodec": "none", "height": 1440, "fps": 60, "tbr": 9000.0},
            {"format_id": "271", "url": "u", "vcodec": "vp9", "acodec": "none", "height": 1440, "fps": 60, "tbr": 8000.0},
            {"format_id": "137", "url": "u", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "fps": 30, "tbr": 4000.0},
            {"format_id": "251-drc", "url": "u", "vcodec": "none", "acodec": "opus", "tbr": 200.0, "language": "en", "format_note": "English original (default), medium, DRC"},
            {"format_id": "251-1", "url": "u", "vcodec": "none", "acodec": "opus", "tbr": 300.0, "language": "fr", "format_note": "French dubbed, medium"},
            {"format_id": "251", "url": "u", "vcodec": "none", "acodec": "opus", "tbr": 130.0, "language": "en", "language_preference": 10, "format_note": "English original (default), medium"},
            {"format_id": "140", "url": "u", "vcodec": "none", "acodec": "mp4a.40.2", "tbr": 129.0, "language": "en", "format_note": "English original (default), medium"}
        ]});
        let (v, a, _, _) = pick(&info, 1440, None);
        assert_eq!(v.unwrap().id, "271");
        assert_eq!(a.unwrap().id, "251");
    }
}
