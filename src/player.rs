//! Lecture Media Foundation (intégré à Windows, décodage et affichage par la carte graphique).
//! YouTube sert la vidéo et le son séparément (DASH) : une extension du moteur résout l'URL
//! « tubeagg://… » en une source agrégée (MFCreateAggregateSource) des deux flux HTTP.
//! Le WebM/VP9 est préféré : son index (Cues) permet des sauts rapides, contrairement au MP4 fragmenté.
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;
use windows::core::*;

pub fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    }
}

#[implement(IMFMediaEngineNotify)]
struct Notify {
    q: Arc<Mutex<Vec<u32>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, e: u32, _a: usize, _b: u32) -> Result<()> {
        // TIMEUPDATE (≈ 4 par seconde) et PROGRESS ne réveillent pas l'interface :
        // la logique les relève sur son propre minuteur, l'affichage du temps suit à 1 Hz.
        if e == MF_MEDIA_ENGINE_EVENT_TIMEUPDATE.0 as u32 || e == MF_MEDIA_ENGINE_EVENT_PROGRESS.0 as u32 {
            return Ok(());
        }
        if let Ok(mut q) = self.q.lock() {
            q.push(e);
        }
        (self.wake)();
        Ok(())
    }
}

/// Source Media Foundation lue par notre flux HTTP par blocs (voir http.rs).
fn source(url: &str, ext: &str, cache_mb: usize) -> Result<IMFMediaSource> {
    let bs = if let Some(pl) = url.strip_prefix("hls:") {
        // direct : segments TS mis bout à bout
        crate::live::LiveStream::new(crate::live::Live::start(pl))
    } else {
        let remote = crate::http::Remote::open(url, cache_mb).map_err(|e| {
            crate::store::log(&format!("ouverture du flux : {e}"));
            Error::from(E_FAIL)
        })?;
        crate::http::ByteStream::new(remote)
    };
    unsafe {
        let r = MFCreateSourceResolver()?;
        let mut t = MF_OBJECT_TYPE::default();
        let mut obj = None;
        r.CreateObjectFromByteStream(
            &bs,
            &HSTRING::from(format!("flux.{ext}")),
            (MF_RESOLUTION_MEDIASOURCE.0 | MF_RESOLUTION_CONTENT_DOES_NOT_HAVE_TO_MATCH_EXTENSION_OR_MIME_TYPE.0) as u32,
            None,
            &mut t,
            &mut obj,
        )?;
        obj.ok_or_else(|| Error::from(E_FAIL))?.cast()
    }
}

#[implement(IMFMediaEngineExtension)]
struct Ext {
    /// (url, extension) vidéo puis son ; son vide = flux unique
    urls: ((String, String), (String, String)),
}
impl IMFMediaEngineExtension_Impl for Ext_Impl {
    fn CanPlayType(&self, _a: BOOL, _m: &BSTR) -> Result<MF_MEDIA_ENGINE_CANPLAY> {
        Ok(MF_MEDIA_ENGINE_CANPLAY_PROBABLY)
    }
    fn BeginCreateObject(&self, url: &BSTR, _bs: Ref<IMFByteStream>, ty: MF_OBJECT_TYPE, cookie: OutRef<IUnknown>, cb: Ref<IMFAsyncCallback>, st: Ref<IUnknown>) -> Result<()> {
        let _ = cookie.write(None);
        if ty != MF_OBJECT_MEDIASOURCE || !url.to_string().starts_with("tubeagg:") {
            return Err(Error::from(MF_E_UNSUPPORTED_SCHEME));
        }
        let ((vu, ve), (au, ae)) = self.urls.clone();
        let cb: IMFAsyncCallback = cb.ok()?.clone();
        let st: Option<IUnknown> = st.as_ref().cloned();
        let (cbp, stp) = (cb.into_raw() as usize, st.map(|s| s.into_raw() as usize).unwrap_or(0));
        // ouverture réseau hors du fil appelant (le fil de l'interface, sinon elle gèlerait)
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let cb = IMFAsyncCallback::from_raw(cbp as *mut _);
            let st = if stp == 0 { None } else { Some(IUnknown::from_raw(stp as *mut _)) };
            let built: Result<IMFMediaSource> = (|| {
                if au.is_empty() {
                    return source(&vu, &ve, 56);
                }
                let th = std::thread::spawn(move || {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    source(&au, &ae, 8).map(|s| s.into_raw() as usize).map_err(|e| e.code())
                });
                let v = source(&vu, &ve, 48);
                let a = th.join().map_err(|_| Error::from(E_FAIL))?.map_err(Error::from)?;
                // SAFETY : pointeur COM transmis par le fil ci-dessus, propriété reprise ici.
                let a = IMFMediaSource::from_raw(a as *mut _);
                let v = v?;
                let c = MFCreateCollection()?;
                c.AddElement(&v)?;
                c.AddElement(&a)?;
                MFCreateAggregateSource(&c)
            })();
            let res = match &built {
                Ok(src) => MFCreateAsyncResult(src, &cb, st.as_ref()),
                Err(_) => MFCreateAsyncResult(None, &cb, st.as_ref()),
            };
            if let Ok(res) = res {
                if let Err(e) = &built {
                    let _ = res.SetStatus(e.code());
                }
                let _ = MFInvokeCallback(&res);
            }
        });
        Ok(())
    }
    fn CancelObjectCreation(&self, _c: Ref<IUnknown>) -> Result<()> {
        Ok(())
    }
    fn EndCreateObject(&self, r: Ref<IMFAsyncResult>) -> Result<IUnknown> {
        let r = r.ok()?;
        unsafe {
            r.GetStatus()?;
            r.GetObject()
        }
    }
}

pub struct Player {
    engine: IMFMediaEngine,
    ex: IMFMediaEngineEx,
    events: Arc<Mutex<Vec<u32>>>,
    pub ready: bool,
    pub duration: f64,
    pub size: (u32, u32),
    pub error: Option<String>,
    pub ended: bool,
    pub buffering: bool,
    pending_seek: Option<f64>,
    want_play: bool,
    play_after_seek: bool,
    logged_play: bool,
}

impl Player {
    /// `audio` vide : `video` est un flux complet (son inclus).
    pub fn open(hwnd: isize, video: (&str, &str), audio: (&str, &str), start: f64, play: bool, volume: f64, rate: f64, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Player> {
        unsafe {
            let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
            let mut attr = None;
            MFCreateAttributes(&mut attr, 4)?;
            let attr = attr.ok_or_else(|| Error::from(E_FAIL))?;
            let events = Arc::new(Mutex::new(Vec::new()));
            let notify: IMFMediaEngineNotify = Notify { q: events.clone(), wake }.into();
            attr.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attr.SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd as u64)?;
            attr.SetUINT32(&MF_MEDIA_ENGINE_AUDIO_CATEGORY, 11)?; // AudioCategory_Media
            let ext: IMFMediaEngineExtension = Ext { urls: ((video.0.to_string(), video.1.to_string()), (audio.0.to_string(), audio.1.to_string())) }.into();
            attr.SetUnknown(&MF_MEDIA_ENGINE_EXTENSION, &ext)?;
            let engine = factory.CreateInstance(0, &attr)?;
            let ex: IMFMediaEngineEx = engine.cast()?;
            let _ = engine.SetVolume(volume);
            let _ = engine.SetDefaultPlaybackRate(rate);
            let _ = engine.SetPlaybackRate(rate);
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let src = format!("tubeagg://{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            engine.SetSource(&BSTR::from(src))?;
            // reprise : on se place d'abord, on lance la lecture une fois le saut terminé
            // (Play() avant le saut initial bloque parfois le moteur).
            if play && start <= 1.0 {
                let _ = engine.Play();
            }
            Ok(Player {
                engine,
                ex,
                events,
                ready: false,
                duration: 0.0,
                size: (0, 0),
                error: None,
                ended: false,
                buffering: true,
                pending_seek: if start > 1.0 { Some(start) } else { None },
                want_play: play,
                play_after_seek: play && start > 1.0,
                logged_play: false,
            })
        }
    }

    pub fn poll(&mut self) -> bool {
        let evs: Vec<u32> = self.events.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
        for e in &evs {
            let e = *e as i32;
            if std::env::var_os("TUBE_LOG_EVENTS").is_some() {
                crate::store::log(&format!("mf événement {e} t={:.2} pause={}", unsafe { self.engine.GetCurrentTime() }, unsafe { self.engine.IsPaused().as_bool() }));
            }
            unsafe {
                if e == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 || e == MF_MEDIA_ENGINE_EVENT_DURATIONCHANGE.0 || e == MF_MEDIA_ENGINE_EVENT_FORMATCHANGE.0 {
                    let d = self.engine.GetDuration();
                    if d.is_finite() {
                        self.duration = d;
                    }
                    let (mut w, mut h) = (0u32, 0u32);
                    let _ = self.engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                    self.size = (w, h);
                    if !self.ready {
                        crate::store::log(&format!("métadonnées : {w}x{h}"));
                        self.ready = true;
                        if let Some(t) = self.pending_seek.take() {
                            let _ = self.engine.SetCurrentTime(t);
                        }
                        // le saut est demandé : on peut lancer la lecture (SEEKED n'arrive pas toujours en pause)
                        if self.play_after_seek {
                            self.play_after_seek = false;
                            if self.want_play {
                                let _ = self.engine.Play();
                            }
                        }
                    }
                } else if e == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                    let (code, hr) = match self.engine.GetError() {
                        Ok(er) => (er.GetErrorCode(), er.GetExtendedErrorCode().err().map(|h| h.code().0).unwrap_or(0)),
                        Err(_) => (0, 0),
                    };
                    self.error = Some(format!("lecture impossible (code {code}, {hr:#010x})"));
                } else if e == MF_MEDIA_ENGINE_EVENT_ENDED.0 {
                    self.ended = true;
                } else if e == MF_MEDIA_ENGINE_EVENT_WAITING.0 || e == MF_MEDIA_ENGINE_EVENT_SEEKING.0 {
                    self.buffering = true;
                } else if e == MF_MEDIA_ENGINE_EVENT_PLAYING.0 || e == MF_MEDIA_ENGINE_EVENT_SEEKED.0 || e == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 || e == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 || e == MF_MEDIA_ENGINE_EVENT_PAUSE.0 {
                    self.buffering = false;
                    if e == MF_MEDIA_ENGINE_EVENT_PLAYING.0 {
                        if !self.logged_play {
                            self.logged_play = true;
                            crate::store::log("lecture démarrée");
                        }
                        self.ended = false;
                    }
                    if e == MF_MEDIA_ENGINE_EVENT_SEEKED.0 && self.play_after_seek {
                        self.play_after_seek = false;
                        if self.want_play {
                            let _ = self.engine.Play();
                        }
                    }
                }
            }
        }
        !evs.is_empty()
    }

    pub fn playing(&self) -> bool {
        if self.play_after_seek {
            return false;
        }
        unsafe { !self.engine.IsPaused().as_bool() && !self.ended }
    }
    pub fn play(&mut self) {
        self.play_after_seek = false;
        if self.ended {
            self.seek(0.0);
        }
        self.want_play = true;
        unsafe {
            let _ = self.engine.Play();
        }
    }
    pub fn pause(&mut self) {
        self.want_play = false;
        unsafe {
            let _ = self.engine.Pause();
        }
    }
    pub fn toggle(&mut self) {
        if self.want_play && self.playing() { self.pause() } else { self.play() }
    }
    pub fn wants_play(&self) -> bool {
        self.want_play
    }
    pub fn time(&self) -> f64 {
        if let Some(t) = self.pending_seek {
            return t;
        }
        unsafe { self.engine.GetCurrentTime() }
    }
    pub fn seek(&mut self, t: f64) {
        let t = if self.duration > 0.0 { t.clamp(0.0, self.duration - 0.5) } else { t.max(0.0) };
        if !self.ready {
            self.pending_seek = Some(t);
            return;
        }
        self.ended = false;
        unsafe {
            // saut « approximatif » : sur l'image clé la plus proche (comme le site), sans décoder
            // toutes les images jusqu'à la position exacte — nettement plus rapide
            if self.ex.SetCurrentTimeEx(t, MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE).is_err() {
                let _ = self.engine.SetCurrentTime(t);
            }
        }
    }
    pub fn set_volume(&mut self, v: f64) {
        unsafe {
            let _ = self.engine.SetVolume(v.clamp(0.0, 1.0));
        }
    }
    pub fn set_rate(&mut self, r: f64) {
        unsafe {
            let _ = self.engine.SetDefaultPlaybackRate(r);
            let _ = self.engine.SetPlaybackRate(r);
        }
    }
    /// La sortie vidéo occupe toute la sous-fenêtre (w × h pixels).
    pub fn set_output(&self, w: i32, h: i32) {
        let dst = RECT { left: 0, top: 0, right: w, bottom: h };
        let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        unsafe {
            let _ = self.ex.UpdateVideoStream(None, Some(&dst), Some(&black));
        }
    }
    /// (images affichées, images perdues)
    pub fn stats(&self) -> (u64, u64) {
        let get = |id| unsafe {
            self.ex
                .GetStatistics(id)
                .ok()
                .map(|v| {
                    let i = &v.Anonymous.Anonymous;
                    if i.vt.0 == 19 || i.vt.0 == 3 { i.Anonymous.ulVal as u64 } else { i.Anonymous.uhVal }
                })
                .unwrap_or(0)
        };
        (get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_RENDERED), get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_DROPPED))
    }
    /// Proportion déjà en mémoire tampon après la position courante (secondes).
    pub fn buffered_ahead(&self) -> f64 {
        unsafe {
            let Ok(r) = self.engine.GetBuffered() else { return 0.0 };
            let now = self.engine.GetCurrentTime();
            let mut best = 0.0;
            for i in 0..r.GetLength() {
                let (Ok(s), Ok(e)) = (r.GetStart(i), r.GetEnd(i)) else { continue };
                if s <= now + 0.5 && e > now {
                    best = e - now;
                }
            }
            best
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
        }
    }
}
