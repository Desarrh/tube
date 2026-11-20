//! Réglages et données locales : %LOCALAPPDATA%\Tube (settings.json, cookies.txt, outils).
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Mode test (TUBE_TEST, TUBE_SCRIPT ou TUBE_QUIT_AFTER) : l'utilisateur ne doit RIEN remarquer.
/// Fenêtre hors écran sans focus, muette, sans raccourcis globaux ni panneau média, sans toucher
/// au navigateur, et données à part (%TEMP%\tube-test) : ses réglages et son journal restent intacts.
pub fn test_mode() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| ["TUBE_TEST", "TUBE_SCRIPT", "TUBE_QUIT_AFTER"].iter().any(|k| std::env::var_os(k).is_some()) || socle::testmode::enabled())
}

/// `%LOCALAPPDATA%\Tube` : cookies, outils (venv, deno), journal — données lourdes ou locales.
fn real_dir() -> PathBuf {
    socle::paths::cache_dir("Tube")
}

pub fn dir() -> PathBuf {
    if test_mode() {
        let d = std::env::temp_dir().join("tube-test");
        let _ = std::fs::create_dir_all(&d);
        // copie en lecture seule des cookies (jamais réécrits chez l'utilisateur)
        let c = d.join("cookies.txt");
        let src = real_dir().join("cookies.txt");
        let mt = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if !c.exists() || mt(&src) > mt(&c) {
            let _ = std::fs::copy(&src, &c);
        }
        return d;
    }
    real_dir()
}
/// Outils (venv yt-dlp, deno) : partagés, utilisés en lecture seule par le mode test.
pub fn bin_dir() -> PathBuf {
    real_dir().join("bin")
}
pub fn cookies_path() -> PathBuf {
    dir().join("cookies.txt")
}

/// Échéance (secondes Unix) des adresses d'une extraction : paramètre `expire=` de googlevideo.
pub fn extract_expiry(info: &serde_json::Value) -> Option<u64> {
    let u = info["formats"].as_array()?.iter().find_map(|f| f["url"].as_str())?;
    let i = u.find("expire=")? + 7;
    u[i..].split(['&', '/']).next()?.parse().ok()
}

/// Extractions récentes gardées sur disque (`extraits.json`) : une vidéo déjà préparée démarre
/// sans nouvelle extraction, même après un redémarrage (adresses valables ~6 h).
pub fn load_extracts() -> Vec<(String, serde_json::Value)> {
    let Ok(b) = std::fs::read(dir().join("extraits.json")) else { return Vec::new() };
    let v: Vec<(String, serde_json::Value)> = serde_json::from_slice(&b).unwrap_or_default();
    // encore valables au moins 30 min
    v.into_iter().filter(|(_, i)| extract_expiry(i).is_some_and(|e| e > now() + 1800)).collect()
}

pub fn save_extracts(v: Vec<(String, serde_json::Value)>) {
    std::thread::spawn(move || {
        if let Ok(b) = serde_json::to_vec(&v) {
            let _ = socle::paths::write_atomic(&dir().join("extraits.json"), &b);
        }
    });
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    Max,
    Q2160,
    Q1440,
    Q1080,
    Q720,
}
impl Quality {
    pub fn cap(self) -> u32 {
        match self {
            Quality::Max => 99999,
            Quality::Q2160 => 2160,
            Quality::Q1440 => 1440,
            Quality::Q1080 => 1080,
            Quality::Q720 => 720,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Quality::Max => "Max",
            Quality::Q2160 => "2160p",
            Quality::Q1440 => "1440p",
            Quality::Q1080 => "1080p",
            Quality::Q720 => "720p",
        }
    }
    pub const ALL: [Quality; 5] = [Quality::Max, Quality::Q2160, Quality::Q1440, Quality::Q1080, Quality::Q720];
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub volume: f64,
    pub muted: bool,
    pub quality: Quality,
    pub hide_shorts: bool,
    pub sponsorblock: bool,
    pub sb_categories: Vec<String>,
    pub report_history: bool,
    pub autoplay_next: bool,
    pub window: Option<(f32, f32)>,
    pub mini_rect: Option<(i32, i32, i32, i32)>,
    pub mini_opacity: f32,
    pub side_w: f32,
    /// position de reprise par vidéo : (secondes, durée, horodatage)
    pub positions: HashMap<String, (f64, f64, u64)>,
    pub last_update_check: u64,
    pub speed: f64,
    /// largeur des vignettes de l'accueil (points)
    pub card_w: f32,
    /// aperçu vidéo muet au survol d'une vignette
    pub hover_preview: bool,
    /// masquer des listes les vidéos déjà regardées (≥ 90 %)
    pub hide_watched: bool,
    /// plein écran : les bandes noires prennent les couleurs des bords de l'image
    pub cinema: bool,
    /// vitesse mémorisée par chaîne (podcasts en accéléré, musique à ×1…)
    pub channel_speed: HashMap<String, f64>,
    /// langue des sous-titres voulue (None = désactivés)
    pub subs_lang: Option<String>,
    /// vidéos ouvertes récemment (carte « Reprendre » de l'accueil)
    pub recent: Vec<crate::api::Item>,
    /// onglets ouverts à la fermeture : (vue, vidéo de l'onglet)
    pub tabs: Vec<(crate::app::View, Option<crate::api::Item>)>,
    pub tab: usize,
    /// recherches passées (autocomplétion du socle : fréquence + récence)
    pub search_history: socle::autocomplete::History,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark: true,
            volume: 0.8,
            muted: false,
            quality: Quality::Max,
            hide_shorts: true,
            sponsorblock: true,
            sb_categories: vec!["sponsor".into(), "selfpromo".into(), "interaction".into()],
            report_history: true,
            autoplay_next: false,
            window: None,
            mini_rect: None,
            mini_opacity: 1.0,
            side_w: 210.0,
            positions: HashMap::new(),
            last_update_check: 0,
            speed: 1.0,
            card_w: 360.0,
            hover_preview: true,
            hide_watched: false,
            cinema: true,
            channel_speed: HashMap::new(),
            subs_lang: None,
            recent: Vec::new(),
            tabs: Vec::new(),
            tab: 0,
            search_history: Default::default(),
        }
    }
}

impl Settings {
    /// Réglages : `%APPDATA%\Tube\settings.json` (convention commune, via le socle). L'ancien
    /// fichier de `%LOCALAPPDATA%\Tube` est repris une fois. En test : dossier de test.
    pub fn load() -> Settings {
        if test_mode() {
            return socle::settings::load_from(&dir().join("settings.json"));
        }
        if !socle::settings::path("Tube").exists() {
            let old = real_dir().join("settings.json");
            if old.exists() {
                let s: Settings = socle::settings::load_from(&old);
                let _ = socle::settings::save("Tube", &s);
                let _ = std::fs::rename(&old, real_dir().join("settings.json.migre"));
                return s;
            }
        }
        socle::settings::load("Tube")
    }
    pub fn save(&mut self) {
        // ne garder que les 2000 reprises les plus récentes
        if self.positions.len() > 2000 {
            let mut v: Vec<(String, u64)> = self.positions.iter().map(|(k, p)| (k.clone(), p.2)).collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            for (k, _) in v.into_iter().skip(2000) {
                self.positions.remove(&k);
            }
        }
        // écriture atomique (socle)
        let r = if test_mode() { socle::settings::save_to(&dir().join("settings.json"), self) } else { socle::settings::save("Tube", self) };
        if let Err(e) = r {
            log(&format!("réglages non enregistrés : {e}"));
        }
    }
}

/// Journal de diagnostic (%LOCALAPPDATA%\Tube\tube.log), limité à ~1 Mo.
pub fn log(msg: &str) {
    use std::io::Write;
    let p = dir().join("tube.log");
    if std::fs::metadata(&p).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir().join("tube.old.log"));
    }
    let new = !p.exists();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        if new {
            // BOM UTF-8 : le journal s'ouvre avec les bons accents dans les éditeurs Windows
            let _ = f.write_all("\u{feff}".as_bytes());
        }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100_000).unwrap_or(0);
        let _ = writeln!(f, "[{:>6}.{:03}] {msg}", ms / 1000, ms % 1000);
    }
}
