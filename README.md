# Tube

**A lightweight native YouTube client for Windows, written in Rust, signed in to your own account.**
Your personalised home feed, subscriptions, Watch later, playlists, history and search. Playback goes up to 4K with GPU decoding, and there is an always-on-top mini-player. Watch history is reported back to your account, so recommendations keep learning.

> The user interface is in **French** (the author's language). This README is in English.

## Why

Measured with the same video (Big Buck Bunny, 1440p60) on an i9-9900K + RTX 3070 Ti:

| | Tube (app + yt-dlp worker) | YouTube embed in Chrome |
|---|---|---|
| CPU | **~8 % of one core** | ~15 % of one core |
| Memory | **~450 MB** (2 processes) | ~1 GB working set / ~680 MB private (16 processes) |
| Dropped frames | 0 | — |
| Click → playing | **~0.9 s** when the thumbnail was hovered first, ~4–6 s cold | — |

## How it works

| Need | Choice |
|---|---|
| UI | `egui` + `winit`, rendered on the CPU via `softbuffer`. It uses the same theme and components as *Explorateur*, *Lecteur universel* and *Vigie* |
| Video | **Media Foundation** (`IMFMediaEngine`), drawing straight into a child window on the GPU. It uses Windows' VP9 and AV1 extensions, so there is no ffmpeg or mpv |
| Separate audio and video (DASH) | A media-engine extension combines both streams into one aggregate source (`MFCreateAggregateSource`) |
| Live streams | HLS playlists: `src/live.rs` chains the MPEG-TS segments into one endless byte stream |
| Thumbnails | Decoded at the exact on-screen pixel size, so the CPU renderer copies them instead of resampling each pixel. At 2500×1350, a scrolling frame went from 61 ms to 7 ms |
| HTTP | A custom `IMFByteStream` (`src/http.rs`) downloads 512 KB blocks (up to 4 per request) with `&range=` and reads up to 24 MB ahead. googlevideo throttles open-ended requests to about 70 KB/s, while bounded chunks arrive at 5–17 MB/s |
| Fragmented MP4 (H.264, AV1) | YouTube files carry a `sidx` index but no `mfra`, so Windows would scan every fragment before playing. Tube builds a `mfra` from the `sidx` and serves it after the end of the file (`src/mp4.rs`) |
| Stream URLs | **yt-dlp**, kept loaded in a persistent Python worker, with **deno** for YouTube's JS challenges. It extracts anonymously first, which is about twice as fast with the same formats, and uses the account only when a video requires it |
| Account | InnerTube (`youtubei/v1`) with the browser's cookies and `SAPISIDHASH` |
| Watch history | `playbackTracking` pings, sent the same way the website does |

## Install and first run

Requirements: Windows 10/11 x64, **Python 3** (used only to run yt-dlp), and the *VP9 Video Extensions* / *AV1 Video Extension* from the Microsoft Store.

1. Run `dist\Tube.exe`. On first start it creates `%LOCALAPPDATA%\Tube\bin`, with a Python venv for yt-dlp and `deno.exe`. This takes about a minute.
2. Click **« Se connecter avec Opera GX »** (Opera, Chrome and Edge are also detected). Tube copies the browser's **youtube.com** cookies to `%LOCALAPPDATA%\Tube\cookies.txt` (never the google.com ones, which would open the rest of the Google account). No password is ever typed or read.
   - Chromium browsers lock their cookie file while they run. Tube never interrupts the browser on its own: when the file is locked and the session has expired, a banner offers **Reconnecter** (Tube stops the browser's *network service* child process, copies the file within ~40 ms, and the browser restarts the service by itself: about a 1 s network hiccup).
   - Better: start Opera GX with `--disable-features=LockProfileCookieDatabase`, and the cookies can be read at any time without interrupting anything. `Tube.exe --opera-drapeau` (or the banner's **Réappliquer le réglage d'Opera** button, or `scripts\opera-drapeau.ps1`) adds that flag to Opera GX's shortcuts and to its Windows autostart entry. It changes nothing if the flag is already there, and backs up every shortcut or value before changing it. Run it again after an Opera update that recreates its shortcuts.
   - Browsers rotate session cookies, so Tube re-imports them automatically at startup when its copy is older than 30 minutes, and when YouTube reports the session as signed out (at most once every 10 minutes). These automatic imports never stop anything: they only copy the file when it is not locked.
3. yt-dlp updates itself (`pip -U`) when Tube starts, if the last check is more than 20 hours old. deno is pinned to one version and its download is checked against a SHA-256.

Build from source: `cargo build --release`, which produces `target\release\tube.exe`. Unit tests: `cargo test --release`.

## Features

- Home (personalised), Subscriptions, Watch later, Playlists, History and search. Pasting a YouTube link plays it directly. Feeds scroll infinitely, ads are filtered out and Shorts can be hidden.
- Quality up to 4K (VP9 preferred, AV1/H.264 when they offer more), a quality menu, speed 0.25–3×, chapters on the seek bar, and SponsorBlock segments shown in green.
- Hovering the seek bar shows a preview frame, taken from YouTube's storyboards.
- **Audio language**: the video's *original* audio track is chosen by default. YouTube's auto-dubbed tracks are sometimes served first, so a French video could otherwise play in English. A menu lists every available language.
- **Live streams** play from the live edge. The fast extraction (no HLS) finds no format for a live, so the worker retries with the `mweb` client. They use HLS through a custom byte stream, with the `mweb` client because it serves muxed audio and video.
- **Hover preview**: after 0.65 s on a thumbnail, a muted 480p preview plays inside it. It can be turned off in the settings.
- **Channels**: click a channel name to open its videos; middle-click opens it in a new tab.
- The right-click menu also offers *Pas intéressé* and *Ne pas recommander la chaîne*, which send the same feedback as the website.
- Tabs are restored at the next start, and Ctrl+Shift+T reopens a closed tab.
- **Faster start**: the first videos of the page you are on are extracted in advance (one at a time, clicks first). Hovering a thumbnail for 0.2 s starts its extraction and also downloads the first blocks of the video and audio streams. A click on a prefetched video starts playing in about 0.8 s, against 4–6 s before. Extraction itself takes 2.5–4 s; the faster alternatives (a single client, `player_skip`) were measured but return no usable formats today.
- **Cinema mode** (key C): in full screen and half screen, the black bars take on the colours of the picture's edges. They are sampled 4 times per second, smoothed, and drawn as a gradient fading to black.
- When playback is paused, the frame is shown in dimmed black and white with a pause symbol. While buffering, it keeps its colours and shows a spinner.
- The home page starts with a *Reprendre* row of videos you have started.
- **Audio only** (in the quality menu) downloads no video, which suits music. The background is a real full-quality frame of the video: the last one shown, or one decoded at about 1 s in the background (the HD thumbnail until then).
- `,` / `.` step one frame back or forward, and Ctrl+C copies a `youtu.be` link at the current time.
- **⚙ menu** in the player: quality, speed, audio language, subtitles, sleep timer, audio only, loop, cinema mode and screenshot. Sub-pages work like YouTube's, and a badge shows 4K / QHD / HD.
- **Subtitles** (key V or the CC button): manual tracks, automatic ones and **automatic translation into French**. They are drawn on a dark band over the video, and the last language you chose is reused.
- **Speed remembered per channel**: for example ×1.5 for a podcast and ×1 for music.
- **Music mode** (audio only): the thumbnail stays in colour, with a live spectrum of the sound. Windows 11 *process loopback* captures only Tube's audio, not other apps.
- **Live rewind**: a seek bar covers about the last 35 minutes, with an *EN DIRECT* button to jump back. The mweb playlist only lists 30 s, so older segments are addressed through their `sq` number. Resuming after a long pause restarts at the same delay.
- **Windows media controls**: the keyboard media keys (play/pause, next, previous) work from anywhere. Title, channel and thumbnail appear in Windows' media panel.
- **S** saves the current frame to `Images\Tube`.
- **Streaming under YouTube's random throttling**: googlevideo throttles some requests to about 100 KB/s, chosen at random, and caps some responses at about 1 MB.
  - Tube downloads 512 KB blocks with up to 3 parallel requests per stream and makes each block readable as soon as it arrives.
  - A request still below 300 KB/s after 1.2 s is retried (on the first two attempts), because the next one is often fast.
  - Capped responses simply continue from where they stopped.
  - Extractions run one at a time in priority order (click, then hover, then prefetch), because parallel extractions returned throttled URLs.
  - Measured result: start in about 1.2 s and seeks in about 1–1.4 s.
- **Adaptive quality**: when the measured throughput cannot sustain the current resolution (stalling, video starved while the audio plays, or a start that never completes), Tube steps down a resolution at the same position. Picking a quality in the ⚙ menu overrides this.
- **Most replayed**: hovering the seek bar shows YouTube's replay curve.
- **Search suggestions** appear while typing (↑/↓, Enter).
- Ctrl+← / Ctrl+→ jump to the previous / next chapter. In full screen, a double-click on the left or right quarter goes back / forward 10 s.
- An optional setting hides videos you have already watched.
- Pages stay in memory, with their scroll position, when you navigate back and forth. They are refreshed only with F5 or after 30 minutes.
- **Resume** where you left off, either from local progress or from YouTube's progress when you watched on another device.
- **Mini-player**: borderless and always on top.
  - Drag it anywhere; it snaps to screen edges and can be resized from its edges.
  - Opacity is adjustable from 20 to 100 % (slider, or Ctrl + wheel), and the wheel alone changes the volume.
  - Double-click switches to full screen.
- **Full screen**: the controls overlay hides itself after 2.5 s, along with the cursor.
- **Global shortcuts Win+Alt** work while you code or play. They are intercepted only while a video is loaded; otherwise Windows keeps its own Win+Alt shortcuts.
- **SponsorBlock** uses a hashed-prefix query, so the server never learns which video you watch. Ctrl+Z jumps back before the segment.
- **Tabs**: middle-click a thumbnail, or right-click → *Ouvrir dans un nouvel onglet*. Ctrl+T / Ctrl+W / Ctrl+Tab work too. Only one video plays at a time: switching to a video tab plays that video, and every video resumes where it was left.
- The right-click menu on a thumbnail can also play a video next, add it to the queue, or save it to *Watch later* on your account.
- When playback is paused, the last frame is shown greyed out with a pause symbol. While the network is buffering, a spinner shows instead.
- Playback keeps running while you browse or when the window is minimised. The sidebar card *« En cours »* shows the current video.
- If playback recovers from expired URLs or stalls, Tube re-extracts the streams and resumes at the same position.

### Shortcuts

| Key | Action |
|---|---|
| Space / K | play / pause |
| J / L, ← / → | −10 / +10 s, −5 / +5 s |
| ↑ / ↓, M | volume, mute |
| Click on the video | play / pause |
| F / double-click, I, H | full screen, mini-player, half screen (full height, half width: made for 32:9 monitors) |
| 0 … 9, Home | jump to 0–90 %, restart |
| Shift+, / Shift+. | speed − / + |
| Shift+N | next video in the queue |
| Ctrl+Z | undo the last SponsorBlock skip |
| Ctrl+1 … 5, Ctrl+F or /, Alt+← or Backspace, F5 | sections, search, back, refresh |
| Esc | leave full screen / mini-player |
| , / . , Ctrl+C, Ctrl+K | frame step, copy timestamped link, command palette |
| Ctrl+← / Ctrl+→ | previous / next chapter (or replay peak) |
| Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+Shift+T, middle-click | new tab, close tab, next / previous tab, reopen closed tab, open in a new tab |
| F1 / F2 | shortcut help / playback statistics |
| V, S, C | subtitles, screenshot, cinema mode |
| Ctrl + wheel | thumbnail size (home) · opacity (mini-player) |
| **Win+Alt+Space, ←/→, ↑/↓, M, H, N** | global: play/pause, ±10 s, volume, mini-player, half screen, mute |

## Risks, honestly

- This is an unofficial client, which goes against YouTube's Terms of Service.
- yt-dlp's wiki warns that accounts used through cookies *may* be rate-limited or banned. For watching videos at a normal pace this is rarely reported. Tube extracts anonymously whenever it can.
- YouTube regularly changes its protections, which breaks yt-dlp from time to time. The automatic update usually fixes it within a few days.
- `cookies.txt` is stored in plain text in your Windows profile (youtube.com cookies only). The Opera flag lowers the protection of Opera's own cookie file against other programs; it is your choice to apply it.

## Files

`%APPDATA%\Tube`: `settings.json` (settings, resume positions, open tabs) and `crash.log` (crash reports).

`%LOCALAPPDATA%\Tube`:

| File | Content |
|---|---|
| `cookies.txt` | youtube.com cookies only |
| `extraits.json` | recent extractions still valid (instant start after a restart) |
| `tube.log`, `tube.old.log` | diagnostic log (rotated at 1 MB) |
| `worker.py` | rewritten when the worker starts |
| `bin\` | venv + deno |

Opera shortcut backups go to `sauvegarde-raccourcis-opera` (next to the project when run from a build, otherwise in `%LOCALAPPDATA%\Tube`).

Diagnostics: `TUBE_LOG_EVENTS=1`, `TUBE_LOG_READS=1`, `TUBE_LOG_PAINT=1`, `TUBE_CAPTURE` / `TUBE_CAPTURE_AFTER`, `TUBE_DUMP_URL`. Isolated test mode: `TUBE_TEST=1`, `TUBE_SCRIPT` / `TUBE_QUIT_AFTER` for scripted UI tests (see `src/main.rs`): invisible window, muted, separate data in `%TEMP%	ube-test`.
