//! Directs : YouTube les diffuse en HLS (liste de segments MPEG-TS vidéo + son, renouvelée en
//! continu). Un fil télécharge les nouveaux segments au fil de l'eau et les met bout à bout ;
//! Media Foundation lit le tout comme un flux TS sans fin (non déplaçable) via `LiveStream`.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::core::*;

struct State {
    /// octets disponibles ; `base` = position (dans le flux) du premier octet de `buf`
    buf: VecDeque<u8>,
    base: u64,
    ended: bool,
    error: Option<String>,
    /// position de lecture (pour ne jamais jeter ce qui n'a pas encore été lu)
    read: u64,
}

pub struct Live {
    st: Mutex<State>,
    cv: Condvar,
    stop: AtomicBool,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_recv_response(Some(Duration::from_secs(8)))
        .timeout_recv_body(Some(Duration::from_secs(20)))
        .user_agent(crate::api::UA)
        // les adresses des segments sont liées à l'IPv4 de l'extraction (« /ip/… ») : une connexion
        // en IPv6 est refusée (403)
        .ip_family(ureq::config::IpFamily::Ipv4Only)
        .build()
        .into()
}

fn get(a: &ureq::Agent, url: &str) -> std::result::Result<Vec<u8>, String> {
    let mut last = String::new();
    for _ in 0..3 {
        match a.get(url).call() {
            Ok(mut r) => match r.body_mut().with_config().limit(64 << 20).read_to_vec() {
                Ok(v) => return Ok(v),
                Err(e) => last = e.to_string(),
            },
            Err(e) => {
                last = e.to_string();
                if let Some(d) = std::env::var_os("TUBE_DUMP_URL") {
                    let _ = std::fs::write(std::path::Path::new(&d), url);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    Err(last)
}

/// Numéro de segment de départ partagé par la vidéo et le son d'un même direct
/// (ils doivent commencer au même segment pour rester synchrones).
fn start_seq(group: &str, latest: u64, first: u64, back_segs: u64) -> u64 {
    static M: std::sync::OnceLock<Mutex<std::collections::HashMap<String, u64>>> = std::sync::OnceLock::new();
    let mut m = M.get_or_init(Default::default).lock().unwrap();
    *m.entry(group.to_string()).or_insert(latest.saturating_sub(3 + back_segs).max(first))
}

/// Adresse du segment `seq` déduite d'une adresse de segment connue (paramètre « /sq/N/ ») :
/// la liste du client mweb ne couvre que ~30 s, mais les segments plus anciens restent servis.
fn seg_url(template: &str, seq: u64) -> Option<String> {
    let i = template.find("/sq/")? + 4;
    let j = template[i..].find('/').map(|j| i + j)?;
    Some(format!("{}{}{}", &template[..i], seq, &template[j..]))
}

/// Durée (s) que l'on peut remonter dans le direct en cours (fenêtre de la liste HLS).
static WINDOW: Mutex<f64> = Mutex::new(0.0);
pub fn window() -> f64 {
    *WINDOW.lock().unwrap()
}

/// Les segments de son commencent par une balise ID3 (horodatage) : on la retire, le lecteur
/// AAC de Windows attend des trames ADTS bout à bout.
fn strip_id3(b: &[u8]) -> &[u8] {
    let mut b = b;
    while b.len() > 10 && &b[..3] == b"ID3" {
        let n = ((b[6] as usize & 0x7F) << 21) | ((b[7] as usize & 0x7F) << 14) | ((b[8] as usize & 0x7F) << 7) | (b[9] as usize & 0x7F);
        let footer = if b[5] & 0x10 != 0 { 10 } else { 0 };
        b = &b[(10 + n + footer).min(b.len())..];
    }
    b
}

impl Live {
    /// Démarre la lecture de la liste HLS `playlist` (au bord du direct, ~3 segments de marge).
    /// `spec` = « groupe|url » : les flux d'un même groupe démarrent au même segment.
    pub fn start(spec: &str) -> Arc<Live> {
        // « groupe|retard en s|url » : départ `retard` secondes avant le bord du direct
        let mut parts = spec.splitn(3, '|');
        let (group, back, playlist) = match (parts.next(), parts.next(), parts.next()) {
            (Some(g), Some(b), Some(u)) => (g, b.parse::<f64>().unwrap_or(0.0), u),
            _ => ("", 0.0, spec),
        };
        let group = group.to_string();
        let me = Arc::new(Live { st: Mutex::new(State { buf: VecDeque::new(), base: 0, ended: false, error: None, read: 0 }), cv: Condvar::new(), stop: AtomicBool::new(false) });
        let l = me.clone();
        let playlist = playlist.to_string();
        std::thread::spawn(move || {
            let a = agent();
            let mut last: Option<u64> = None;
            let mut fails = 0;
            while !l.stop.load(Ordering::Relaxed) {
                let text = match get(&a, &playlist) {
                    Ok(b) => String::from_utf8_lossy(&b).to_string(),
                    Err(e) => {
                        fails += 1;
                        if fails > 5 {
                            l.fail(format!("liste du direct : {e}"));
                            return;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                };
                fails = 0;
                let mut seq0 = 0u64;
                let mut target = 2.0f64;
                let mut segs = Vec::new();
                let mut ended = false;
                for line in text.lines() {
                    let line = line.trim();
                    if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
                        seq0 = v.parse().unwrap_or(0);
                    } else if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
                        target = v.parse().unwrap_or(2.0);
                    } else if line.starts_with("#EXT-X-ENDLIST") {
                        ended = true;
                    } else if !line.is_empty() && !line.starts_with('#') {
                        segs.push(line.to_string());
                    }
                }
                let n = segs.len() as u64;
                let template = segs.first().filter(|u| u.contains("/sq/")).cloned();
                // fenêtre de retour arrière : la liste, ou ~35 min si on peut adresser les anciens segments
                *WINDOW.lock().unwrap() = if template.is_some() { (n as f64 * target).max(35.0 * 60.0) } else { n as f64 * target };
                if last.is_none() && n > 0 {
                    // départ près du bord du direct, ou plus tôt (même segment pour la vidéo et le son)
                    let back_segs = (back / target.max(1.0)) as u64;
                    let first = if template.is_some() { 0 } else { seq0 };
                    last = Some(start_seq(&group, seq0 + n - 1, first, back_segs).saturating_sub(1));
                }
                let from = last.map(|lq| lq + 1).unwrap_or(seq0);
                let mut seq = from;
                let mut misses = 0;
                while seq < seq0 + n && !l.stop.load(Ordering::Relaxed) {
                    // le lecteur a assez d'avance : on attend qu'il consomme (rattrapage d'un retour arrière)
                    loop {
                        let st = l.st.lock().unwrap();
                        let ahead = (st.base + st.buf.len() as u64).saturating_sub(st.read);
                        if ahead < 24 << 20 || l.stop.load(Ordering::Relaxed) {
                            break;
                        }
                        drop(l.cv.wait_timeout(st, Duration::from_millis(500)).unwrap());
                    }
                    let url = if seq >= seq0 { Some(segs[(seq - seq0) as usize].clone()) } else { template.as_deref().and_then(|t| seg_url(t, seq)) };
                    let Some(url) = url else {
                        seq = seq0;
                        continue;
                    };
                    match get(&a, &url) {
                        Ok(b) => {
                            misses = 0;
                            let b = strip_id3(&b);
                            let mut st = l.st.lock().unwrap();
                            st.buf.extend(b);
                            // on ne garde que 8 Mo derrière la position de lecture
                            let keep_from = st.read.saturating_sub(8 << 20);
                            let drop_n = (keep_from.saturating_sub(st.base) as usize).min(st.buf.len());
                            if drop_n > 0 {
                                st.buf.drain(..drop_n);
                                st.base += drop_n as u64;
                            }
                            drop(st);
                            l.cv.notify_all();
                        }
                        Err(e) => {
                            crate::store::log(&format!("segment du direct {seq} : {e}"));
                            misses += 1;
                            // segment trop ancien (plus servi) : on avance jusqu'au premier disponible
                            if seq < seq0 && misses > 3 {
                                seq += 30;
                                continue;
                            }
                            // segments récents refusés : adresses périmées, le lecteur relancera une extraction
                            if seq >= seq0 && misses >= 3 {
                                l.fail("segments du direct refusés : nouvelle extraction".into());
                                return;
                            }
                        }
                    }
                    last = Some(seq);
                    seq += 1;
                }
                if ended {
                    l.st.lock().unwrap().ended = true;
                    l.cv.notify_all();
                    return;
                }
                std::thread::sleep(Duration::from_secs_f64((target / 2.0).clamp(0.5, 5.0)));
            }
        });
        me
    }

    fn fail(&self, e: String) {
        crate::store::log(&e);
        self.st.lock().unwrap().error = Some(e);
        self.cv.notify_all();
    }

    /// Lit à partir de `pos` (attend que les données arrivent).
    fn read_at(&self, pos: u64, out: &mut [u8]) -> std::result::Result<usize, String> {
        let mut st = self.st.lock().unwrap();
        loop {
            if pos < st.base {
                return Err("lecture trop en retard sur le direct".into());
            }
            let off = (pos - st.base) as usize;
            if off < st.buf.len() {
                let n = (st.buf.len() - off).min(out.len());
                // seules les lectures réellement servies comptent (Windows sonde parfois très loin)
                st.read = pos + n as u64;
                let (a, b) = st.buf.as_slices();
                for (i, o) in out[..n].iter_mut().enumerate() {
                    let k = off + i;
                    *o = if k < a.len() { a[k] } else { b[k - a.len()] };
                }
                drop(st);
                self.cv.notify_all();
                return Ok(n);
            }
            if st.ended {
                return Ok(0);
            }
            if let Some(e) = &st.error {
                return Err(e.clone());
            }
            if self.stop.load(Ordering::Relaxed) {
                return Ok(0);
            }
            st = self.cv.wait_timeout(st, Duration::from_millis(500)).unwrap().0;
        }
    }
}

/// Flux TS sans fin pour Media Foundation (lecture seule, séquentielle).
#[implement(IMFByteStream)]
pub struct LiveStream {
    l: Arc<Live>,
    pos: Mutex<u64>,
    results: Arc<Mutex<std::collections::HashMap<usize, u32>>>,
}

impl LiveStream {
    pub fn new(l: Arc<Live>) -> IMFByteStream {
        LiveStream { l, pos: Mutex::new(0), results: Arc::new(Mutex::new(Default::default())) }.into()
    }
}

impl Drop for LiveStream {
    fn drop(&mut self) {
        self.l.stop.store(true, Ordering::Relaxed);
        self.l.cv.notify_all();
    }
}

impl IMFByteStream_Impl for LiveStream_Impl {
    fn GetCapabilities(&self) -> Result<u32> {
        // « déplaçable » : sans cela les lecteurs TS/AAC de Windows refusent le flux. Les retours en
        // arrière restent possibles dans ce qui est encore en mémoire (voir SetCurrentPosition).
        Ok(MFBYTESTREAM_IS_READABLE | MFBYTESTREAM_IS_REMOTE | MFBYTESTREAM_IS_SEEKABLE)
    }
    fn GetLength(&self) -> Result<u64> {
        if std::env::var_os("TUBE_LOG_READS").is_some() {
            crate::store::log("direct : taille demandée");
        }
        Ok(u64::MAX) // inconnue
    }
    fn SetLength(&self, _l: u64) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetCurrentPosition(&self) -> Result<u64> {
        Ok(*self.pos.lock().unwrap())
    }
    fn SetCurrentPosition(&self, p: u64) -> Result<()> {
        if std::env::var_os("TUBE_LOG_READS").is_some() {
            crate::store::log(&format!("direct : position {p}"));
        }
        let mut cur = self.pos.lock().unwrap();
        // un direct ne se « déplace » pas ; on tolère un retour en arrière dans ce qui est encore en mémoire
        if p >= self.l.st.lock().unwrap().base {
            *cur = p;
            Ok(())
        } else {
            Err(E_INVALIDARG.into())
        }
    }
    fn IsEndOfStream(&self) -> Result<BOOL> {
        let st = self.l.st.lock().unwrap();
        Ok((st.ended && *self.pos.lock().unwrap() >= st.base + st.buf.len() as u64).into())
    }
    fn Read(&self, pb: *mut u8, cb: u32, pcbread: *mut u32) -> Result<()> {
        let pos = *self.pos.lock().unwrap();
        let buf = unsafe { std::slice::from_raw_parts_mut(pb, cb as usize) };
        let n = self.l.read_at(pos, buf).map_err(|e| {
            crate::store::log(&format!("direct : {e}"));
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
            crate::store::log(&format!("direct : lecture {pos} +{cb}"));
        }
        let l = self.l.clone();
        let results = self.results.clone();
        let key = res.as_raw() as usize;
        let raw = res.into_raw() as usize;
        let pb = pb as usize;
        // la lecture attend l'arrivée des segments : dans un fil, jamais sur les files de Media Foundation
        std::thread::spawn(move || {
            // SAFETY : le tampon reste valide jusqu'à EndRead (contrat de IMFByteStream::BeginRead).
            let buf = unsafe { std::slice::from_raw_parts_mut(pb as *mut u8, cb as usize) };
            let res: IMFAsyncResult = unsafe { IMFAsyncResult::from_raw(raw as *mut _) };
            match l.read_at(pos, buf) {
                Ok(n) => {
                    results.lock().unwrap().insert(key, n as u32);
                    unsafe {
                        let _ = res.SetStatus(S_OK);
                    }
                }
                Err(e) => {
                    crate::store::log(&format!("direct : {e}"));
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
        // lectures séquentielles : la position avance du nombre d'octets réellement lus
        *self.pos.lock().unwrap() += n as u64;
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
        let cur = *self.pos.lock().unwrap();
        let p = if origin == msoCurrent { (cur as i64 + off).max(0) as u64 } else { off.max(0) as u64 };
        self.SetCurrentPosition(p)?;
        Ok(p)
    }
    fn Flush(&self) -> Result<()> {
        Ok(())
    }
    fn Close(&self) -> Result<()> {
        self.l.stop.store(true, Ordering::Relaxed);
        self.l.cv.notify_all();
        Ok(())
    }
}
