//! Prototype de faisabilité : lit une vidéo YouTube (flux vidéo AV1/VP9 + flux audio
//! séparés, DASH) avec Media Foundation, affichage direct GPU dans une fenêtre Win32.
//! Usage : tube <url_video> <url_audio>   (URLs googlevideo obtenues par yt-dlp -g)
//! Touches : Espace pause, ← → ±10 s, Échap quitter. Mesures imprimées toutes les 2 s.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::*;

static mut URLS: (String, String) = (String::new(), String::new());

#[implement(IMFMediaEngineNotify)]
struct Notify {
    q: Arc<Mutex<Vec<u32>>>,
}
impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, e: u32, _a: usize, _b: u32) -> Result<()> {
        self.q.lock().unwrap().push(e);
        Ok(())
    }
}

/// Résout une URL http en source Media Foundation.
fn source(url: &str) -> Result<IMFMediaSource> {
    unsafe {
        let r = MFCreateSourceResolver()?;
        let mut t = MF_OBJECT_TYPE::default();
        let mut obj = None;
        let w = HSTRING::from(url);
        r.CreateObjectFromURL(
            &w,
            (MF_RESOLUTION_MEDIASOURCE.0 | MF_RESOLUTION_CONTENT_DOES_NOT_HAVE_TO_MATCH_EXTENSION_OR_MIME_TYPE.0) as u32,
            None,
            &mut t,
            &mut obj,
        )?;
        obj.ok_or_else(|| Error::from(E_FAIL))?.cast()
    }
}

/// Extension du moteur : l'URL « tubeagg:// » devient la source agrégée vidéo + audio.
#[implement(IMFMediaEngineExtension)]
struct Ext;
impl IMFMediaEngineExtension_Impl for Ext_Impl {
    fn CanPlayType(&self, _a: BOOL, _m: &BSTR) -> Result<MF_MEDIA_ENGINE_CANPLAY> {
        Ok(MF_MEDIA_ENGINE_CANPLAY_PROBABLY)
    }
    fn BeginCreateObject(
        &self,
        url: &BSTR,
        _bs: Ref<IMFByteStream>,
        ty: MF_OBJECT_TYPE,
        cookie: OutRef<IUnknown>,
        cb: Ref<IMFAsyncCallback>,
        st: Ref<IUnknown>,
    ) -> Result<()> {
        let _ = cookie.write(None);
        eprintln!("[ext] BeginCreateObject {} type={}", url, ty.0);
        if ty != MF_OBJECT_MEDIASOURCE || !url.to_string().starts_with("tubeagg:") {
            return Err(Error::from(MF_E_UNSUPPORTED_SCHEME));
        }
        let t0 = Instant::now();
        let (vu, au) = unsafe { (&*std::ptr::addr_of!(URLS)).clone() };
        let v = source(&vu)?;
        let a = source(&au)?;
        eprintln!("[ext] sources résolues en {:?}", t0.elapsed());
        unsafe {
            let c = MFCreateCollection()?;
            c.AddElement(&v)?;
            c.AddElement(&a)?;
            let agg = MFCreateAggregateSource(&c)?;
            let res = MFCreateAsyncResult(&agg, cb.as_ref(), st.as_ref())?;
            MFInvokeCallback(&res)?;
        }
        Ok(())
    }
    fn CancelObjectCreation(&self, _c: Ref<IUnknown>) -> Result<()> {
        Ok(())
    }
    fn EndCreateObject(&self, r: Ref<IMFAsyncResult>) -> Result<IUnknown> {
        unsafe { r.ok()?.GetObject() }
    }
}

static mut ENGINE: Option<IMFMediaEngineEx> = None;
static mut SEEK_T0: Option<Instant> = None;

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let eng = (&*std::ptr::addr_of!(ENGINE)).clone();
        match m {
            WM_SIZE => {
                if let Some(e) = &eng {
                    let dst = RECT { left: 0, top: 0, right: (lp.0 & 0xffff) as i32, bottom: ((lp.0 >> 16) & 0xffff) as i32 };
                    let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
                    let _ = e.UpdateVideoStream(None, Some(&dst), Some(&black));
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if let Some(e) = &eng {
                    match wp.0 as u16 {
                        k if k == VK_SPACE.0 => {
                            if e.IsPaused().as_bool() { let _ = e.Play(); } else { let _ = e.Pause(); }
                        }
                        k if k == VK_LEFT.0 || k == VK_RIGHT.0 => {
                            let d = if k == VK_LEFT.0 { -10.0 } else { 10.0 };
                            *std::ptr::addr_of_mut!(SEEK_T0) = Some(Instant::now());
                            let _ = e.SetCurrentTime((e.GetCurrentTime() + d).max(0.0));
                        }
                        k if k == VK_ESCAPE.0 => PostQuitMessage(0),
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }
}

fn proc_stats() -> (f64, f64, f64) {
    unsafe {
        let p = GetCurrentProcess();
        let (mut c, mut x, mut k, mut u) = Default::default();
        let _ = GetProcessTimes(p, &mut c, &mut x, &mut k, &mut u);
        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 1e7;
        let mut mc = PROCESS_MEMORY_COUNTERS_EX::default();
        let _ = GetProcessMemoryInfo(p, &mut mc as *mut _ as *mut _, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
        (ft(k) + ft(u), mc.WorkingSetSize as f64 / 1048576.0, mc.PrivateUsage as f64 / 1048576.0)
    }
}

fn main() -> Result<()> {
    let start = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let (vu, au) = if args.len() >= 3 {
        (args[1].clone(), args[2].clone())
    } else {
        let s = std::fs::read_to_string(args.get(1).map(|s| s.as_str()).unwrap_or("urls.txt")).expect("urls.txt");
        let mut l = s.lines();
        (l.next().unwrap().to_string(), l.next().unwrap().to_string())
    };
    unsafe {
        *std::ptr::addr_of_mut!(URLS) = (vu, au);
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL)?;
        let inst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: w!("TubeProto"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("TubeProto"),
            w!("Tube — prototype"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100, 100, 1600, 920,
            None, None, Some(inst.into()), None,
        )?;

        let factory: IMFMediaEngineClassFactory = CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)?;
        let mut attr = None;
        MFCreateAttributes(&mut attr, 4)?;
        let attr = attr.unwrap();
        let q = Arc::new(Mutex::new(Vec::new()));
        let notify: IMFMediaEngineNotify = Notify { q: q.clone() }.into();
        attr.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
        attr.SetUINT64(&MF_MEDIA_ENGINE_PLAYBACK_HWND, hwnd.0 as u64)?;
        let ext: IMFMediaEngineExtension = Ext.into();
        attr.SetUnknown(&MF_MEDIA_ENGINE_EXTENSION, &ext)?;
        let engine = factory.CreateInstance(0, &attr)?;
        let ex: IMFMediaEngineEx = engine.cast()?;
        *std::ptr::addr_of_mut!(ENGINE) = Some(ex.clone());
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        let _ = ex.UpdateVideoStream(None, Some(&rc), Some(&black));
        engine.SetSource(&BSTR::from("tubeagg://video"))?;
        engine.SetAutoPlay(true)?;
        SetTimer(Some(hwnd), 1, 100, None);

        let mut last = (Instant::now(), proc_stats().0);
        let mut first_frame = false;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            // Test auto (TUBE_AUTOSEEK=1) : sauts à 10, 18, 26, 34 s, arrêt à 45 s.
            if std::env::var("TUBE_AUTOSEEK").is_ok() {
                let s = start.elapsed().as_secs();
                static mut DONE: u64 = 0;
                let plan: [(u64, f64); 4] = [(10, 120.0), (18, 60.0), (26, 500.0), (34, 5.0)];
                for (at, to) in plan {
                    if s >= at && *std::ptr::addr_of!(DONE) < at {
                        *std::ptr::addr_of_mut!(DONE) = at;
                        eprintln!("[seek] → {to} s");
                        *std::ptr::addr_of_mut!(SEEK_T0) = Some(Instant::now());
                        let _ = engine.SetCurrentTime(to);
                    }
                }
                if s >= 45 {
                    break;
                }
            }
            if let Ok(v) = std::env::var("TUBE_RUN_SECS") {
                if start.elapsed().as_secs() >= v.parse().unwrap_or(60) { break; }
            }
            let evs: Vec<u32> = std::mem::take(&mut *q.lock().unwrap());
            for e in evs {
                let e = e as i32;
                if e == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 {
                    let (mut w, mut h) = (0, 0);
                    let _ = engine.GetNativeVideoSize(Some(&mut w), Some(&mut h));
                    eprintln!("[{:>6} ms] métadonnées : {}x{}, durée {:.0} s, audio {}", start.elapsed().as_millis(), w, h, engine.GetDuration(), engine.HasAudio().as_bool());
                } else if e == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 || (e == MF_MEDIA_ENGINE_EVENT_PLAYING.0 && !first_frame) {
                    first_frame = true;
                    eprintln!("[{:>6} ms] première image / lecture", start.elapsed().as_millis());
                } else if e == MF_MEDIA_ENGINE_EVENT_SEEKED.0 {
                    if let Some(t) = (*std::ptr::addr_of_mut!(SEEK_T0)).take() {
                        eprintln!("[seek] {:?}", t.elapsed());
                    }
                } else if e == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                    let c = engine.GetError().map(|er| er.GetErrorCode()).unwrap_or(0);
                    let hr = engine.GetError().map(|er| er.GetExtendedErrorCode().err().map(|h| h.code().0).unwrap_or(0)).unwrap_or(0);
                    eprintln!("ERREUR moteur : code {c} hr {hr:#x}");
                } else if e == MF_MEDIA_ENGINE_EVENT_WAITING.0 {
                    eprintln!("[{:>6} ms] mise en mémoire tampon…", start.elapsed().as_millis());
                }
            }
            if last.0.elapsed().as_secs_f64() >= 2.0 {
                let (cpu, ws, pv) = proc_stats();
                let dt = last.0.elapsed().as_secs_f64();
                let get = |id| {
                    ex.GetStatistics(id).ok().map(|v| {
                        let i = &v.Anonymous.Anonymous;
                        if i.vt.0 == 19 || i.vt.0 == 3 { i.Anonymous.ulVal as u64 } else { i.Anonymous.uhVal }
                    }).unwrap_or(0)
                };
                eprintln!(
                    "t={:>7.1}s  CPU {:>5.1}% (1 cœur=100)  RAM ws {:>5.0} Mo priv {:>5.0} Mo  images {} perdues {}",
                    engine.GetCurrentTime(),
                    (cpu - last.1) / dt * 100.0,
                    ws, pv,
                    get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_RENDERED),
                    get(MF_MEDIA_ENGINE_STATISTIC_FRAMES_DROPPED),
                );
                last = (Instant::now(), cpu);
            }
        }
        let _ = engine.Shutdown();
    }
    Ok(())
}
