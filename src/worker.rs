//! Outils externes : yt-dlp (dans un environnement Python dédié) + deno (défis JS de YouTube).
//! Installés automatiquement au premier lancement dans %LOCALAPPDATA%\Tube\bin, mis à jour une fois par jour.
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

const NO_WINDOW: u32 = 0x0800_0000;

/// deno à version fixe, vérifié avant extraction. Mise à jour : changer les deux constantes
/// (empreinte publiée à côté de l'archive : `….zip.sha256sum`).
const DENO_VERSION: &str = "v2.9.8";
const DENO_SHA256: &str = "cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf";

fn py() -> std::path::PathBuf {
    crate::store::bin_dir().join("py").join("Scripts").join("python.exe")
}
fn deno() -> std::path::PathBuf {
    crate::store::bin_dir().join("deno.exe")
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let o = cmd.creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("échec").to_string())
    }
}

pub fn ready() -> bool {
    py().exists() && deno().exists()
}

/// Installation (bloquant, à lancer dans un fil). `step` reçoit l'étape en cours.
pub fn setup(step: &dyn Fn(&str)) -> Result<(), String> {
    let bin = crate::store::bin_dir();
    std::fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    if !deno().exists() {
        step("Téléchargement de deno (défis JavaScript de YouTube)…");
        let zip = bin.join("deno.zip");
        let url = format!("https://github.com/denoland/deno/releases/download/{DENO_VERSION}/deno-x86_64-pc-windows-msvc.zip");
        run(Command::new("curl.exe").args(["-sSLf", "-o"]).arg(&zip).arg(url))?;
        let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
        let got: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if got != DENO_SHA256 {
            let _ = std::fs::remove_file(&zip);
            return Err(format!("deno : empreinte inattendue ({got}), archive refusée"));
        }
        run(Command::new("tar.exe").arg("-xf").arg(&zip).arg("-C").arg(&bin))?;
        let _ = std::fs::remove_file(zip);
    }
    if !py().exists() {
        step("Création de l'environnement Python pour yt-dlp…");
        let venv = bin.join("py");
        let ok = run(Command::new("py").args(["-3", "-m", "venv"]).arg(&venv)).or_else(|_| run(Command::new("python").args(["-m", "venv"]).arg(&venv)));
        ok.map_err(|e| format!("Python 3 introuvable ({e}). Installe Python depuis python.org puis relance Tube."))?;
        step("Installation de yt-dlp…");
        run(Command::new(py()).args(["-m", "pip", "install", "-q", "--disable-pip-version-check", "yt-dlp[default]"]))?;
    }
    Ok(())
}

/// Mise à jour de yt-dlp (pip -U). Renvoie la nouvelle version si elle a changé.
pub fn update() -> Result<Option<String>, String> {
    let ver = || run(Command::new(py()).args(["-c", "import yt_dlp;print(yt_dlp.version.__version__)"])).map(|s| s.trim().to_string());
    let before = ver()?;
    run(Command::new(py()).args(["-m", "pip", "install", "-q", "-U", "--disable-pip-version-check", "yt-dlp[default]"]))?;
    let after = ver()?;
    Ok(if after != before { Some(after) } else { None })
}

pub type Reply = Result<Value, String>;

struct Proc {
    child: Child,
    stdin: ChildStdin,
}

/// Processus Python persistant ; chaque requête reçoit sa réponse via un canal.
pub struct Worker {
    proc: Mutex<Option<Proc>>,
    pending: Arc<Mutex<HashMap<u64, Sender<Reply>>>>,
    next: Mutex<u64>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Worker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Arc<Worker> {
        Arc::new(Worker { proc: Mutex::new(None), pending: Arc::new(Mutex::new(HashMap::new())), next: Mutex::new(1), wake })
    }

    fn spawn(&self) -> Result<Proc, String> {
        let script = crate::store::dir().join("worker.py");
        std::fs::write(&script, include_str!("worker.py")).map_err(|e| e.to_string())?;
        let path = format!("{};{}", crate::store::bin_dir().display(), std::env::var("PATH").unwrap_or_default());
        // -I : Python isolé (ni variables PYTHON*, ni site utilisateur, ni dossier du script dans
        // sys.path) ; -X utf8 remplace PYTHONIOENCODING, ignoré en mode isolé.
        let mut child = Command::new(py())
            .args(["-I", "-X", "utf8", "-u"])
            .arg(&script)
            .env("PATH", path)
            .env("TUBE_COOKIES", crate::store::cookies_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(NO_WINDOW)
            .spawn()
            .map_err(|e| format!("yt-dlp : {e}"))?;
        let stdin = child.stdin.take().ok_or("stdin")?;
        let stdout = child.stdout.take().ok_or("stdout")?;
        let pending = self.pending.clone();
        let wake = self.wake.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let id = v["id"].as_u64().unwrap_or(0);
                let reply = if v["ok"].as_bool() == Some(true) { Ok(v["res"].clone()) } else { Err(v["error"].as_str().unwrap_or("erreur").to_string()) };
                if let Some(tx) = pending.lock().unwrap().remove(&id) {
                    let _ = tx.send(reply);
                }
                wake();
            }
            // le processus s'est arrêté : on débloque les requêtes en attente
            for (_, tx) in pending.lock().unwrap().drain() {
                let _ = tx.send(Err("yt-dlp s'est arrêté".into()));
            }
            wake();
        });
        Ok(Proc { child, stdin })
    }

    /// Envoie une requête ; la réponse arrivera sur le canal renvoyé.
    /// Si Python doit être (re)lancé, c'est fait dans un fil : jamais dans celui de l'interface.
    pub fn send(self: &Arc<Self>, mut req: Value) -> Receiver<Reply> {
        let (tx, rx) = channel();
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            *n
        };
        req["id"] = json!(id);
        let alive = self.proc.lock().unwrap().as_mut().is_some_and(|p| matches!(p.child.try_wait(), Ok(None)));
        if alive {
            self.deliver(id, req, tx);
        } else {
            let me = self.clone();
            std::thread::spawn(move || me.deliver(id, req, tx));
        }
        rx
    }

    fn deliver(&self, id: u64, req: Value, tx: Sender<Reply>) {
        let mut g = self.proc.lock().unwrap();
        let alive = g.as_mut().is_some_and(|p| matches!(p.child.try_wait(), Ok(None)));
        if !alive {
            match self.spawn() {
                Ok(p) => *g = Some(p),
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        }
        self.pending.lock().unwrap().insert(id, tx.clone());
        let line = format!("{}\n", req);
        if let Some(p) = g.as_mut() {
            if p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()).is_err() {
                self.pending.lock().unwrap().remove(&id);
                let _ = tx.send(Err("yt-dlp ne répond pas".into()));
                *g = None;
            }
        }
    }

    /// `prio` : 0 = clic, 1 = survol, 2 = préchargement.
    pub fn extract(self: &Arc<Self>, video_id: &str, prio: u8) -> Receiver<Reply> {
        self.send(json!({"cmd": "extract", "prio": prio, "url": format!("https://www.youtube.com/watch?v={video_id}")}))
    }

    /// Arrête le processus (il sera relancé à la prochaine requête, par ex. après une mise à jour).
    pub fn restart(&self) {
        if let Some(mut p) = self.proc.lock().unwrap().take() {
            let _ = p.child.kill();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.restart();
    }
}
