#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(deprecated)]

mod api;
mod app;
mod cookies;
mod hotkeys;
mod live;
mod viz;
mod smtc;
mod http;
mod mp4;
mod player;
mod sponsor;
mod store;
mod thumbs;
mod ui;
mod vidwin;
mod worker;

use app::{App, Mode, WinOp};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

/// Interface rendue par le processeur (comme l'Explorateur et Vigie) ; seule la vidéo passe par
/// la carte graphique, dans une sous-fenêtre où Media Foundation dessine directement.
struct Gfx {
    window: Rc<Window>,
    egui: egui_winit::State,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    renderer: EguiSoftwareRender,
}

enum UserEvent {
    RepaintAt(Instant),
    Hotkey(hotkeys::Hk),
    /// arguments reçus d'une 2ᵉ instance (lien YouTube ou recherche)
    Open(Vec<String>),
}

struct Runner {
    ctx: egui::Context,
    proxy: EventLoopProxy<UserEvent>,
    gfx: Option<Gfx>,
    app: Option<App>,
    next_repaint: Option<Instant>,
    next_tick: Option<Instant>,
    last_frame: Instant,
    normal_rect: Option<(i32, i32, i32, i32, bool)>,
    moved_at: Option<Instant>,
    open_arg: Option<String>,
    script_done: Vec<usize>,
    paints: (u32, f64),
    paints_at: Instant,
}

fn hwnd_of(w: &Window) -> isize {
    match w.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

mod win {
    use windows::Win32::Foundation::{COLORREF, HWND, RECT};
    use windows::Win32::Graphics::Gdi::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    pub fn rect(h: isize) -> (i32, i32, i32, i32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(HWND(h as *mut _), &mut r);
        }
        (r.left, r.top, r.right - r.left, r.bottom - r.top)
    }
    pub fn set_rect(h: isize, r: (i32, i32, i32, i32)) {
        unsafe {
            let _ = SetWindowPos(HWND(h as *mut _), None, r.0, r.1, r.2, r.3, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
    pub fn monitor_rect(h: isize) -> (i32, i32, i32, i32) {
        unsafe {
            let m = MonitorFromWindow(HWND(h as *mut _), MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(m, &mut mi);
            let r = mi.rcMonitor;
            (r.left, r.top, r.right - r.left, r.bottom - r.top)
        }
    }
    pub fn work_area(h: isize) -> (i32, i32, i32, i32) {
        unsafe {
            let m = MonitorFromWindow(HWND(h as *mut _), MONITOR_DEFAULTTONEAREST);
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(m, &mut mi);
            let r = mi.rcWork;
            (r.left, r.top, r.right - r.left, r.bottom - r.top)
        }
    }
    /// Capture de débogage : `screen` = ce qui est réellement composé à l'écran (opacité comprise),
    /// sinon la fenêtre elle-même (PrintWindow, contenu GPU inclus).
    pub fn shot(h: isize, path: &str, screen: bool) {
        unsafe {
            let (x, y, w, hh) = rect(h);
            let sdc = GetDC(None);
            let mdc = CreateCompatibleDC(Some(sdc));
            let bmp = CreateCompatibleBitmap(sdc, w, hh);
            let old = SelectObject(mdc, bmp.into());
            if screen {
                let _ = BitBlt(mdc, 0, 0, w, hh, Some(sdc), x, y, SRCCOPY);
            } else {
                let _ = windows::Win32::Storage::Xps::PrintWindow(HWND(h as *mut _), mdc, windows::Win32::Storage::Xps::PRINT_WINDOW_FLAGS(2));
            }
            let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -hh, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
            let mut px = vec![0u8; (w * hh * 4) as usize];
            GetDIBits(mdc, bmp, 0, hh as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
            SelectObject(mdc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mdc);
            ReleaseDC(None, sdc);
            for c in px.chunks_exact_mut(4) {
                c.swap(0, 2);
                c[3] = 255;
            }
            let _ = image::save_buffer(path, &px, w as u32, hh as u32, image::ColorType::Rgba8);
        }
    }

    pub fn opacity(h: isize, a: f32) {
        unsafe {
            let hw = HWND(h as *mut _);
            let ex = GetWindowLongPtrW(hw, GWL_EXSTYLE);
            if a >= 0.999 {
                SetWindowLongPtrW(hw, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
            } else {
                SetWindowLongPtrW(hw, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
                let _ = SetLayeredWindowAttributes(hw, COLORREF(0), (a.clamp(0.1, 1.0) * 255.0) as u8, LWA_ALPHA);
            }
        }
    }
}

impl Runner {
    fn paint(&mut self) {
        let (Some(gfx), Some(app)) = (self.gfx.as_mut(), self.app.as_mut()) else { return };
        let size = gfx.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        self.last_frame = Instant::now();
        let t_paint = Instant::now();
        let raw = gfx.egui.take_egui_input(&gfx.window);
        let out = self.ctx.run_ui(raw, |ui| {
            app.tick();
            app.handle_keys(ui.ctx());
            app.draw(ui);
        });
        gfx.egui.handle_platform_output(&gfx.window, out.platform_output);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        if gfx.surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = gfx.surface.buffer_mut() else { return };
        let bg = if app.mode == Mode::Normal { app.pal.bg } else { egui::Color32::BLACK };
        buf.fill(((bg.r() as u32) << 16) | ((bg.g() as u32) << 8) | bg.b() as u32);
        {
            let px: &mut [[u8; 4]] = bytemuck::cast_slice_mut(&mut buf[..]);
            let mut target = BufferMutRef::new(px, w.get() as usize, h.get() as usize);
            gfx.renderer.render(&mut target, &prims, &out.textures_delta, out.pixels_per_point);
        }
        if let Ok(path) = std::env::var("TUBE_CAPTURE") {
            if app.feeds.values().any(|f| !f.items.is_empty()) && std::env::var("TUBE_CAPTURE_AFTER").ok().and_then(|s| s.parse::<u64>().ok()).is_none_or(|s| app_age() > s) {
                let mut rgba = Vec::with_capacity(buf.len() * 4);
                for &px in buf.iter() {
                    rgba.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, px as u8, 255]);
                }
                let _ = image::save_buffer(&path, &rgba, w.get(), h.get(), image::ColorType::Rgba8);
                env_shim::remove_var_safe("TUBE_CAPTURE");
            }
            self.ctx.request_repaint_after(Duration::from_millis(300));
        }
        let _ = buf.present();
        if self.paints.0 == 0 && self.paints_at.elapsed().as_secs() < 60 {
            store::log(&format!("première image à {} ms", START.get().map(|s| s.elapsed().as_millis()).unwrap_or(0)));
        }
        self.paints.0 += 1;
        self.paints.1 += t_paint.elapsed().as_secs_f64();
        if self.paints_at.elapsed().as_secs() >= 10 && std::env::var_os("TUBE_LOG_PAINT").is_some() {
            store::log(&format!("interface : {} images en 10 s, {:.1} ms en moyenne", self.paints.0, self.paints.1 * 1000.0 / self.paints.0.max(1) as f64));
            self.paints = (0, 0.0);
            self.paints_at = Instant::now();
        }
        self.place_video();
        self.apply_ops();
    }

    fn place_video(&mut self) {
        let Some(app) = self.app.as_mut() else { return };
        let target = app.video_px.filter(|_| app.watch.as_ref().is_some_and(|w| w.player.is_some()));
        let cuts = app.video_cuts.clone();
        let Some(vw) = app.vidwin.as_mut() else { return };
        match target {
            Some(r) if r.2 > 4 && r.3 > 4 => {
                let resized = vw.place(r, &cuts);
                let (w, h) = vw.size();
                if let Some(p) = app.watch.as_mut().and_then(|w| w.player.as_mut()) {
                    if resized || app.output_dirty {
                        p.set_output(w, h);
                    }
                }
                app.output_dirty = false;
            }
            _ => vw.hide(),
        }
    }

    fn apply_ops(&mut self) {
        let (Some(gfx), Some(app)) = (self.gfx.as_ref(), self.app.as_mut()) else { return };
        let win = &gfx.window;
        let hwnd = app.hwnd;
        let test = store::test_mode();
        for op in std::mem::take(&mut app.ops) {
            // test : plein écran, demi-écran et mini-lecteur simulés hors écran (taille seulement),
            // jamais au premier plan ni avec le focus
            if test {
                match op {
                    // pas de focus, et l'opacité de test (0) ne doit pas être modifiée
                    WinOp::Focus | WinOp::Opacity(_) | WinOp::Drag | WinOp::Resize(_) => continue,
                    WinOp::Fullscreen(on) | WinOp::Half(on) | WinOp::Mini(on) => {
                        let (mw, mh) = el_monitor_size(win);
                        let (w, h) = match op {
                            WinOp::Fullscreen(_) => (mw, mh),
                            WinOp::Half(_) => (mw / 2, mh),
                            _ => (480, 270),
                        };
                        let target = match op {
                            WinOp::Fullscreen(_) => Mode::Full,
                            WinOp::Half(_) => Mode::Half,
                            _ => Mode::Mini,
                        };
                        if on {
                            win::set_rect(hwnd, (0, 0, w, h));
                            app.mode = target;
                        } else if app.mode == target {
                            win::set_rect(hwnd, (0, 0, 1480, 900));
                            app.mode = Mode::Normal;
                        }
                        app.output_dirty = true;
                        continue;
                    }
                    _ => {}
                }
            }
            match op {
                WinOp::Title(t) => win.set_title(&t),
                WinOp::Focus => win.focus_window(),
                WinOp::Drag => {
                    let _ = win.drag_window();
                }
                WinOp::Resize(d) => {
                    let _ = win.drag_resize_window(d);
                }
                WinOp::Opacity(a) => {
                    if app.mode == Mode::Mini {
                        win::opacity(hwnd, a);
                    }
                }
                WinOp::Fullscreen(on) => {
                    if on && app.mode != Mode::Full {
                        if matches!(app.mode, Mode::Mini | Mode::Half) {
                            exit_mini(win, app, &mut self.normal_rect);
                        }
                        win.set_fullscreen(Some(Fullscreen::Borderless(None)));
                        app.mode = Mode::Full;
                        app.overlay_until = Instant::now() + Duration::from_secs(2);
                    } else if !on && app.mode == Mode::Full {
                        win.set_fullscreen(None);
                        app.mode = Mode::Normal;
                        if app.view != app::View::Watch && app.watch.is_some() {
                            app.view = app::View::Watch;
                        }
                    }
                    app.output_dirty = true;
                }
                WinOp::Half(on) => {
                    if on && app.mode != Mode::Half {
                        match app.mode {
                            // la position normale est déjà mémorisée
                            Mode::Mini => {
                                save_mini_rect(app);
                                win::opacity(hwnd, 1.0);
                            }
                            Mode::Full => {
                                win.set_fullscreen(None);
                                self.normal_rect = Some(normal_of(win));
                            }
                            _ => self.normal_rect = Some(normal_of(win)),
                        }
                        win.set_decorations(false);
                        win.set_window_level(WindowLevel::AlwaysOnTop);
                        let (mx, my, mw, mh) = win::monitor_rect(hwnd);
                        let (x, _, w, _) = win::rect(hwnd);
                        let hw = mw / 2;
                        let right = x + w / 2 > mx + mw / 2;
                        win::set_rect(hwnd, (if right { mx + mw - hw } else { mx }, my, hw, mh));
                        app.mode = Mode::Half;
                        app.overlay_until = Instant::now() + Duration::from_secs(2);
                        app.output_dirty = true;
                    } else if !on && app.mode == Mode::Half {
                        exit_mini(win, app, &mut self.normal_rect);
                        if app.watch.is_some() {
                            app.view = app::View::Watch;
                        }
                    }
                }
                WinOp::Mini(on) => {
                    if on && app.mode != Mode::Mini {
                        if app.mode == Mode::Full {
                            win.set_fullscreen(None);
                        }
                        if app.mode != Mode::Half {
                            self.normal_rect = Some(normal_of(win));
                        }
                        win.set_decorations(false);
                        win.set_window_level(WindowLevel::AlwaysOnTop);
                        let wa = win::work_area(hwnd);
                        let scale = win.scale_factor();
                        // (une taille aberrante — fenêtre aimantée par Windows sur une moitié d'écran — est ignorée)
                        let mr = app.s.mini_rect.filter(|m| m.2 > 100 && m.3 > 60 && m.2 <= wa.2 * 6 / 10 && m.3 <= wa.3 * 6 / 10).unwrap_or_else(|| {
                            let (mw, mh) = ((480.0 * scale) as i32, (270.0 * scale) as i32);
                            let m = (16.0 * scale) as i32;
                            (wa.0 + wa.2 - mw - m, wa.1 + wa.3 - mh - m, mw, mh)
                        });
                        win::set_rect(hwnd, mr);
                        app.mode = Mode::Mini;
                        win::opacity(hwnd, app.s.mini_opacity);
                        app.output_dirty = true;
                    } else if !on && app.mode == Mode::Mini {
                        exit_mini(win, app, &mut self.normal_rect);
                        if app.watch.is_some() {
                            app.view = app::View::Watch;
                        }
                        win.focus_window();
                    }
                }
            }
        }
    }

    /// Scénario de test (TUBE_SCRIPT="5:shot:a.png;8:mini;12:opacity:0.6;14:screen:b.png;16:full;20:normal;22:seek:120;24:key:space")
    fn script(&mut self) {
        let Ok(sc) = std::env::var("TUBE_SCRIPT") else { return };
        let age = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let Some(app) = self.app.as_mut() else { return };
        for (i, step) in sc.split(';').enumerate() {
            let mut f = step.splitn(3, ':');
            let (Some(t), Some(cmd)) = (f.next().and_then(|t| t.parse::<f64>().ok()), f.next()) else { continue };
            let arg = f.next().unwrap_or("");
            if age < t || self.script_done.contains(&i) {
                continue;
            }
            self.script_done.push(i);
            store::log(&format!("script {t}s : {cmd} {arg}"));
            match cmd {
                "shot" => win::shot(app.hwnd, arg, false),
                // (« screen » capture désormais la fenêtre elle-même : jamais l'écran réel)
                "screen" => win::shot(app.hwnd, arg, false),
                "mini" => app.ops.push(WinOp::Mini(true)),
                "full" => app.ops.push(WinOp::Fullscreen(true)),
                "half" => app.ops.push(WinOp::Half(true)),
                "normal" => {
                    app.ops.push(WinOp::Mini(false));
                    app.ops.push(WinOp::Half(false));
                    app.ops.push(WinOp::Fullscreen(false));
                }
                "opacity" => {
                    app.s.mini_opacity = arg.parse().unwrap_or(1.0);
                    app.ops.push(WinOp::Opacity(app.s.mini_opacity));
                }
                "seek" => app.seek_to(arg.parse().unwrap_or(0.0)),
                "play" => {
                    let n: usize = arg.parse().unwrap_or(0);
                    let items = app.feeds.get("FEwhat_to_watch").map(|f| f.items.clone()).unwrap_or_default();
                    if let Some(it) = items.iter().filter(|i| i.kind == api::Kind::Video && !i.live).nth(n).cloned() {
                        app.s.positions.remove(&it.id);
                        app.open(&it, None);
                    }
                }
                "newtab" => {
                    let n: usize = arg.parse().unwrap_or(0);
                    let items = app.feeds.get("FEwhat_to_watch").map(|f| f.items.clone()).unwrap_or_default();
                    if let Some(it) = items.iter().filter(|i| i.kind == api::Kind::Video && !i.live).nth(n).cloned() {
                        app.open_in_tab(&it);
                    }
                }
                "tab" => app.switch_tab(arg.parse().unwrap_or(0)),
                "menu" => app.menu = Some((ui::Menu::Gear, egui::Rect::from_min_size(egui::pos2(900.0, 700.0), egui::vec2(30.0, 30.0)))),
                "subs" => app.toggle_subs(),
                // tests Y1–Y3 (aucune vraie réduction de fenêtre : on simule l'état)
                "bgmin" => app.minimized = Some(Instant::now() - Duration::from_secs(6)),
                "bgrestore" => app.minimized = None,
                "nearend" => {
                    if let Some(p) = app.player() {
                        let d = p.duration;
                        p.seek((d - 25.0).max(0.0));
                    }
                }
                "chapnext" => {
                    app.chapter_jump(1);
                    let hl = app.watch.as_ref().map(app::highlights).unwrap_or_default();
                    let ch = app.watch.as_ref().and_then(|w| w.info.as_ref()).and_then(|i| i["chapters"].as_array().map(|a| a.len())).unwrap_or(0);
                    store::log(&format!("après Ctrl+→ : {:?} ({}) · chapitres {ch} · moments forts {hl:?}", app.player().map(|p| p.time()), app.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()));
                }
                "autoplay" => app.s.autoplay_next = true,
                "sugglog" => store::log(&format!("suggestions : {:?}", app.search_state.suggestions.iter().map(|s| s.text.clone()).collect::<Vec<_>>())),
                "palette" => app.palette.open_with(arg.to_string()),
                "type" => {
                    app.search = arg.to_string();
                    app.focus_search = true;
                    app.suggest();
                }
                "audioonly" => app.set_audio_only(arg != "0"),
                "liveseek" => app.live_seek(arg.parse().unwrap_or(0.0)),
                "behind" => store::log(&format!("retard sur le direct : {:?}", app.live_behind())),
                "scroll" => app.dbg_scroll.0 = arg.parse().unwrap_or(0.0),
                "cardw" => app.s.card_w = arg.parse().unwrap_or(300.0),
                "hover" => {
                    let n: usize = arg.parse().unwrap_or(0);
                    let items = app.feeds.get("FEwhat_to_watch").map(|f| f.items.clone()).unwrap_or_default();
                    if let Some(it) = items.iter().filter(|i| i.kind == api::Kind::Video && !i.live).nth(n).cloned() {
                        app.extract(&it.id);
                    }
                }
                "toggle" => app.toggle(),
                "view" => match arg {
                    "home" => app.go(app::View::Home),
                    "subs" => app.go(app::View::Subs),
                    "later" => app.go(app::View::Later),
                    "playlists" => app.go(app::View::Playlists),
                    "history" => app.go(app::View::History),
                    "settings" => app.go(app::View::Settings),
                    "watch" => app.go(app::View::Watch),
                    q => app.go(app::View::Search(q.to_string())),
                },
                "stats" => store::log(&app.stats_line().unwrap_or_default()),
                "minimize" => {
                    if let Some(g) = &self.gfx {
                        g.window.set_minimized(true);
                    }
                }
                "restore" => {
                    if let Some(g) = &self.gfx {
                        g.window.set_minimized(false);
                    }
                }
                "time" => store::log(&format!("position {:.1} s, lecture {}", app.watch.as_ref().and_then(|w| w.player.as_ref()).map(|p| p.time()).unwrap_or(-1.0), app.watch.as_ref().and_then(|w| w.player.as_ref()).is_some_and(|p| p.wants_play()))),
                "hk" => hotkeys::simulate(match arg {
                    "right" => windows::Win32::UI::Input::KeyboardAndMouse::VK_RIGHT,
                    "left" => windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT,
                    "up" => windows::Win32::UI::Input::KeyboardAndMouse::VK_UP,
                    "m" => windows::Win32::UI::Input::KeyboardAndMouse::VK_M,
                    _ => windows::Win32::UI::Input::KeyboardAndMouse::VK_SPACE,
                }),
                _ => {}
            }
            app.overlay_until = Instant::now() + Duration::from_secs(3);
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
        }
        self.next_tick = Some(self.next_tick.map_or(Instant::now() + Duration::from_millis(200), |t| t.min(Instant::now() + Duration::from_millis(200))));
    }

    /// Mini-lecteur lâché près d'un bord de l'écran : il s'y aimante.
    fn snap(&mut self) {
        let Some(app) = self.app.as_mut() else { return };
        if app.mode != Mode::Mini {
            return;
        }
        let (x, y, w, h) = win::rect(app.hwnd);
        let wa = win::work_area(app.hwnd);
        let m = 14;
        let th = 60;
        let mut nx = x;
        let mut ny = y;
        if (x - wa.0).abs() < th {
            nx = wa.0 + m;
        }
        if ((wa.0 + wa.2) - (x + w)).abs() < th {
            nx = wa.0 + wa.2 - w - m;
        }
        if (y - wa.1).abs() < th {
            ny = wa.1 + m;
        }
        if ((wa.1 + wa.3) - (y + h)).abs() < th {
            ny = wa.1 + wa.3 - h - m;
        }
        if (nx, ny) != (x, y) {
            win::set_rect(app.hwnd, (nx, ny, w, h));
        }
        if w <= wa.2 * 6 / 10 && h <= wa.3 * 6 / 10 {
            app.s.mini_rect = Some((nx, ny, w, h));
        }
    }

    fn save(&mut self) {
        if let Some(app) = self.app.as_mut() {
            if let Some(g) = &self.gfx {
                if app.mode == Mode::Normal && !g.window.is_maximized() {
                    let sz = g.window.inner_size().to_logical::<f32>(g.window.scale_factor());
                    if sz.width > 300.0 {
                        app.s.window = Some((sz.width, sz.height));
                    }
                }
            }
            app.store_tabs();
            app.close_watch();
            app.s.save();
        }
    }
}

/// Mémorise la position normale de la fenêtre (avant mini-lecteur ou demi-écran).
fn normal_of(win: &Window) -> (i32, i32, i32, i32, bool) {
    let maxed = win.is_maximized();
    if maxed {
        win.set_maximized(false);
    }
    let pos = win.outer_position().unwrap_or_default();
    let sz = win.inner_size();
    (pos.x, pos.y, sz.width as i32, sz.height as i32, maxed)
}

/// Revient à la fenêtre normale depuis le mini-lecteur ou le demi-écran.
/// Mémorise la position du mini-lecteur, sauf taille aberrante (aimantation Windows sur une moitié d'écran).
fn save_mini_rect(app: &mut App) {
    let r = win::rect(app.hwnd);
    let wa = win::work_area(app.hwnd);
    if r.2 <= wa.2 * 6 / 10 && r.3 <= wa.3 * 6 / 10 {
        app.s.mini_rect = Some(r);
    }
}

fn exit_mini(win: &Window, app: &mut App, normal: &mut Option<(i32, i32, i32, i32, bool)>) {
    if app.mode == Mode::Mini {
        save_mini_rect(app);
    }
    win::opacity(app.hwnd, 1.0);
    win.set_window_level(WindowLevel::Normal);
    win.set_decorations(true);
    // (x, y) = position extérieure, (w, h) = taille intérieure, en pixels physiques
    if let Some((x, y, w, h, maxed)) = normal.take() {
        let _ = win.request_inner_size(winit::dpi::PhysicalSize::new(w as u32, h as u32));
        win.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        if maxed {
            win.set_maximized(true);
        }
    }
    app.mode = Mode::Normal;
    app.output_dirty = true;
}

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
fn app_age() -> u64 {
    START.get().map(|s| s.elapsed().as_secs()).unwrap_or(0)
}

mod env_shim {
    pub fn remove_var_safe(k: &str) {
        // SAFETY : appelé depuis le fil principal uniquement (mode capture de débogage).
        unsafe { std::env::remove_var(k) }
    }
}

impl ApplicationHandler<UserEvent> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let s = store::Settings::load();
        let (mut w, mut h) = s.window.unwrap_or((1480.0, 900.0));
        // jamais plus grand que l'écran (taille enregistrée aberrante, écran changé…)
        if let Some(m) = el.primary_monitor() {
            let ms = m.size().to_logical::<f32>(m.scale_factor());
            w = w.min(ms.width * 0.9);
            h = h.min(ms.height * 0.88);
        }
        let attrs = Window::default_attributes()
            .with_title("Tube")
            .with_inner_size(LogicalSize::new(w.max(640.0), h.max(400.0)))
            .with_min_inner_size(LogicalSize::new(200.0, 112.0))
            .with_window_icon(window_icon(include_bytes!("../assets/icon32.png")));
        // grande icône (barre des tâches, Alt+Tab) : pleine définition au lieu d'une version agrandie
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_taskbar_icon(window_icon(include_bytes!("../assets/icon64.png")))
        };
        // test : invisible (opacité 0, voir win::no_activate), sans focus, sans bouton dans la barre des tâches.
        // (Une fenêtre hors écran ne reçoit plus de demandes de dessin : elle resterait blanche.)
        let attrs = if store::test_mode() {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_position(winit::dpi::PhysicalPosition::new(0, 0)).with_active(false).with_skip_taskbar(true)
        } else {
            attrs
        };
        let window = Rc::new(el.create_window(attrs).expect("création de la fenêtre"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer");
        let surface = softbuffer::Surface::new(&context, window.clone()).expect("surface");
        let egui = egui_winit::State::new(self.ctx.clone(), egui::ViewportId::ROOT, &*window, Some(window.scale_factor() as f32), None, Some(8192));
        let renderer = EguiSoftwareRender::new(ColorFieldOrder::Bgra);
        let hwnd = hwnd_of(&window);
        if store::test_mode() {
            // règle n° 0 : invisible (opacité 0), sans focus, traversée par la souris (socle)
            socle::testmode::hide(hwnd);
        }
        player::init();
        let proxy = self.proxy.clone();
        let mut app = App::new(&self.ctx, s, hwnd, move |hk| {
            let _ = proxy.send_event(UserEvent::Hotkey(hk));
        });
        let proxy2 = self.proxy.clone();
        app.smtc = if store::test_mode() { None } else { smtc::Smtc::new(hwnd, move |b| {
            let hk = match b {
                smtc::Btn::PlayPause => hotkeys::Hk::Toggle,
                smtc::Btn::Next => hotkeys::Hk::Next,
                smtc::Btn::Previous => hotkeys::Hk::Prev,
            };
            let _ = proxy2.send_event(UserEvent::Hotkey(hk));
        }) }; // test : pas de panneau média (il capterait les touches multimédia de l'utilisateur)
        if std::env::var_os("TUBE_SCRIPT").is_none() {
            app.restore_tabs();
        }
        if let Some(arg) = self.open_arg.take() {
            app.open_arg(&arg);
        }
        if let Some(c) = socle::crash::pending("Tube") {
            app.toast(format!("Tube s'était arrêté : {c}"));
        }
        self.app = Some(app);
        window.request_redraw();
        self.gfx = Some(Gfx { window, egui, _context: context, surface, renderer });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let resp = gfx.egui.on_window_event(&gfx.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                self.save();
                el.exit();
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Occluded(o) => {
                if let Some(app) = self.app.as_mut() {
                    app.occluded = o;
                }
                if !o {
                    gfx.window.request_redraw();
                }
            }
            WindowEvent::Moved(_) => {
                if self.app.as_ref().is_some_and(|a| a.mode == Mode::Mini) {
                    self.moved_at = Some(Instant::now());
                }
            }
            WindowEvent::Resized(_) => {
                if let Some(app) = self.app.as_mut() {
                    app.occluded = gfx.window.is_minimized().unwrap_or(false);
                    // Y3 : instant de la réduction (None = fenêtre visible)
                    app.minimized = if app.occluded { app.minimized.or(Some(Instant::now())) } else { None };
                    if app.mode == Mode::Mini {
                        self.moved_at = Some(Instant::now());
                    }
                }
                gfx.window.request_redraw();
            }
            _ => {
                if resp.repaint {
                    gfx.window.request_redraw();
                }
            }
        }
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::RepaintAt(t) => {
                if t <= Instant::now() {
                    if let Some(g) = &self.gfx {
                        g.window.request_redraw();
                    }
                } else {
                    self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
                }
            }
            UserEvent::Open(args) => {
                if let Some(app) = self.app.as_mut() {
                    let a = args.join(" ");
                    if !a.trim().is_empty() {
                        app.open_arg(&a);
                    }
                    if !store::test_mode() {
                        app.ops.push(app::WinOp::Focus);
                    }
                }
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
            UserEvent::Hotkey(hk) => {
                if let Some(app) = self.app.as_mut() {
                    use hotkeys::Hk;
                    match hk {
                        Hk::Toggle => app.toggle(),
                        Hk::Back => app.seek_by(-10.0),
                        Hk::Fwd => app.seek_by(10.0),
                        Hk::VolUp => app.change_volume(0.05),
                        Hk::VolDown => app.change_volume(-0.05),
                        Hk::Mute => app.toggle_mute(),
                        Hk::Mini => app.toggle_mini(),
                        Hk::Half => app.toggle_half(),
                        Hk::Next => {
                            app.next_in_queue();
                        }
                        Hk::Prev => app.seek_to(0.0),
                    }
                    app.overlay_until = Instant::now() + Duration::from_millis(1500);
                }
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if let Some(q) = std::env::var("TUBE_QUIT_AFTER").ok().and_then(|s| s.parse::<u64>().ok()) {
            if app_age() >= q {
                self.save();
                el.exit();
                return;
            }
            self.next_tick = Some(self.next_tick.map_or(now + Duration::from_secs(1), |t| t.min(now + Duration::from_secs(1))));
        }
        self.script();
        if let Some(t) = self.moved_at {
            if now.duration_since(t) > Duration::from_millis(300) {
                self.moved_at = None;
                self.snap();
            }
        }
        // logique (lecteur, historique, SponsorBlock) même sans redessiner l'interface
        let mut want_frame = None;
        if let Some(app) = self.app.as_mut() {
            if self.next_tick.is_none_or(|t| t <= now) {
                let d = app.tick();
                self.next_tick = d.map(|d| now + d);
            }
            if std::env::var_os("TUBE_SCRIPT").is_some() || std::env::var_os("TUBE_QUIT_AFTER").is_some() {
                let t = now + Duration::from_millis(200);
                self.next_tick = Some(self.next_tick.map_or(t, |n| n.min(t)));
            }
            let playing = app.watch.as_ref().and_then(|w| w.player.as_ref()).is_some_and(|p| p.playing());
            if playing && !app.occluded {
                // le temps affiché avance : une image par seconde suffit (la vidéo, elle, est dessinée par le GPU)
                want_frame = match app.mode {
                    Mode::Normal => Some(Duration::from_millis(1000)),
                    _ if now < app.overlay_until => Some(Duration::from_millis(500)),
                    _ => None,
                };
            }
        }
        if let Some(d) = want_frame {
            if now.duration_since(self.last_frame) >= d {
                if let Some(g) = &self.gfx {
                    g.window.request_redraw();
                }
            } else {
                let t = self.last_frame + d;
                self.next_repaint = Some(self.next_repaint.map_or(t, |n| n.min(t)));
            }
        }
        let mut wake = [self.next_repaint, self.next_tick, self.moved_at.map(|t| t + Duration::from_millis(320))].into_iter().flatten().min();
        if let Some(t) = self.next_repaint.filter(|t| *t <= now) {
            let _ = t;
            self.next_repaint = None;
            if let Some(g) = &self.gfx {
                g.window.request_redraw();
            }
            wake = [self.next_tick].into_iter().flatten().min();
        }
        el.set_control_flow(match wake {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }
}

/// Taille du moniteur principal (pixels) : plein écran simulé du mode test.
fn el_monitor_size(win: &Window) -> (i32, i32) {
    win.primary_monitor().or_else(|| win.current_monitor()).map(|m| (m.size().width as i32, m.size().height as i32)).unwrap_or((1920, 1080))
}

fn window_icon(png: &[u8]) -> Option<winit::window::Icon> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn main() {
    let _ = START.set(Instant::now());
    store::log("démarrage");
    // journal de plantage %APPDATA%\Tube\crash.log, signalé au lancement suivant (socle)
    socle::crash::install("Tube", env!("CARGO_PKG_VERSION"));
    if store::test_mode() {
        socle::testmode::low_priority();
    }
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--test-hors-ecran").collect();
    let open_arg = (!args.is_empty()).then(|| args.join(" "));
    // instance unique (socle : mutex + tube nommé) : une 2ᵉ instance transmet son lien ou sa
    // recherche à la 1ʳᵉ. En test, un nom à part : on ne touche jamais l'instance de l'utilisateur.
    let name = if store::test_mode() { format!("TubeTest{}", std::process::id()) } else { "Tube".to_string() };
    let inst = match socle::instance::start(&name, &args) {
        socle::instance::Start::Forwarded => return,
        socle::instance::Start::Primary(i) => Some(i),
        socle::instance::Start::Alone => None,
    };
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("boucle d'événements");
    let proxy = event_loop.create_proxy();
    if let Some(i) = inst {
        let p3 = proxy.clone();
        i.listen(move |args| {
            let _ = p3.send_event(UserEvent::Open(args));
        });
    }
    let ctx = egui::Context::default();
    let p2 = proxy.clone();
    ctx.set_request_repaint_callback(move |info| {
        let _ = p2.send_event(UserEvent::RepaintAt(Instant::now() + info.delay));
    });
    let mut runner = Runner {
        ctx,
        proxy,
        gfx: None,
        app: None,
        next_repaint: None,
        next_tick: None,
        last_frame: Instant::now(),
        normal_rect: None,
        moved_at: None,
        open_arg,
        script_done: Vec::new(),
        paints: (0, 0.0),
        paints_at: Instant::now(),
    };
    let _ = event_loop.run_app(&mut runner);
}
