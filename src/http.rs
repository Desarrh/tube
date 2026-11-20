//! Flux d'octets HTTP pour Media Foundation, adapté à googlevideo.
//! Les serveurs de YouTube brident les requêtes « ouvertes » (≈ 70 Ko/s) mais servent vite les
//! morceaux bornés : on lit donc par blocs (paramètre `&range=a-b`), avec préchargement en
//! arrière-plan et un cache limité. Utilisé à la place du lecteur HTTP intégré à Windows.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use windows::Win32::Foundation::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::core::*;

const CH: u64 = 512 << 10; // taille d'un bloc (petit : après un saut, la lecture repart dès le premier reçu)
const FETCH: u64 = 4; // blocs par requête (googlevideo bride au hasard chaque requête : plusieurs petites en parallèle)
const PARALLEL: u8 = 3; // requêtes de préchargement simultanées par flux
const AHEAD: u64 = 48; // blocs préchargés devant la lecture (24 Mo)

fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| {
        ureq::Agent::config_builder()
            // délais courts : une requête figée ou bridée est abandonnée et relancée (nouvelle
            // connexion) ; la durée totale de chaque requête dépend de sa taille (voir stream_range)
            .timeout_connect(Some(std::time::Duration::from_secs(3)))
            .timeout_recv_response(Some(std::time::Duration::from_secs(4)))
            .user_agent(crate::api::UA)
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Débit mesuré (octets/s par connexion, moyenne glissante) et instant de la mesure :
/// sert à choisir une qualité que la connexion peut tenir.
static SPEED: Mutex<Option<(f64, std::time::Instant)>> = Mutex::new(None);

/// Octets reçus récemment (toutes connexions) : débit réel total.
static RECV: Mutex<Vec<(std::time::Instant, usize)>> = Mutex::new(Vec::new());

fn note_recv(n: usize) {
    let mut v = RECV.lock().unwrap();
    v.push((std::time::Instant::now(), n));
    if v.len() > 4096 {
        v.drain(..2048);
    }
}

/// Débit total des 5 dernières secondes (octets/s).
pub fn recent_rate() -> f64 {
    let v = RECV.lock().unwrap();
    v.iter().filter(|x| x.0.elapsed().as_secs_f64() < 5.0).map(|x| x.1).sum::<usize>() as f64 / 5.0
}

/// Débit total estimé (plusieurs connexions en parallèle) si mesuré il y a moins de 2 min.
pub fn speed() -> Option<f64> {
    SPEED.lock().unwrap().filter(|s| s.1.elapsed().as_secs() < 120).map(|s| s.0 * PARALLEL as f64 * 0.8)
}

fn record_speed(bytes: usize, secs: f64) {
    if secs < 0.05 || bytes < 256 << 10 {
        return;
    }
    let v = bytes as f64 / secs;
    let mut s = SPEED.lock().unwrap();
    let n = match *s {
        Some((old, t)) if t.elapsed().as_secs() < 120 => old * 0.7 + v * 0.3,
        _ => v,
    };
    *s = Some((n, std::time::Instant::now()));
}

struct State {
    chunks: HashMap<u64, Arc<Vec<u8>>>,
    inflight: HashSet<u64>,
    last: u64,
    failed: Option<String>,
    /// fils de préchargement en cours (2 au plus : deux connexions tiennent mieux le débit de la 4K)
    prefetching: u8,
}

pub struct Remote {
    pub url: String,
    pub len: u64,
    max_chunks: usize,
    born: std::time::Instant,
    st: Mutex<State>,
    cv: Condvar,
    /// MP4 fragmenté : index « mfra » synthétisé (servi après la fin réelle du fichier)
    tail: OnceLock<Vec<u8>>,
}

/// Flux « réchauffés » au survol d'une vignette (premiers blocs déjà téléchargés) : le clic
/// les reprend tels quels au lieu de tout recommencer.
fn warm() -> &'static Mutex<Vec<(String, Arc<Remote>, std::time::Instant)>> {
    static W: OnceLock<Mutex<Vec<(String, Arc<Remote>, std::time::Instant)>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// Ouvre `url` en arrière-plan et télécharge son début, pour un démarrage quasi immédiat.
pub fn prewarm(url: &str) {
    if url.is_empty() || warm().lock().unwrap().iter().any(|w| w.0 == url) {
        return;
    }
    let url = url.to_string();
    std::thread::spawn(move || {
        if let Ok(r) = Remote::open_fresh(&url, 48) {
            let _ = r.chunk(0);
            let mut w = warm().lock().unwrap();
            w.retain(|x| x.2.elapsed().as_secs() < 120);
            w.push((url, r, std::time::Instant::now()));
            while w.len() > 6 {
                w.remove(0);
            }
        }
    });
}

impl Remote {
    pub fn open(url: &str, max_mb: usize) -> std::result::Result<Arc<Remote>, String> {
        {
            let mut w = warm().lock().unwrap();
            if let Some(i) = w.iter().position(|x| x.0 == url && x.2.elapsed().as_secs() < 120) {
                return Ok(w.remove(i).1);
            }
        }
        Remote::open_fresh(url, max_mb)
    }

    fn open_fresh(url: &str, max_mb: usize) -> std::result::Result<Arc<Remote>, String> {
        let len = url
            .split(['?', '&'])
            .find_map(|p| p.strip_prefix("clen="))
            .and_then(|v| v.parse::<u64>().ok())
            .map(Ok)
            .unwrap_or_else(|| probe_len(url))?;
        let r = Arc::new(Remote {
            url: url.to_string(),
            len,
            max_chunks: (max_mb * 2).max(16),
            born: std::time::Instant::now(),
            st: Mutex::new(State { chunks: HashMap::new(), inflight: HashSet::new(), last: 0, failed: None, prefetching: 0 }),
            cv: Condvar::new(),
            tail: OnceLock::new(),
        });
        // Les MP4 de YouTube n'ont pas d'index « mfra » : sans lui, Media Foundation parcourt
        // tous les fragments du fichier avant de lire. On le reconstruit depuis le « sidx » de tête.
        if url.contains("mime=video%2Fmp4") || url.contains("mime=audio%2Fmp4") {
            let head = r.chunk(0)?;
            match crate::mp4::mfra_from_head(&head) {
                Some(t) => {
                    let _ = r.tail.set(t);
                }
                None => crate::store::log("MP4 sans sidx lisible : pas d'index synthétisé"),
            }
        }
        Ok(r)
    }

    /// Taille vue par Media Foundation (fichier + index synthétisé).
    pub fn total(&self) -> u64 {
        self.len + self.tail.get().map(|t| t.len() as u64).unwrap_or(0)
    }

    /// Télécharge les blocs [first, first+n) (ceux qui manquent) en une requête. Chaque bloc est
    /// rendu disponible DÈS qu'il est arrivé (la lecture n'attend pas la fin de la requête) ;
    /// une requête figée est abandonnée vite et seuls les blocs manquants sont redemandés.
    fn fetch(&self, first: u64, n: u64) -> std::result::Result<(), String> {
        let last_chunk = (self.len.max(1) - 1) / CH;
        let mut todo: Vec<u64> = {
            let mut st = self.st.lock().unwrap();
            let v: Vec<u64> = (first..(first + n).min(last_chunk + 1)).take_while(|c| !st.chunks.contains_key(c) && !st.inflight.contains(c)).collect();
            for c in &v {
                st.inflight.insert(*c);
            }
            v
        };
        let mut attempt = 0;
        while !todo.is_empty() {
            let before = todo.len();
            let res = self.stream_range(&mut todo, attempt);
            // des blocs sont arrivés : ce n'est pas un échec (réponse plafonnée par le serveur,
            // connexion lente coupée) — on redemande simplement la suite
            if todo.len() < before && res.is_err() && !res.as_ref().err().is_some_and(|e| e.starts_with("HTTP")) {
                attempt = 0;
                continue;
            }
            match res {
                Ok(()) => {}
                // une URL toute neuve (client TV) répond 403 pendant 1 à 2 s : on patiente ;
                // plus tard, un 403 signifie une URL expirée : inutile d'insister.
                Err(e) if e.starts_with("HTTP 4") && self.born.elapsed().as_secs() <= 10 && attempt < 8 => {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                }
                Err(e) => {
                    if !e.starts_with("connexion lente") || attempt >= 2 {
                        crate::store::log(&format!("blocs {:?} : {e} (essai {})", todo, attempt + 1));
                    }
                    if attempt >= 6 || e.starts_with("HTTP 4") {
                        let mut st = self.st.lock().unwrap();
                        for c in &todo {
                            st.inflight.remove(c);
                        }
                        st.failed = Some(e.clone());
                        self.cv.notify_all();
                        return Err(e);
                    }
                }
            }
            attempt += 1;
        }
        Ok(())
    }

    fn block_len(&self, c: u64) -> usize {
        (self.len - c * CH).min(CH) as usize
    }

    /// Une requête pour les blocs `todo` (consécutifs) ; chaque bloc complet est rangé au fil de
    /// l'eau et retiré de `todo`.
    fn stream_range(&self, todo: &mut Vec<u64>, attempt: u32) -> std::result::Result<(), String> {
        use std::io::Read;
        let a = todo[0] * CH;
        let b = ((todo[todo.len() - 1] + 1) * CH).min(self.len) - 1;
        let url = format!("{}&range={a}-{b}", self.url);
        // durée maximale : 4 s + taille à 500 Ko/s. À partir du 3e essai, on laisse finir une requête
        // lente mais qui avance (≥ 80 Ko/s) plutôt que de la relancer en boucle
        let rate = if attempt < 2 { 500_000.0 } else { 80_000.0 };
        let limit = std::time::Duration::from_secs_f64(4.0 + (b - a + 1) as f64 / rate);
        let req = agent().get(&url).config().timeout_global(Some(limit)).build();
        let mut r = req.call().map_err(|e| e.to_string())?;
        let code = r.status().as_u16();
        if code >= 400 {
            let body = r.body_mut().read_to_string().unwrap_or_default();
            if std::env::var_os("TUBE_DUMP_URL").is_some() {
                let _ = std::fs::write(crate::store::dir().join("url403.txt"), &url);
            }
            let c = self.url.split(['?', '&']).find(|p| p.starts_with("c=")).unwrap_or("");
            let itag = self.url.split(['?', '&']).find(|p| p.starts_with("itag=")).unwrap_or("");
            return Err(format!("HTTP {code} ({c} {itag} plage {a}-{b}) {}", body.chars().take(120).collect::<String>()));
        }
        let mut rd = r.body_mut().with_config().limit(64 << 20).reader();
        let mut cur: Vec<u8> = Vec::with_capacity(CH as usize);
        let mut tmp = vec![0u8; 256 << 10];
        let t0 = std::time::Instant::now();
        let mut got = 0usize;
        while let Some(&c) = todo.first() {
            let want = self.block_len(c);
            let m = (want - cur.len()).min(tmp.len());
            let n = rd.read(&mut tmp[..m]).map_err(|e| e.to_string())?;
            if n == 0 {
                // googlevideo plafonne parfois une réponse (~1 Mo) : la suite sera redemandée
                return Err(format!("réponse plafonnée (bloc {c})"));
            }
            got += n;
            note_recv(n);
            // requête bridée (googlevideo en bride une partie au hasard) : on la relance tôt, la
            // suivante est souvent rapide ; à partir du 3e essai on garde ce qu'on a
            let el = t0.elapsed().as_secs_f64();
            if attempt < 2 && el > 1.2 && (got as f64 / el) < 300_000.0 {
                return Err(format!("connexion lente ({:.0} Ko/s)", got as f64 / el / 1024.0));
            }
            cur.extend_from_slice(&tmp[..n]);
            if cur.len() == want {
                record_speed(got, t0.elapsed().as_secs_f64());
                self.store_one(c, std::mem::replace(&mut cur, Vec::with_capacity(CH as usize)));
                todo.remove(0);
            }
        }
        Ok(())
    }

    fn store_one(&self, c: u64, data: Vec<u8>) {
        let mut st = self.st.lock().unwrap();
        st.inflight.remove(&c);
        st.chunks.insert(c, Arc::new(data));
        // cache borné : on évince les blocs les plus éloignés de la lecture
        if st.chunks.len() > self.max_chunks {
            let cur = st.last / CH;
            // jamais le bloc qu'on vient de recevoir (sinon boucle après un saut)
            let mut keys: Vec<u64> = st.chunks.keys().copied().filter(|k| *k != c).collect();
            keys.sort_by_key(|k| std::cmp::Reverse(if *k + 2 < cur { (cur - k) * 4 } else { k.abs_diff(cur) }));
            let drop = st.chunks.len() - self.max_chunks;
            for k in keys.into_iter().take(drop) {
                st.chunks.remove(&k);
            }
        }
        self.cv.notify_all();
    }

    fn chunk(&self, c: u64) -> std::result::Result<Arc<Vec<u8>>, String> {
        loop {
            {
                let mut st = self.st.lock().unwrap();
                if let Some(d) = st.chunks.get(&c) {
                    return Ok(d.clone());
                }
                if st.inflight.contains(&c) {
                    st = self.cv.wait(st).unwrap();
                    if let Some(d) = st.chunks.get(&c) {
                        return Ok(d.clone());
                    }
                    if let Some(e) = st.failed.take() {
                        return Err(e);
                    }
                    continue;
                }
            }
            // début du fichier ou saut : requête courte (premiers octets plus tôt), le préchargement suit
            let jump = { let st = self.st.lock().unwrap(); (st.last / CH).abs_diff(c) > 1 };
            self.fetch(c, if c == 0 || jump { 2 } else { FETCH })?;
        }
    }

    pub fn read_at(self: &Arc<Self>, pos: u64, buf: &mut [u8]) -> std::result::Result<usize, String> {
        // lecture à cheval sur la fin du fichier et l'index synthétisé : on enchaîne les deux
        if pos < self.len && pos + buf.len() as u64 > self.len && self.tail.get().is_some() {
            let k = (self.len - pos) as usize;
            let (a, b) = buf.split_at_mut(k);
            let n = self.read_file(pos, a)?;
            return Ok(if n == k { n + self.read_at(self.len, b)? } else { n });
        }
        if pos >= self.len {
            let Some(t) = self.tail.get() else { return Ok(0) };
            let off = (pos - self.len) as usize;
            if off >= t.len() {
                return Ok(0);
            }
            let n = (t.len() - off).min(buf.len());
            buf[..n].copy_from_slice(&t[off..off + n]);
            return Ok(n);
        }
        self.read_file(pos, buf)
    }

    fn read_file(self: &Arc<Self>, pos: u64, buf: &mut [u8]) -> std::result::Result<usize, String> {
        let want = (buf.len() as u64).min(self.len - pos) as usize;
        // la position de lecture guide l'éviction et le préchargement : à jour avant de télécharger
        self.st.lock().unwrap().last = pos;
        let mut done = 0;
        while done < want {
            let p = pos + done as u64;
            let d = self.chunk(p / CH)?;
            let off = (p % CH) as usize;
            let n = (d.len() - off).min(want - done);
            buf[done..done + n].copy_from_slice(&d[off..off + n]);
            done += n;
        }
        self.st.lock().unwrap().last = pos + done as u64;
        self.prefetch();
        Ok(done)
    }

    /// Garde AHEAD blocs d'avance, dans un fil à part (un seul à la fois par flux).
    fn prefetch(self: &Arc<Self>) {
        {
            let mut st = self.st.lock().unwrap();
            if st.prefetching >= PARALLEL {
                return;
            }
            st.prefetching += 1;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            loop {
                let next = {
                    let st = me.st.lock().unwrap();
                    let cur = st.last / CH;
                    let end = (cur + AHEAD).min((me.len.max(1) - 1) / CH + 1);
                    (cur..end).find(|c| !st.chunks.contains_key(c) && !st.inflight.contains(c))
                };
                match next {
                    Some(c) if me.fetch(c, FETCH).is_ok() => {}
                    _ => break,
                }
            }
            me.st.lock().unwrap().prefetching -= 1;
        });
    }
}

fn probe_len(url: &str) -> std::result::Result<u64, String> {
    let r = agent().get(url).header("Range", "bytes=0-0").call().map_err(|e| e.to_string())?;
    r.headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("taille inconnue (HTTP {})", r.status()))
}

/// L'objet COM vu par Media Foundation.
#[implement(IMFByteStream)]
pub struct ByteStream {
    r: Arc<Remote>,
    pos: Mutex<u64>,
    results: Arc<Mutex<HashMap<usize, u32>>>,
}

impl ByteStream {
    pub fn new(r: Arc<Remote>) -> IMFByteStream {
        ByteStream { r, pos: Mutex::new(0), results: Arc::new(Mutex::new(HashMap::new())) }.into()
    }
}

impl IMFByteStream_Impl for ByteStream_Impl {
    fn GetCapabilities(&self) -> Result<u32> {
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_SEEKABLE)
    }
    fn GetLength(&self) -> Result<u64> {
        Ok(self.r.total())
    }
    fn SetLength(&self, _l: u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCurrentPosition(&self) -> Result<u64> {
        Ok(*self.pos.lock().unwrap())
    }
    fn SetCurrentPosition(&self, p: u64) -> Result<()> {
        *self.pos.lock().unwrap() = p.min(self.r.total());
        Ok(())
    }
    fn IsEndOfStream(&self) -> Result<BOOL> {
        Ok((*self.pos.lock().unwrap() >= self.r.total()).into())
    }
    fn Read(&self, pb: *mut u8, cb: u32, pcbread: *mut u32) -> Result<()> {
        let pos = *self.pos.lock().unwrap();
        if std::env::var_os("TUBE_LOG_READS").is_some() {
            crate::store::log(&format!("lecture sync : {pos} +{cb}"));
        }
        let buf = unsafe { std::slice::from_raw_parts_mut(pb, cb as usize) };
        let n = self.r.read_at(pos, buf).map_err(|e| {
            crate::store::log(&format!("flux HTTP : {e}"));
            Error::from(E_FAIL)
        })?;
        *self.pos.lock().unwrap() = pos + n as u64;
        unsafe { *pcbread = n as u32 };
        Ok(())
    }
    fn BeginRead(&self, pb: *mut u8, cb: u32, cb_obj: Ref<IMFAsyncCallback>, st: Ref<IUnknown>) -> Result<()> {
        let res = unsafe { MFCreateAsyncResult(None, cb_obj.as_ref(), st.as_ref())? };
        let pos = *self.pos.lock().unwrap();
        if std::env::var_os("TUBE_LOG_READS").is_some() {
            crate::store::log(&format!("lecture {} : {pos} +{cb} (taille {})", if self.r.url.contains("mime=video") { "vidéo" } else { "son" }, self.r.len));
        }
        // lecture dans un fil : ne bloque jamais les files de travail de Media Foundation
        let r = self.r.clone();
        let results = self.results.clone();
        let key = res.as_raw() as usize;
        let raw = res.into_raw() as usize;
        let pb = pb as usize;
        // la position avance tout de suite (lectures séquentielles de la source)
        let n_est = (cb as u64).min(self.r.total().saturating_sub(pos));
        *self.pos.lock().unwrap() = pos + n_est;
        std::thread::spawn(move || {
            // SAFETY : le tampon reste valide jusqu'à EndRead (contrat de IMFByteStream::BeginRead).
            let buf = unsafe { std::slice::from_raw_parts_mut(pb as *mut u8, cb as usize) };
            let res: IMFAsyncResult = unsafe { IMFAsyncResult::from_raw(raw as *mut _) };
            let t0 = std::time::Instant::now();
            let out = r.read_at(pos, buf);
            if std::env::var_os("TUBE_LOG_READS").is_some() {
                crate::store::log(&format!("  fin lecture {pos} : {:?} en {} ms", out.as_ref().map(|n| *n), t0.elapsed().as_millis()));
            }
            match out {
                Ok(n) => {
                    results.lock().unwrap().insert(key, n as u32);
                    unsafe {
                        let _ = res.SetStatus(S_OK);
                    }
                }
                Err(e) => {
                    crate::store::log(&format!("flux HTTP : {e}"));
                    results.lock().unwrap().insert(key, 0);
                    unsafe {
                        let _ = res.SetStatus(E_FAIL);
                    }
                }
            }
            unsafe {
                let _ = MFInvokeCallback(&res);
            }
        });
        Ok(())
    }
    fn EndRead(&self, r: Ref<IMFAsyncResult>) -> Result<u32> {
        let r = r.ok()?;
        let key = r.as_raw() as usize;
        let n = self.results.lock().unwrap().remove(&key).unwrap_or(0);
        unsafe { r.GetStatus()? };
        Ok(n)
    }
    fn Write(&self, _pb: *const u8, _cb: u32) -> Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn BeginWrite(&self, _pb: *const u8, _cb: u32, _c: Ref<IMFAsyncCallback>, _s: Ref<IUnknown>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn EndWrite(&self, _r: Ref<IMFAsyncResult>) -> Result<u32> {
        Err(E_NOTIMPL.into())
    }
    fn Seek(&self, origin: MFBYTESTREAM_SEEK_ORIGIN, off: i64, _flags: u32) -> Result<u64> {
        if std::env::var_os("TUBE_LOG_READS").is_some() {
            crate::store::log(&format!("seek {} {off}", origin.0));
        }
        let mut p = self.pos.lock().unwrap();
        let base = if origin == msoCurrent { *p as i64 } else { 0 };
        *p = (base + off).clamp(0, self.r.total() as i64) as u64;
        Ok(*p)
    }
    fn Flush(&self) -> Result<()> {
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        Ok(())
    }
}
