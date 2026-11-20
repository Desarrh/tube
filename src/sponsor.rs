//! SponsorBlock : segments à passer (sponsor, autopromo, rappel d'abonnement…).
//! Requête par préfixe de hachage (4 caractères du SHA-256) : le serveur ne sait pas quelle vidéo on regarde.
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct Seg {
    pub start: f64,
    pub end: f64,
    pub category: String,
}

pub fn label(cat: &str) -> &'static str {
    match cat {
        "sponsor" => "sponsor",
        "selfpromo" => "autopromo",
        "interaction" => "rappel d'abonnement",
        "intro" => "intro",
        "outro" => "générique de fin",
        "preview" => "aperçu",
        "filler" => "digression",
        "music_offtopic" => "passage hors musique",
        _ => "segment",
    }
}

pub const CATEGORIES: [&str; 8] = ["sponsor", "selfpromo", "interaction", "intro", "outro", "preview", "filler", "music_offtopic"];

pub fn fetch(api: &crate::api::Api, video_id: &str, cats: &[String]) -> Vec<Seg> {
    let hash = format!("{:x}", Sha256::digest(video_id.as_bytes()));
    let cats_json = serde_json::to_string(cats).unwrap_or_else(|_| "[]".into());
    let url = format!("https://sponsor.ajay.app/api/skipSegments/{}?categories={}&actionTypes=%5B%22skip%22%5D", &hash[..4], crate::api::urlenc(&cats_json));
    let Ok(v) = api.get_json(&url) else { return vec![] };
    let mut out = Vec::new();
    for e in v.as_array().into_iter().flatten() {
        if e["videoID"].as_str() != Some(video_id) {
            continue;
        }
        for s in e["segments"].as_array().into_iter().flatten() {
            let a = &s["segment"];
            if let (Some(st), Some(en)) = (a[0].as_f64(), a[1].as_f64()) {
                if en - st > 0.5 {
                    out.push(Seg { start: st, end: en, category: s["category"].as_str().unwrap_or("").to_string() });
                }
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}
