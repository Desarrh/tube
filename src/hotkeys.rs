//! Raccourcis globaux Win+Alt (actifs même quand Tube n'a pas le focus : en jeu, dans l'IDE…).
//! Windows réserve plusieurs Win+Alt (accrochage des fenêtres, Game Bar) : RegisterHotKey échoue.
//! On utilise donc un crochet clavier bas niveau qui n'intercepte ces combinaisons QUE lorsqu'une
//! vidéo est chargée ; le reste du temps, elles gardent leur effet Windows habituel.
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hk {
    Toggle,
    Back,
    Fwd,
    VolUp,
    VolDown,
    Mini,
    Half,
    Mute,
    /// touches multimédia (panneau Windows)
    Next,
    Prev,
}

pub const LIST: [(Hk, VIRTUAL_KEY, &str); 8] = [
    (Hk::Toggle, VK_SPACE, "Win+Alt+Espace : lecture / pause"),
    (Hk::Back, VK_LEFT, "Win+Alt+← : reculer de 10 s"),
    (Hk::Fwd, VK_RIGHT, "Win+Alt+→ : avancer de 10 s"),
    (Hk::VolUp, VK_UP, "Win+Alt+↑ : volume +"),
    (Hk::VolDown, VK_DOWN, "Win+Alt+↓ : volume −"),
    (Hk::Mini, VK_M, "Win+Alt+M : mini-lecteur"),
    (Hk::Half, VK_H, "Win+Alt+H : demi-écran"),
    (Hk::Mute, VK_N, "Win+Alt+N : couper le son"),
];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static SEND: OnceLock<Box<dyn Fn(Hk) + Send + Sync>> = OnceLock::new();

/// Vrai quand une vidéo est chargée : les raccourcis sont alors interceptés.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
}

/// État des modificateurs, suivi par le crochet lui-même (plus fiable que GetAsyncKeyState ici).
static WIN: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        let msg = wp.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk = k.vkCode;
        if vk == VK_LWIN.0 as u32 || vk == VK_RWIN.0 as u32 {
            WIN.store(is_down, Ordering::Relaxed);
        } else if vk == VK_MENU.0 as u32 || vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32 {
            ALT.store(is_down, Ordering::Relaxed);
        }
        let ours = k.flags.0 & LLKHF_INJECTED.0 == 0 || (k.dwExtraInfo == 0x7475_6265 && std::env::var_os("TUBE_SCRIPT").is_some());
        // Win+L (et le bureau sécurisé) avale le relâchement de Win : sans contre-vérification,
        // WIN resterait « appuyé » et Alt+← serait intercepté dans les autres applis.
        let win_held = || unsafe { GetAsyncKeyState(VK_LWIN.0 as i32) < 0 || GetAsyncKeyState(VK_RWIN.0 as i32) < 0 };
        if is_down && ours && ACTIVE.load(Ordering::Relaxed) && WIN.load(Ordering::Relaxed) && !win_held() {
            WIN.store(false, Ordering::Relaxed);
        }
        if is_down && ours && ACTIVE.load(Ordering::Relaxed) && WIN.load(Ordering::Relaxed) && ALT.load(Ordering::Relaxed) {
            if let Some((hk, _, _)) = LIST.iter().find(|(_, vk, _)| vk.0 as u32 == k.vkCode) {
                if let Some(f) = SEND.get() {
                    f(*hk);
                }
                // touche « masque » : sans elle, relâcher Win ouvrirait le menu Démarrer
                // et relâcher Alt activerait la barre de menus de l'appli au premier plan.
                unsafe {
                    keybd_event(0xE8, 0, KEYBD_EVENT_FLAGS(0), 0);
                    keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
                }
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

/// Lance le fil du crochet clavier. Renvoie la liste des raccourcis indisponibles (vide si tout va bien).
pub fn start(send: impl Fn(Hk) + Send + Sync + 'static) -> std::sync::mpsc::Receiver<Vec<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = SEND.set(Box::new(send));
    std::thread::spawn(move || unsafe {
        let ok = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0).is_ok();
        let _ = tx.send(if ok { vec![] } else { LIST.iter().map(|l| l.2.to_string()).collect() });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
    rx
}

/// Test (TUBE_SCRIPT « hk ») : simule Win+Alt+touche.
pub fn simulate(vk: VIRTUAL_KEY) {
    let key = |v: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: v, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0x7475_6265 } },
    };
    let seq = [key(VK_LWIN, false), key(VK_MENU, false), key(vk, false), key(vk, true), key(VK_MENU, true), key(VK_LWIN, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}
