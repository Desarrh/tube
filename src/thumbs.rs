//! Miniatures : téléchargées et décodées par 6 fils (les plus récemment demandées d'abord).
//! Chaque miniature est recadrée en 16:9 puis réduite à la largeur EXACTE d'affichage en pixels :
//! le moteur de rendu logiciel la recopie alors telle quelle au lieu de la ré-échantillonner
//! pixel par pixel à chaque image — c'est ce qui rend le défilement fluide sur un grand écran.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

const CACHE_BYTES: usize = 80 << 20;

pub enum Slot {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

type Key = (String, u32);

struct Shared {
    queue: Mutex<VecDeque<Key>>,
    cv: Condvar,
    done: Mutex<Vec<(Key, Option<egui::ColorImage>)>>,
    /// Images encodées (JPEG/WebP, ~30 Ko) : un changement de taille ne retélécharge rien.
    /// (contenu, ordre d'arrivée, total en octets)
    bytes: Mutex<(HashMap<String, Arc<Vec<u8>>>, VecDeque<String>, usize)>,
}

pub struct Thumbs {
    sh: Arc<Shared>,
    map: HashMap<Key, (Slot, u64)>,
    /// dernière taille prête par URL (affichée en attendant la bonne)
    any: HashMap<String, u32>,
    frame: u64,
}

fn decode(b: &[u8], w: u32) -> Option<egui::ColorImage> {
    let i = image::load_from_memory(b).ok()?;
    let i = if w == 0 {
        if i.width() > 1280 { i.resize(1280, 1280 * i.height() / i.width().max(1), image::imageops::FilterType::Triangle) } else { i }
    } else {
        // 4:3 à bandes noires (hqdefault) : on garde la bande 16:9 centrale
        let (iw, ih) = (i.width(), i.height());
        let i = if (iw as f32 / ih.max(1) as f32) < 1.5 {
            let h = (iw * 9 / 16).min(ih);
            i.crop_imm(0, (ih - h) / 2, iw, h)
        } else {
            i
        };
        // +1 pixel en largeur et en hauteur : le chemin de recopie directe du rendu logiciel
        // exige que la zone dessinée s'arrête avant le bord de la texture
        let h = ((w as f32) * 9.0 / 16.0).round() as u32;
        i.resize_exact(w + 1, h.max(1) + 1, image::imageops::FilterType::Triangle)
    };
    let rgba = i.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
}

impl Thumbs {
    pub fn new(api: Arc<crate::api::Api>, ctx: egui::Context) -> Thumbs {
        let sh = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            done: Mutex::new(Vec::new()),
            bytes: Mutex::new((HashMap::new(), VecDeque::new(), 0)),
        });
        for _ in 0..6 {
            let sh = sh.clone();
            let api = api.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || loop {
                let (url, w) = {
                    let mut q = sh.queue.lock().unwrap();
                    loop {
                        if let Some(u) = q.pop_back() {
                            break u;
                        }
                        q = sh.cv.wait(q).unwrap();
                    }
                };
                let cached = sh.bytes.lock().unwrap().0.get(&url).cloned();
                let bytes = cached.or_else(|| {
                    let b = Arc::new(api.get_bytes(&url).ok()?);
                    let mut c = sh.bytes.lock().unwrap();
                    if let Some(prev) = c.0.insert(url.clone(), b.clone()) {
                        c.2 -= prev.len();
                    } else {
                        c.1.push_back(url.clone());
                    }
                    c.2 += b.len();
                    // ~80 Mo de fichiers d'images au plus, les plus anciens évincés
                    while c.2 > CACHE_BYTES {
                        let Some(old) = c.1.pop_front() else { break };
                        if let Some(x) = c.0.remove(&old) {
                            c.2 -= x.len();
                        }
                    }
                    Some(b)
                });
                let img = bytes.and_then(|b| decode(&b, w));
                sh.done.lock().unwrap().push(((url, w), img));
                // regroupe les miniatures arrivées ensemble en un seul rafraîchissement
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            });
        }
        Thumbs { sh, map: HashMap::new(), any: HashMap::new(), frame: 0 }
    }

    /// À appeler une fois par image : intègre les miniatures prêtes, purge le cache.
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let done: Vec<_> = std::mem::take(&mut *self.sh.done.lock().unwrap());
        for (key, img) in done {
            if !self.map.contains_key(&key) {
                continue; // demande abandonnée entre-temps
            }
            let slot = match img {
                Some(i) => {
                    if key.1 > 0 {
                        self.any.insert(key.0.clone(), key.1);
                    }
                    Slot::Ready(ctx.load_texture(format!("{}#{}", key.0, key.1), i, egui::TextureOptions::LINEAR))
                }
                None => Slot::Failed,
            };
            let f = self.frame;
            self.map.insert(key, (slot, f));
        }
        // ~250 textures au plus : on évince les moins récemment affichées
        if self.map.len() > 400 {
            let mut v: Vec<(Key, u64)> = self.map.iter().map(|(k, s)| (k.clone(), s.1)).collect();
            v.sort_by_key(|x| x.1);
            for (k, _) in v.into_iter().take(80) {
                if self.any.get(&k.0) == Some(&k.1) {
                    self.any.remove(&k.0);
                }
                self.map.remove(&k);
            }
        }
    }

    /// Miniature à `w` pixels de large (0 = taille d'origine, non recadrée).
    /// Renvoie (texture, taille exacte ?) ; en attendant la bonne taille, une autre déjà prête.
    pub fn get(&mut self, url: &str, w: u32) -> Option<(&egui::TextureHandle, bool)> {
        if url.is_empty() {
            return None;
        }
        let frame = self.frame;
        let key = (url.to_string(), w);
        if !self.map.contains_key(&key) {
            self.map.insert(key.clone(), (Slot::Loading, frame));
            let mut q = self.sh.queue.lock().unwrap();
            q.push_back(key.clone());
            // pendant un redimensionnement, les tailles intermédiaires ne servent plus
            while q.len() > 120 {
                if let Some(old) = q.pop_front() {
                    self.map.remove(&old);
                }
            }
            self.sh.cv.notify_one();
        }
        if let Some(e) = self.map.get_mut(&key) {
            e.1 = frame;
        }
        let ready = matches!(self.map.get(&key), Some((Slot::Ready(_), _)));
        let key = if ready || w == 0 {
            key
        } else {
            match self.any.get(url) {
                Some(&aw) => (url.to_string(), aw),
                None => return None,
            }
        };
        let e = self.map.get_mut(&key)?;
        e.1 = frame;
        match &e.0 {
            Slot::Ready(t) => Some((t, ready || w == 0)),
            _ => None,
        }
    }
}
