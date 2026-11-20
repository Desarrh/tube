//! Cookies du compte YouTube (format Netscape, partagé avec yt-dlp) et import depuis Opera GX.
//! Opera verrouille son fichier Cookies tant qu'il tourne : seul son processus « NetworkService »
//! le tient. On termine ce processus (Opera le relance aussitôt), on copie le fichier dans la
//! foulée, puis yt-dlp le déchiffre (DPAPI). Aucun mot de passe n'est jamais saisi ni lu.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Cookie {
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub expires: u64,
    pub name: String,
    pub value: String,
}

#[derive(Default)]
pub struct Jar {
    pub list: Vec<Cookie>,
    pub dirty: bool,
}

impl Jar {
    /// Ne garde que les cookies youtube.com ; un ancien fichier qui contenait aussi ceux de
    /// google.com (accès au reste du compte) est réécrit aussitôt sans eux.
    pub fn load(p: &Path) -> Jar {
        let mut list = Vec::new();
        let mut dropped = false;
        if let Ok(s) = std::fs::read_to_string(p) {
            for l in s.lines() {
                let l = l.strip_prefix("#HttpOnly_").unwrap_or(l);
                if l.starts_with('#') || l.trim().is_empty() {
                    continue;
                }
                let f: Vec<&str> = l.split('\t').collect();
                if f.len() < 7 {
                    continue;
                }
                if !f[0].ends_with("youtube.com") {
                    dropped = true;
                    continue;
                }
                list.push(Cookie {
                    domain: f[0].to_string(),
                    path: f[2].to_string(),
                    secure: f[3] == "TRUE",
                    expires: f[4].parse().unwrap_or(0),
                    name: f[5].to_string(),
                    value: f[6].to_string(),
                });
            }
        }
        let mut j = Jar { list, dirty: false };
        if dropped {
            j.save(p);
        }
        j
    }

    pub fn save(&mut self, p: &Path) {
        let mut s = String::from("# Netscape HTTP Cookie File\n");
        for c in &self.list {
            s += &format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                c.domain,
                if c.domain.starts_with('.') { "TRUE" } else { "FALSE" },
                c.path,
                if c.secure { "TRUE" } else { "FALSE" },
                c.expires,
                c.name,
                c.value
            );
        }
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() && std::fs::rename(&tmp, p).is_ok() {
            self.dirty = false;
        }
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.list.iter().find(|c| c.name == name && c.domain.ends_with("youtube.com")).map(|c| c.value.as_str())
    }

    pub fn logged_in(&self) -> bool {
        self.get("SAPISID").or(self.get("__Secure-3PAPISID")).is_some()
    }

    /// En-tête Cookie pour un hôte (youtube.com).
    pub fn header(&self, host: &str) -> String {
        let mut v: Vec<String> = Vec::new();
        for c in &self.list {
            let d = c.domain.trim_start_matches('.');
            if host == d || host.ends_with(&format!(".{d}")) {
                v.push(format!("{}={}", c.name, c.value));
            }
        }
        v.join("; ")
    }

    /// En-tête Authorization (SAPISIDHASH + variantes 1P/3P), comme le site web.
    pub fn auth(&self, origin: &str) -> Option<String> {
        let ts = crate::store::now();
        let h = |v: &str| {
            let mut s = sha1_smol::Sha1::new();
            s.update(format!("{ts} {v} {origin}").as_bytes());
            format!("{ts}_{}", s.digest())
        };
        let sap = self.get("SAPISID").or(self.get("__Secure-3PAPISID"))?;
        let mut out = format!("SAPISIDHASH {}", h(sap));
        if let Some(v) = self.get("__Secure-1PAPISID") {
            out += &format!(" SAPISID1PHASH {}", h(v));
        }
        if let Some(v) = self.get("__Secure-3PAPISID") {
            out += &format!(" SAPISID3PHASH {}", h(v));
        }
        Some(out)
    }

    /// Applique un en-tête Set-Cookie reçu de www.youtube.com (rotation des jetons de session).
    pub fn set_cookie(&mut self, header: &str) {
        let mut parts = header.split(';');
        let Some((name, value)) = parts.next().and_then(|kv| kv.split_once('=')) else { return };
        let (name, value) = (name.trim().to_string(), value.trim().to_string());
        let mut domain = ".youtube.com".to_string();
        let mut path = "/".to_string();
        let mut expires = crate::store::now() + 3600 * 24 * 365;
        let mut secure = false;
        for a in parts {
            let a = a.trim();
            let (k, v) = a.split_once('=').unwrap_or((a, ""));
            match k.to_ascii_lowercase().as_str() {
                "domain" => domain = if v.starts_with('.') { v.to_string() } else { format!(".{v}") },
                "path" => path = v.to_string(),
                "max-age" => {
                    if let Ok(s) = v.parse::<i64>() {
                        expires = (crate::store::now() as i64 + s).max(0) as u64;
                    }
                }
                "secure" => secure = true,
                _ => {}
            }
        }
        if expires <= crate::store::now() {
            self.list.retain(|c| !(c.name == name && c.domain == domain));
        } else if let Some(c) = self.list.iter_mut().find(|c| c.name == name && c.domain == domain) {
            if c.value != value {
                c.value = value;
                c.expires = expires;
                self.dirty = true;
            }
            return;
        } else {
            self.list.push(Cookie { domain, path, secure, expires, name, value });
        }
        self.dirty = true;
    }
}

/// Profils Opera GX / Opera / Chrome / Edge détectés (dossier contenant « Local State »).
pub fn browser_profiles() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    let roam = dirs::data_dir().unwrap_or_default();
    let local = dirs::data_local_dir().unwrap_or_default();
    for (name, p) in [
        ("Opera GX", roam.join("Opera Software").join("Opera GX Stable")),
        ("Opera", roam.join("Opera Software").join("Opera Stable")),
        ("Chrome", local.join("Google").join("Chrome").join("User Data")),
        ("Edge", local.join("Microsoft").join("Edge").join("User Data")),
    ] {
        if p.join("Local State").exists() {
            v.push((name.to_string(), p));
        }
    }
    v
}

fn cookie_file(profile: &Path) -> Option<PathBuf> {
    [profile.join("Default").join("Network").join("Cookies"), profile.join("Network").join("Cookies"), profile.join("Cookies")]
        .into_iter()
        .find(|p| p.exists())
}

/// Processus qui tiennent un fichier ouvert (Restart Manager).
fn lockers(file: &Path) -> Vec<u32> {
    use windows::Win32::System::RestartManager::*;
    use windows::core::{PCWSTR, PWSTR};
    unsafe {
        let mut h = 0u32;
        let mut key = [0u16; 64];
        if RmStartSession(&mut h, None, PWSTR(key.as_mut_ptr())).0 != 0 {
            return vec![];
        }
        let w: Vec<u16> = file.as_os_str().encode_wide_z();
        let _ = RmRegisterResources(h, Some(&[PCWSTR(w.as_ptr())]), None, None);
        let (mut need, mut n, mut r) = (0u32, 0u32, 0u32);
        let _ = RmGetList(h, &mut need, &mut n, None, &mut r);
        let mut info = vec![RM_PROCESS_INFO::default(); need as usize];
        n = need;
        let mut out = vec![];
        if need > 0 && RmGetList(h, &mut need, &mut n, Some(info.as_mut_ptr()), &mut r).0 == 0 {
            out = info[..n as usize].iter().map(|i| i.Process.dwProcessId).collect();
        }
        let _ = RmEndSession(h);
        out
    }
}

trait WideZ {
    fn encode_wide_z(&self) -> Vec<u16>;
}
impl WideZ for std::ffi::OsStr {
    fn encode_wide_z(&self) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        self.encode_wide().chain(std::iter::once(0)).collect()
    }
}

/// (nom de l'exe, pid parent) d'un processus.
fn proc_info(pid: u32) -> Option<(String, u32)> {
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut res = None;
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                if e.th32ProcessID == pid {
                    let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
                    res = Some((String::from_utf16_lossy(&e.szExeFile[..len]), e.th32ParentProcessID));
                    break;
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
        res
    }
}

/// Copie le profil (Local State + Cookies) vers `dst`, en libérant le verrou si besoin.
/// Ne termine qu'un processus enfant du navigateur (le service réseau), jamais la fenêtre principale.
/// Copie le fichier de cookies du navigateur. `allow_kill` = faux : seulement si le fichier
/// n'est pas verrouillé (Opera lancé avec `--disable-features=LockProfileCookieDatabase`, ou
/// fermé) — rien n'est arrêté. Vrai (clic sur « Reconnecter ») : on arrête le service réseau
/// du navigateur, qui se relance seul (coupure d'environ 1 s).
/// Erreur « fichier verrouillé » d'un import silencieux : Tube affiche alors le bandeau « Reconnecter ».
pub const LOCKED: &str = "cookies du navigateur verrouillés";

pub fn copy_profile(profile: &Path, dst: &Path, allow_kill: bool) -> Result<String, String> {
    let src = cookie_file(profile).ok_or("fichier de cookies introuvable")?;
    let _ = std::fs::remove_dir_all(dst);
    let net = dst.join("Default").join("Network");
    std::fs::create_dir_all(&net).map_err(|e| e.to_string())?;
    std::fs::copy(profile.join("Local State"), dst.join("Local State")).map_err(|e| format!("Local State : {e}"))?;
    let target = net.join("Cookies");
    if std::fs::copy(&src, &target).is_ok() {
        return Ok("copie directe, sans rien arrêter".into());
    }
    if !allow_kill {
        return Err(LOCKED.into());
    }
    let mut note = String::new();
    for pid in lockers(&src) {
        let Some((exe, parent)) = proc_info(pid) else { continue };
        let parent_exe = proc_info(parent).map(|p| p.0).unwrap_or_default();
        // le processus principal a pour parent l'Explorateur : on ne touche qu'aux enfants du navigateur
        if !parent_exe.eq_ignore_ascii_case(&exe) {
            return Err(format!("le fichier est tenu par le processus principal de {exe} : ferme le navigateur puis réessaie"));
        }
        unsafe {
            use windows::Win32::System::Threading::*;
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 0);
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
        }
        note = format!("service réseau de {exe} relancé");
    }
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 2500 {
        if std::fs::copy(&src, &target).is_ok() {
            return Ok(note);
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    Err("impossible de copier les cookies (fichier verrouillé)".into())
}

#[cfg(test)]
mod tests {
    use super::Jar;

    #[test]
    fn max_age_zero_supprime() {
        let mut j = Jar::default();
        j.set_cookie("SID=abc; Domain=.youtube.com; Path=/; Max-Age=3600; Secure");
        assert_eq!(j.list.len(), 1);
        j.set_cookie("SID=; Domain=.youtube.com; Max-Age=0");
        assert!(j.list.is_empty());
    }

    #[test]
    fn seulement_youtube() {
        let p = std::env::temp_dir().join(format!("tube-test-cookies-{}.txt", std::process::id()));
        let l = |d: &str, n: &str| format!("{d}\tTRUE\t/\tTRUE\t9999999999\t{n}\tx\n");
        std::fs::write(&p, format!("# Netscape HTTP Cookie File\n{}{}{}", l(".youtube.com", "SID"), l(".google.com", "SID"), l("mail.google.com", "OSID"))).unwrap();
        let j = Jar::load(&p);
        assert_eq!(j.list.len(), 1);
        // le fichier a été réécrit sans les cookies google.com
        assert!(!std::fs::read_to_string(&p).unwrap().contains("google.com"));
        let _ = std::fs::remove_file(&p);
    }
}
