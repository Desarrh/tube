//! Mode « audio seul » : visualisation du son réellement joué par Tube.
//! Windows 11 permet de capturer le son d'UN processus (« process loopback ») : on écoute
//! uniquement Tube (pas les autres applications), puis une FFT donne le spectre affiché.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::core::*;

const N: usize = 2048;
const RATE: f32 = 44100.0;

pub struct Viz {
    buf: Arc<Mutex<VecDeque<f32>>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Viz {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Done(Mutex<Option<Sender<()>>>);

impl IAgileObject_Impl for Done_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(&self, _op: Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        if let Some(tx) = self.0.lock().unwrap().take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

/// PROPVARIANT de type VT_BLOB (disposition mémoire x64 : vt, 3 réservés, puis BLOB).
#[repr(C)]
struct BlobVariant {
    vt: u16,
    r: [u16; 3],
    cb: u32,
    _pad: u32,
    ptr: *const u8,
}

fn open() -> Result<(IAudioClient, IAudioCaptureClient)> {
    unsafe {
        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: std::process::id(), ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE },
            },
        };
        let pv = BlobVariant { vt: 65, r: [0; 3], cb: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, _pad: 0, ptr: &params as *const _ as *const u8 };
        let (tx, rx) = channel();
        let done: IActivateAudioInterfaceCompletionHandler = Done(Mutex::new(Some(tx))).into();
        let op = ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&pv as *const _ as *const _), &done)?;
        rx.recv_timeout(Duration::from_secs(3)).map_err(|_| Error::from(windows::Win32::Foundation::E_FAIL))?;
        let mut hr = HRESULT(0);
        let mut unk = None;
        op.GetActivateResult(&mut hr, &mut unk)?;
        hr.ok()?;
        let client: IAudioClient = unk.ok_or_else(|| Error::from(windows::Win32::Foundation::E_FAIL))?.cast()?;
        let fmt = WAVEFORMATEX { wFormatTag: 1, nChannels: 2, nSamplesPerSec: RATE as u32, nAvgBytesPerSec: RATE as u32 * 4, nBlockAlign: 4, wBitsPerSample: 16, cbSize: 0 };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, 2_000_000, 0, &fmt, None)?;
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok((client, cap))
    }
}

impl Viz {
    pub fn start() -> Option<Viz> {
        let buf = Arc::new(Mutex::new(VecDeque::with_capacity(N * 2)));
        let stop = Arc::new(AtomicBool::new(false));
        let (b, s) = (buf.clone(), stop.clone());
        let (ok_tx, ok_rx) = channel();
        std::thread::spawn(move || unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let (client, cap) = match open() {
                Ok(x) => {
                    let _ = ok_tx.send(true);
                    x
                }
                Err(e) => {
                    crate::store::log(&format!("visualisation du son indisponible : {e}"));
                    let _ = ok_tx.send(false);
                    return;
                }
            };
            while !s.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(15));
                while cap.GetNextPacketSize().unwrap_or(0) > 0 {
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0u32;
                    let mut flags = 0u32;
                    if cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None).is_err() {
                        break;
                    }
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    let pcm = std::slice::from_raw_parts(data as *const i16, frames as usize * 2);
                    let mut q = b.lock().unwrap();
                    for f in 0..frames as usize {
                        let v = if silent { 0.0 } else { (pcm[f * 2] as f32 + pcm[f * 2 + 1] as f32) / 65536.0 };
                        q.push_back(v);
                    }
                    while q.len() > N * 2 {
                        q.pop_front();
                    }
                    drop(q);
                    let _ = cap.ReleaseBuffer(frames);
                }
            }
            let _ = client.Stop();
        });
        if ok_rx.recv_timeout(Duration::from_secs(4)).unwrap_or(false) { Some(Viz { buf, stop }) } else { None }
    }

    /// Spectre en `bands` bandes logarithmiques (40 Hz → 16 kHz), valeurs 0..1.
    pub fn spectrum(&self, bands: usize) -> Vec<f32> {
        let q = self.buf.lock().unwrap();
        if q.len() < N {
            return vec![0.0; bands];
        }
        let start = q.len() - N;
        let mut re: Vec<f32> = (0..N).map(|i| q[start + i] * (0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos())).collect();
        drop(q);
        let mut im = vec![0f32; N];
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..N / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
        let (lo, hi) = (40f32.ln(), 16000f32.ln());
        (0..bands)
            .map(|b| {
                let f0 = (lo + (hi - lo) * b as f32 / bands as f32).exp();
                let f1 = (lo + (hi - lo) * (b + 1) as f32 / bands as f32).exp();
                let (k0, k1) = ((f0 / RATE * N as f32) as usize, ((f1 / RATE * N as f32) as usize).max((f0 / RATE * N as f32) as usize + 1));
                let m = mag[k0.min(N / 2 - 1)..k1.min(N / 2)].iter().cloned().fold(0.0f32, f32::max);
                // dB → 0..1 (−60 dB … 0 dB), léger relèvement des aigus
                let db = 20.0 * (m / (N as f32 / 4.0) + 1e-9).log10() + 6.0 * (b as f32 / bands as f32);
                ((db + 60.0) / 60.0).clamp(0.0, 1.0)
            })
            .collect()
    }
}

/// FFT radix-2 en place.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        for i in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (ang * k as f32).sin_cos();
                let (a, b) = (i + k, i + k + len / 2);
                let tr = re[b] * c - im[b] * s;
                let ti = re[b] * s + im[b] * c;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}
