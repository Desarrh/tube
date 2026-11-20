"""Worker yt-dlp persistant de Tube : une requête JSON par ligne sur stdin, une réponse par ligne sur stdout.
Garder le module chargé évite ~1,5 s de démarrage par vidéo et réutilise les caches de yt-dlp.
Les extractions passent une à une, par priorité (clic, puis survol, puis préchargement)."""
import json, os, sys, time, threading, queue, itertools, http.cookiejar
sys.stdout.reconfigure(encoding="utf-8")
sys.stdin.reconfigure(encoding="utf-8")
import yt_dlp

COOKIES = os.environ.get("TUBE_COOKIES", "")


def make(auth):
    o = {"quiet": True, "no_warnings": True, "skip_download": True, "noplaylist": True, "noprogress": True,
         # pas de flux du début pour les directs : seulement le bord du direct
         "live_from_start": False,
         # pas de manifeste HLS pour les vidéos (Tube lit les flux https) : ~1,7 s de gagnées par
         # extraction. Les directs passent par l'instance « mweb », qui le garde.
         "extractor_args": {"youtube": {"skip": ["hls"]}}}
    if auth and COOKIES and os.path.isfile(COOKIES):
        o["cookiefile"] = COOKIES
    return yt_dlp.YoutubeDL(o)


GEN = [0]  # incrémenté par « reload » : chaque fil recrée ses instances
local = threading.local()


def inst():
    """Instances yt-dlp du fil courant : anonyme d'abord (≈ 2× plus rapide, mêmes qualités),
    le compte seulement si la vidéo l'exige ; « mweb » pour les directs (vidéo et son mêlés)."""
    if getattr(local, "gen", -1) != GEN[0]:
        local.anon, local.auth = make(False), make(True)
        local.live = yt_dlp.YoutubeDL({"quiet": True, "no_warnings": True, "skip_download": True, "noplaylist": True,
                                       "extractor_args": {"youtube": {"player_client": ["mweb"]}}})
        local.gen = GEN[0]
    return local


NEEDS_AUTH = ("sign in", "age", "members", "private", "confirm", "inappropriate", "premium")


def extract(url):
    i = inst()
    try:
        info = i.anon.extract_info(url, download=False)
    except Exception as e:
        if not any(k in str(e).lower() for k in NEEDS_AUTH):
            raise
        info = i.auth.extract_info(url, download=False)
    keep = ("format_id", "ext", "vcodec", "acodec", "height", "width", "fps", "tbr", "url", "dynamic_range", "format_note", "language", "language_preference")
    fm = [{k: f.get(k) for k in keep} for f in info.get("formats", []) if f.get("protocol") == "https" and f.get("url")]
    # planches d'aperçu (vignettes au survol de la barre de progression) : 160×90 de préférence
    sb = None
    sbs = [f for f in info.get("formats", []) if str(f.get("format_id", "")).startswith("sb") and f.get("fragments")]
    if sbs:
        f = min(sbs, key=lambda f: abs((f.get("width") or 0) - 160))
        sb = {"w": f.get("width"), "h": f.get("height"), "rows": f.get("rows"), "cols": f.get("columns"), "fps": f.get("fps"),
              "urls": [x.get("url") for x in f["fragments"]]}
    # direct : listes HLS ; on préfère les flux où vidéo et son sont mêlés (une seule horloge)
    hls = []
    if info.get("is_live"):
        fmts = info.get("formats", [])
        def muxed(fs):
            return [f for f in fs if str(f.get("protocol", "")).startswith("m3u8") and f.get("url")
                    and f.get("vcodec") not in (None, "none") and f.get("acodec") not in (None, "none")]
        if not muxed(fmts):
            try:
                fmts = i.live.extract_info(url, download=False).get("formats", []) or fmts
            except Exception:
                pass
        hls = [{k: f.get(k) for k in ("format_id", "height", "fps", "tbr", "url", "vcodec", "acodec")} for f in fmts
               if str(f.get("protocol", "")).startswith("m3u8") and f.get("url")]
    # sous-titres (JSON3) : manuels, plus automatiques d'origine et traductions auto fr / en
    subs = []
    orig = info.get("language")
    for lang, fs in (info.get("subtitles") or {}).items():
        u = next((f.get("url") for f in fs if f.get("ext") == "json3"), None)
        if u and lang != "live_chat":
            subs.append({"lang": lang, "name": fs[0].get("name") or lang, "url": u, "auto": False, "translated": False})
    auto = info.get("automatic_captions") or {}
    have = {s["lang"].split("-")[0] for s in subs}
    for key in [k for k in auto if k.endswith("-orig")] + ["fr", "en"]:
        fs = auto.get(key)
        lang = key.replace("-orig", "")
        if not fs or lang.split("-")[0] in have:
            continue
        u = next((f.get("url") for f in fs if f.get("ext") == "json3"), None)
        if u:
            have.add(lang.split("-")[0])
            subs.append({"lang": lang, "name": fs[0].get("name") or lang, "url": u, "auto": True,
                         "translated": not key.endswith("-orig") and lang != orig})
    return {
        "subs": subs,
        # « les plus revus » : intensité de relecture par tranche de la vidéo
        "heatmap": [{"start": h.get("start_time"), "end": h.get("end_time"), "value": h.get("value")} for h in (info.get("heatmap") or [])],
        "id": info.get("id"), "title": info.get("title"), "duration": info.get("duration"),
        "channel": info.get("channel") or info.get("uploader"), "channel_id": info.get("channel_id"),
        "description": info.get("description") or "", "views": info.get("view_count"),
        "upload_date": info.get("upload_date"), "is_live": bool(info.get("is_live")),
        "chapters": [{"start": c.get("start_time"), "title": c.get("title")} for c in (info.get("chapters") or [])],
        "categories": info.get("categories") or [],
        "formats": fm, "storyboard": sb, "hls": hls,
    }


def import_cookies(src, out, browser="opera"):
    from yt_dlp.cookies import extract_cookies_from_browser
    if browser not in ("opera", "chrome", "edge"):
        raise ValueError(f"navigateur non pris en charge : {browser}")
    jar = extract_cookies_from_browser(browser, profile=src)
    keep = http.cookiejar.MozillaCookieJar(out)
    n = 0
    for c in jar:
        # youtube.com seulement : il porte toute la session YouTube (SID, SAPISID, __Secure-3P*…).
        # Les cookies google.com ouvriraient le reste du compte (Gmail, Drive…) : jamais copiés.
        if c.domain.endswith("youtube.com"):
            keep.set_cookie(c)
            n += 1
    keep.save(ignore_discard=True, ignore_expires=True)
    return {"count": n}


out_lock = threading.Lock()
# Extractions UNE à la fois : plusieurs en parallèle donnent des adresses bridées (~100 Ko/s).
# File à priorité : 0 = clic, 1 = survol, 2 = préchargement ; résultats récents réutilisés.
jobs = queue.PriorityQueue()
order = itertools.count()
recent = {}


def reply(rid, out, t):
    out["id"] = rid
    out["secs"] = round(time.time() - t, 2)
    line = json.dumps(out, ensure_ascii=False)
    with out_lock:
        print(line, flush=True)


def handle(req):
    t = time.time()
    rid = req.get("id", 0)
    try:
        cmd = req.get("cmd")
        if cmd == "extract":
            hit = recent.get(req["url"])
            if hit and time.time() - hit[0] < 120:
                res = hit[1]
            else:
                res = extract(req["url"])
                now = time.time()
                for k in [k for k, v in recent.items() if now - v[0] >= 120]:
                    del recent[k]
                recent[req["url"]] = (now, res)
        elif cmd == "import":
            res = import_cookies(req["src"], req["out"], req.get("browser", "opera"))
        elif cmd == "reload":
            GEN[0] += 1
            res = {}
        elif cmd == "version":
            res = {"version": yt_dlp.version.__version__}
        else:
            raise ValueError(f"commande inconnue {cmd}")
        out = {"ok": True, "res": res}
    except Exception as e:
        out = {"ok": False, "error": str(e).replace("[0;31m", "").replace("[0m", "")}
    reply(rid, out, t)


def extractor():
    while True:
        _, _, req = jobs.get()
        handle(req)


threading.Thread(target=extractor, daemon=True).start()
for line in sys.stdin:
    try:
        req = json.loads(line)
    except Exception:
        continue
    if req.get("cmd") == "extract":
        jobs.put((req.get("prio", 0), next(order), req))
    else:
        threading.Thread(target=handle, args=(req,), daemon=True).start()
