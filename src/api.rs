//! API interne de YouTube (InnerTube) avec les cookies du compte : accueil recommandé,
//! abonnements, « À regarder plus tard », playlists, historique, recherche, suggestions,
//! et remontée de l'historique de visionnage (pings playbackTracking, comme le site).
use crate::cookies::Jar;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const ORIGIN: &str = "https://www.youtube.com";
pub const CLIENT_VERSION: &str = "2.20261117.01.00";
pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum Kind {
    #[default]
    Video,
    Playlist,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    pub channel: String,
    pub meta: String,
    pub duration: String,
    pub thumb: String,
    /// 0..1 : déjà vu en partie (barre rouge de YouTube)
    pub progress: f32,
    pub live: bool,
    pub short: bool,
    /// identifiant de la chaîne (UC…), pour ouvrir sa page
    pub channel_id: String,
    /// jetons de retour « Pas intéressé » / « Ne pas recommander la chaîne »
    #[serde(skip)]
    pub not_interested: String,
    #[serde(skip)]
    pub no_channel: String,
}

#[derive(Default, Clone)]
pub struct Page {
    pub items: Vec<Item>,
    pub cont: Option<String>,
    pub logged_in: Option<bool>,
}

#[derive(Clone, Default)]
pub struct Tracking {
    pub playback: String,
    pub watchtime: String,
}

pub struct Api {
    pub jar: Arc<Mutex<Jar>>,
    agent: ureq::Agent,
}

fn text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(s) = v["simpleText"].as_str() {
        return s.to_string();
    }
    if let Some(s) = v["content"].as_str() {
        return s.to_string();
    }
    if let Some(r) = v["runs"].as_array() {
        return r.iter().filter_map(|x| x["text"].as_str()).collect();
    }
    String::new()
}

/// Première liste « sources »/« thumbnails » trouvée : on garde la plus grande image ≤ 720 px de large.
fn thumb(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            for key in ["sources", "thumbnails"] {
                if let Some(a) = m.get(key).and_then(|x| x.as_array()) {
                    let mut best: Option<(u64, &str)> = None;
                    for s in a {
                        let Some(u) = s["url"].as_str() else { continue };
                        if !u.contains("ytimg") {
                            continue;
                        }
                        let w = s["width"].as_u64().unwrap_or(1);
                        if best.is_none_or(|b| (w <= 720 && w > b.0) || b.0 > 720 && w < b.0) {
                            best = Some((w, u));
                        }
                    }
                    if let Some((_, u)) = best {
                        let u = if u.starts_with("//") { format!("https:{u}") } else { u.to_string() };
                        return Some(u);
                    }
                }
            }
            m.values().find_map(thumb)
        }
        Value::Array(a) => a.iter().find_map(thumb),
        _ => None,
    }
}

fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|x| find(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find(x, key)),
        _ => None,
    }
}

/// Premier identifiant de chaîne (UC…) d'un lien de navigation.
fn channel_id(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            if let Some(id) = m.get("browseEndpoint").and_then(|b| b["browseId"].as_str()) {
                if id.starts_with("UC") {
                    return id.to_string();
                }
            }
            m.values().map(channel_id).find(|s| !s.is_empty()).unwrap_or_default()
        }
        Value::Array(a) => a.iter().map(channel_id).find(|s| !s.is_empty()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Jetons « Pas intéressé » et « Ne pas recommander la chaîne » du menu ⋮ d'une carte.
fn feedback(v: &Value, out: &mut (String, String)) {
    match v {
        Value::Object(m) => {
            let entry = m.get("listItemViewModel").or_else(|| m.get("menuServiceItemRenderer"));
            if let Some(e) = entry {
                if let Some(tok) = find(e, "feedbackToken").and_then(|t| t.as_str()) {
                    let s = e.to_string();
                    if s.contains("\"HIDE\"") || s.contains("NOT_INTERESTED") {
                        out.0 = tok.to_string();
                    } else if s.contains("\"REMOVE\"") || s.contains("NOT_INTERESTED_CHANNEL") || s.contains("DONT_RECOMMEND") {
                        out.1 = tok.to_string();
                    }
                }
                return;
            }
            m.values().for_each(|x| feedback(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| feedback(x, out)),
        _ => {}
    }
}

fn lockup(l: &Value) -> Option<Item> {
    let md = &l["metadata"]["lockupMetadataViewModel"];
    if md.is_null() {
        return None; // publicité (feedAdMetadataViewModel) ou carte inconnue
    }
    let kind = match l["contentType"].as_str().unwrap_or("") {
        "LOCKUP_CONTENT_TYPE_VIDEO" => Kind::Video,
        "LOCKUP_CONTENT_TYPE_PLAYLIST" | "LOCKUP_CONTENT_TYPE_PODCAST" => Kind::Playlist,
        _ => return None,
    };
    let rows = md["metadata"]["contentMetadataViewModel"]["metadataRows"].as_array().cloned().unwrap_or_default();
    let row = |i: usize| -> Vec<String> {
        rows.get(i).and_then(|r| r["metadataParts"].as_array()).map(|p| p.iter().map(|x| text(&x["text"])).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    };
    let img = &l["contentImage"];
    let mut duration = String::new();
    let mut live = false;
    if let Some(b) = find(img, "thumbnailBadgeViewModel") {
        duration = b["text"].as_str().unwrap_or("").to_string();
        live = b["badgeStyle"].as_str().is_some_and(|s| s.contains("LIVE"));
    }
    let progress = find(img, "startPercent").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32 / 100.0;
    let (channel, meta) = if kind == Kind::Playlist {
        (row(0).join(" • "), row(1).join(" • "))
    } else {
        (row(0).first().cloned().unwrap_or_default(), row(1).join(" • "))
    };
    let mut fb = (String::new(), String::new());
    feedback(&md["menuButton"], &mut fb);
    Some(Item {
        kind,
        id: l["contentId"].as_str()?.to_string(),
        title: text(&md["title"]),
        channel,
        meta,
        duration,
        thumb: thumb(img).unwrap_or_default(),
        progress,
        live,
        short: false,
        channel_id: channel_id(&md["metadata"]).or_if_empty(|| channel_id(&md["image"])),
        not_interested: fb.0,
        no_channel: fb.1,
    })
}

fn renderer(v: &Value) -> Option<Item> {
    let id = v["videoId"].as_str()?.to_string();
    let mut duration = text(&v["lengthText"]);
    let mut live = false;
    let mut progress = 0.0;
    if let Some(o) = v["thumbnailOverlays"].as_array() {
        for x in o {
            if let Some(t) = x.get("thumbnailOverlayTimeStatusRenderer") {
                if duration.is_empty() {
                    duration = text(&t["text"]);
                }
                live |= t["style"].as_str() == Some("LIVE");
            }
            if let Some(p) = x["thumbnailOverlayResumePlaybackRenderer"]["percentDurationWatched"].as_f64() {
                progress = p as f32 / 100.0;
            }
        }
    }
    let channel = [&v["ownerText"], &v["shortBylineText"], &v["longBylineText"]].iter().map(|x| text(x)).find(|s| !s.is_empty()).unwrap_or_default();
    let meta = [text(&v["viewCountText"]), text(&v["publishedTimeText"]), text(&v["videoInfo"])].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • ");
    let mut fb = (String::new(), String::new());
    feedback(&v["menu"], &mut fb);
    let cid = [&v["ownerText"], &v["shortBylineText"], &v["longBylineText"], &v["channelThumbnailSupportedRenderers"]].iter().map(|x| channel_id(x)).find(|s| !s.is_empty()).unwrap_or_default();
    Some(Item {
        kind: Kind::Video,
        id,
        title: text(&v["title"]),
        channel,
        meta,
        duration,
        thumb: thumb(&v["thumbnail"]).unwrap_or_default(),
        progress,
        live,
        short: false,
        channel_id: cid,
        not_interested: fb.0,
        no_channel: fb.1,
    })
}

fn walk(v: &Value, out: &mut Page) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match k.as_str() {
                    // publicités et blocs sans intérêt
                    "adSlotRenderer" | "promotedSparklesWebRenderer" | "searchPyvRenderer" | "brandVideoShelfRenderer" | "statementBannerRenderer" | "miniGameCardViewModel" | "topbar" | "frameworkUpdates" => {}
                    "lockupViewModel" => out.items.extend(lockup(x)),
                    "videoRenderer" | "gridVideoRenderer" | "playlistVideoRenderer" | "compactVideoRenderer" => out.items.extend(renderer(x)),
                    "shortsLockupViewModel" => {
                        let id = x["onTap"]["innertubeCommand"]["reelWatchEndpoint"]["videoId"].as_str().or_else(|| find(x, "videoId").and_then(|v| v.as_str()));
                        if let Some(id) = id {
                            out.items.push(Item {
                                kind: Kind::Video,
                                id: id.to_string(),
                                title: text(&x["overlayMetadata"]["primaryText"]),
                                channel: String::new(),
                                meta: text(&x["overlayMetadata"]["secondaryText"]),
                                duration: "Short".into(),
                                thumb: thumb(&x["thumbnail"]).unwrap_or_default(),
                                short: true,
                                ..Default::default()
                            });
                        }
                    }
                    "continuationItemRenderer" => {
                        if let Some(t) = find(x, "token").and_then(|t| t.as_str()) {
                            out.cont = Some(t.to_string());
                        }
                    }
                    _ => walk(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

pub fn parse(v: &Value) -> Page {
    let mut p = Page::default();
    walk(v, &mut p);
    let mut seen = std::collections::HashSet::new();
    p.items.retain(|i| !i.id.is_empty() && seen.insert(i.id.clone()));
    p.logged_in = v["responseContext"]["serviceTrackingParams"].as_array().map(|a| {
        a.iter().flat_map(|s| s["params"].as_array().cloned().unwrap_or_default()).any(|p| p["key"] == "logged_in" && p["value"] == "1")
    });
    p
}

impl Api {
    pub fn new(jar: Arc<Mutex<Jar>>) -> Api {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(25)))
            .user_agent(UA)
            .http_status_as_error(false)
            .build()
            .into();
        Api { jar, agent }
    }

    fn absorb(&self, resp: &ureq::http::Response<ureq::Body>) {
        let mut j = self.jar.lock().unwrap();
        for h in resp.headers().get_all("set-cookie") {
            if let Ok(s) = h.to_str() {
                j.set_cookie(s);
            }
        }
        if j.dirty {
            j.save(&crate::store::cookies_path());
        }
    }

    fn headers(&self, req: ureq::RequestBuilder<ureq::typestate::WithBody>) -> ureq::RequestBuilder<ureq::typestate::WithBody> {
        let j = self.jar.lock().unwrap();
        let mut r = req
            .header("Content-Type", "application/json")
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .header("X-Goog-AuthUser", "0")
            .header("X-Youtube-Client-Name", "1")
            .header("X-Youtube-Client-Version", CLIENT_VERSION)
            .header("Cookie", j.header("www.youtube.com"));
        if let Some(a) = j.auth(ORIGIN) {
            r = r.header("Authorization", a).header("X-Youtube-Bootstrap-Logged-In", "true");
        }
        r
    }

    pub fn post(&self, endpoint: &str, mut body: Value) -> Result<Value, String> {
        body["context"] = json!({"client": {"clientName": "WEB", "clientVersion": CLIENT_VERSION, "hl": "fr", "gl": "FR", "timeZone": "Europe/Paris"}});
        let url = format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false");
        let mut resp = self.headers(self.agent.post(&url)).send(body.to_string()).map_err(|e| format!("réseau : {e}"))?;
        self.absorb(&resp);
        let status = resp.status().as_u16();
        let s = resp.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| e.to_string())?;
        if status >= 400 {
            return Err(format!("YouTube a répondu {status}"));
        }
        serde_json::from_str(&s).map_err(|e| e.to_string())
    }

    pub fn browse(&self, browse_id: &str) -> Result<Page, String> {
        self.post("browse", json!({"browseId": browse_id})).map(|v| {
            if let Some(d) = std::env::var_os("TUBE_DUMP_BROWSE") {
                let _ = std::fs::write(std::path::Path::new(&d).join(format!("{browse_id}.json")), serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            parse(&v)
        })
    }
    pub fn browse_more(&self, token: &str) -> Result<Page, String> {
        self.post("browse", json!({"continuation": token})).map(|v| parse(&v))
    }
    /// Onglet « Vidéos » d'une chaîne.
    pub fn channel(&self, id: &str) -> Result<Page, String> {
        self.post("browse", json!({"browseId": id, "params": "EgZ2aWRlb3PyBgQKAjoA"})).map(|v| parse(&v))
    }
    /// « Pas intéressé » / « Ne pas recommander la chaîne » (jeton du menu de la carte).
    pub fn send_feedback(&self, token: &str) -> Result<(), String> {
        let v = self.post("feedback", json!({"feedbackTokens": [token], "isFeedbackTokenUnencrypted": false, "shouldMerge": false}))?;
        if v["feedbackResponses"][0]["isProcessed"].as_bool() == Some(true) { Ok(()) } else { Err("non pris en compte".into()) }
    }
    /// Ajoute une vidéo à « À regarder plus tard ».
    pub fn watch_later(&self, video_id: &str) -> Result<(), String> {
        let v = self.post("browse/edit_playlist", json!({"playlistId": "WL", "actions": [{"addedVideoId": video_id, "action": "ACTION_ADD_VIDEO"}]}))?;
        if v["status"].as_str() == Some("STATUS_SUCCEEDED") { Ok(()) } else { Err(format!("réponse inattendue : {}", v["status"])) }
    }
    pub fn search(&self, q: &str) -> Result<Page, String> {
        self.post("search", json!({"query": q})).map(|v| parse(&v))
    }
    pub fn search_more(&self, token: &str) -> Result<Page, String> {
        self.post("search", json!({"continuation": token})).map(|v| parse(&v))
    }
    /// Suggestions affichées à côté de la vidéo.
    pub fn related(&self, video_id: &str) -> Result<Page, String> {
        let v = self.post("next", json!({"videoId": video_id}))?;
        let sec = find(&v, "secondaryResults").cloned().unwrap_or(Value::Null);
        let mut p = parse(&sec);
        p.items.retain(|i| i.id != video_id);
        Ok(p)
    }
    /// Adresses de suivi de lecture (historique) du lecteur web, avec le compte.
    pub fn tracking(&self, video_id: &str) -> Result<Tracking, String> {
        let v = self.post("player", json!({"videoId": video_id}))?;
        let t = &v["playbackTracking"];
        let get = |k: &str| t[k]["baseUrl"].as_str().unwrap_or("").replace("https://s.youtube.com", ORIGIN);
        let tr = Tracking { playback: get("videostatsPlaybackUrl"), watchtime: get("videostatsWatchtimeUrl") };
        if tr.playback.is_empty() { Err("pas de suivi".into()) } else { Ok(tr) }
    }

    pub fn ping(&self, base: &str, params: &[(&str, String)]) {
        if base.is_empty() {
            return;
        }
        let mut url = base.to_string();
        for (k, v) in params {
            url += if url.contains('?') { "&" } else { "?" };
            url += &format!("{k}={}", urlenc(v));
        }
        let (cookie, auth) = {
            let j = self.jar.lock().unwrap();
            (j.header("www.youtube.com"), j.auth(ORIGIN))
        };
        let mut r = self.agent.get(&url).header("Cookie", cookie).header("Origin", ORIGIN).header("Referer", format!("{ORIGIN}/"));
        if let Some(a) = auth {
            r = r.header("Authorization", a);
        }
        if let Ok(resp) = r.call() {
            self.absorb(&resp);
        }
    }

    pub fn get_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut r = self.agent.get(url).call().map_err(|e| e.to_string())?;
        if r.status().as_u16() >= 400 {
            return Err(format!("HTTP {}", r.status()));
        }
        r.body_mut().with_config().limit(16 << 20).read_to_vec().map_err(|e| e.to_string())
    }

    pub fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut r = self.agent.get(url).call().map_err(|e| e.to_string())?;
        if r.status().as_u16() == 404 {
            return Ok(Value::Array(vec![]));
        }
        let s = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|e| e.to_string())
    }
}

trait OrIfEmpty {
    fn or_if_empty(self, f: impl FnOnce() -> String) -> String;
}
impl OrIfEmpty for String {
    fn or_if_empty(self, f: impl FnOnce() -> String) -> String {
        if self.is_empty() { f() } else { self }
    }
}

pub fn urlenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            _ => o += &format!("%{b:02X}"),
        }
    }
    o
}

/// Identifiant de lecture client (16 caractères), comme le lecteur web.
pub fn cpn() -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) ^ 0x9E3779B97F4A7C15;
    (0..16)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            A[(x % 64) as usize] as char
        })
        .collect()
}
