//! Opera GX sans verrou sur ses cookies : ajoute `--disable-features=LockProfileCookieDatabase`
//! à tous ses points de lancement (raccourcis de la barre des tâches, du menu Démarrer et du
//! Bureau, et lancement automatique de Windows dans `HKCU\…\Run`). Tube peut alors relire ses
//! cookies sans arrêter quoi que ce soit.
//!
//! Idempotent (rien n'est modifié si le drapeau est déjà là), arguments existants conservés,
//! chaque fichier ou valeur sauvegardé avant modification. N'agit jamais sur Opera lui-même :
//! le drapeau prend effet à son prochain démarrage. Une mise à jour d'Opera peut recréer ses
//! raccourcis : il suffit de relancer (bouton du bandeau, ou `Tube.exe --opera-drapeau`).
use std::path::{Path, PathBuf};
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::*;
use windows::core::*;

pub const FLAG: &str = "--disable-features=LockProfileCookieDatabase";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn is_opera(target: &str) -> bool {
    let t = target.to_ascii_lowercase();
    t.contains("opera gx") && t.ends_with("opera.exe") || t.ends_with("\\opera gx\\launcher.exe")
}

fn dirs_to_scan() -> Vec<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    let mut v = vec![
        appdata.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"),
        appdata.join(r"Microsoft\Internet Explorer\Quick Launch"),
        appdata.join(r"Microsoft\Windows\Start Menu\Programs"),
    ];
    if let Some(d) = dirs::desktop_dir() {
        v.push(d);
    }
    v
}

fn lnk_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            lnk_files(&p, out, depth - 1);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            out.push(p);
        }
    }
}

/// Applique le drapeau partout ; sauvegardes dans `backup`. Renvoie le compte rendu (une ligne
/// par point de lancement).
pub fn apply(backup: &Path) -> Vec<String> {
    let mut report = Vec::new();
    let _ = std::fs::create_dir_all(backup);
    let stamp = crate::store::now();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    // 1) raccourcis
    let mut files = Vec::new();
    for d in dirs_to_scan() {
        lnk_files(&d, &mut files, 2);
    }
    files.sort();
    files.dedup();
    for f in files {
        match patch_lnk(&f, backup, stamp) {
            Ok(Some(msg)) => report.push(msg),
            Ok(None) => {}
            Err(e) => report.push(format!("{} : erreur {e}", f.display())),
        }
    }
    // 2) lancement automatique de Windows
    match patch_run(backup, stamp) {
        Ok(msg) => report.push(msg),
        Err(e) => report.push(format!("clé Run : {e}")),
    }
    for l in &report {
        crate::store::log(&format!("opera : {l}"));
    }
    report
}

fn patch_lnk(f: &Path, backup: &Path, stamp: u64) -> std::result::Result<Option<String>, String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        let file: IPersistFile = link.cast().map_err(|e| e.to_string())?;
        let wide = HSTRING::from(f.as_os_str());
        file.Load(&wide, STGM_READWRITE).map_err(|e| e.to_string())?;
        let mut target = [0u16; 1024];
        link.GetPath(&mut target, std::ptr::null_mut(), 0).map_err(|e| e.to_string())?;
        let target = String::from_utf16_lossy(&target[..target.iter().position(|c| *c == 0).unwrap_or(0)]);
        if !is_opera(&target) {
            return Ok(None);
        }
        let mut args = [0u16; 4096];
        link.GetArguments(&mut args).map_err(|e| e.to_string())?;
        let args = String::from_utf16_lossy(&args[..args.iter().position(|c| *c == 0).unwrap_or(0)]);
        if args.contains(FLAG) {
            return Ok(Some(format!("{} : déjà en place", f.display())));
        }
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        std::fs::copy(f, backup.join(format!("{stamp}_{name}"))).map_err(|e| format!("sauvegarde : {e}"))?;
        let new = format!("{} {FLAG}", args.trim()).trim().to_string();
        link.SetArguments(&HSTRING::from(new.as_str())).map_err(|e| e.to_string())?;
        file.Save(&wide, true).map_err(|e| e.to_string())?;
        Ok(Some(format!("{} : drapeau ajouté (avant : [{}])", f.display(), args.trim())))
    }
}

fn read_run() -> std::result::Result<Vec<(String, String)>, String> {
    unsafe {
        let mut key = HKEY::default();
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN), None, KEY_READ, &mut key);
        if r.is_err() {
            return Err(format!("ouverture : {r:?}"));
        }
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            let mut name = [0u16; 512];
            let mut nlen = name.len() as u32;
            let mut data = vec![0u8; 8192];
            let mut dlen = data.len() as u32;
            let mut ty = 0u32;
            let r = RegEnumValueW(key, i, Some(PWSTR(name.as_mut_ptr())), &mut nlen, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut dlen));
            if r.is_err() {
                break;
            }
            i += 1;
            if ty != REG_SZ.0 && ty != REG_EXPAND_SZ.0 {
                continue;
            }
            let n = String::from_utf16_lossy(&name[..nlen as usize]);
            let w: Vec<u16> = data[..dlen as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let v = String::from_utf16_lossy(&w).trim_end_matches('\0').to_string();
            out.push((n, v));
        }
        let _ = RegCloseKey(key);
        Ok(out)
    }
}

fn patch_run(backup: &Path, stamp: u64) -> std::result::Result<String, String> {
    let vals = read_run()?;
    let Some((name, value)) = vals.into_iter().find(|(_, v)| {
        let exe = v.trim_start_matches('"').split('"').next().unwrap_or("");
        is_opera(exe)
    }) else {
        return Ok("lancement automatique : Opera GX absent".into());
    };
    if value.contains(FLAG) {
        return Ok(format!("lancement automatique « {name} » : déjà en place"));
    }
    // sauvegarde au format .reg (UTF-16, comme regedit)
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let reg = format!("Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\{RUN}]\r\n\"{}\"=\"{}\"\r\n", esc(&name), esc(&value));
    let mut bytes = vec![0xFF, 0xFE];
    for u in reg.encode_utf16() {
        bytes.extend(u.to_le_bytes());
    }
    std::fs::write(backup.join(format!("{stamp}_cle-run-opera.reg")), bytes).map_err(|e| format!("sauvegarde : {e}"))?;
    let new = format!("{} {FLAG}", value.trim());
    let data: Vec<u8> = new.encode_utf16().chain(Some(0)).flat_map(|u| u.to_le_bytes()).collect();
    unsafe {
        let mut key = HKEY::default();
        let r = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN), None, KEY_SET_VALUE, &mut key);
        if r.is_err() {
            return Err(format!("ouverture en écriture : {r:?}"));
        }
        let r = RegSetValueExW(key, &HSTRING::from(name.as_str()), None, REG_SZ, Some(&data));
        let _ = RegCloseKey(key);
        if r.is_err() {
            return Err(format!("écriture : {r:?}"));
        }
    }
    Ok(format!("lancement automatique « {name} » : drapeau ajouté (avant : [{value}])"))
}
