//! Sous-fenêtre vidéo : Media Foundation y affiche la vidéo directement avec la
//! carte graphique (aucun passage par la mémoire CPU) — fluide même en 5K 60 i/s.
//! Elle est placée sous l'interface ; la souris la traverse (HTTRANSPARENT) et les
//! éléments d'interface qui la recouvrent (barre du haut, toasts, menus) sont
//! découpés de sa région pour rester visibles.

use std::sync::Once;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

unsafe extern "system" fn proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    if m == WM_ERASEBKGND {
        return LRESULT(1);
    }
    // Filet de sécurité : si un clic arrive quand même ici, on le renvoie à la fenêtre principale
    // (coordonnées converties) pour que l'interface le voie (clic = pause, double-clic = plein écran).
    if (WM_MOUSEMOVE..=WM_MBUTTONDBLCLK).contains(&m) {
        unsafe {
            if let Ok(parent) = GetParent(h) {
                let x = (lp.0 & 0xFFFF) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                let mut pt = [windows::Win32::Foundation::POINT { x, y }];
                MapWindowPoints(Some(h), Some(parent), &mut pt);
                let lp2 = ((pt[0].y as u16 as isize) << 16) | (pt[0].x as u16 as isize);
                return SendMessageW(parent, m, Some(wp), Some(LPARAM(lp2)));
            }
        }
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}

static CLASS: Once = Once::new();

pub struct VideoWindow {
    pub hwnd: HWND,
    placed: Option<(i32, i32, i32, i32)>,
    cuts: Vec<(i32, i32, i32, i32)>,
    visible: bool,
}

impl VideoWindow {
    pub fn new(parent: isize) -> Option<VideoWindow> {
        unsafe {
            let inst = GetModuleHandleW(None).ok()?;
            CLASS.call_once(|| {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(proc),
                    hInstance: inst.into(),
                    lpszClassName: w!("LecteurVideo"),
                    hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                    ..Default::default()
                };
                RegisterClassW(&wc);
            });
            let parent = HWND(parent as *mut _);
            // Le parent ne doit pas repeindre par-dessus la vidéo.
            let st = GetWindowLongPtrW(parent, GWL_STYLE);
            SetWindowLongPtrW(parent, GWL_STYLE, st | WS_CLIPCHILDREN.0 as isize);
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE,
                w!("LecteurVideo"),
                None,
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                2,
                2,
                Some(parent),
                None,
                Some(inst.into()),
                None,
            )
            .ok()?;
            Some(VideoWindow { hwnd, placed: None, cuts: Vec::new(), visible: false })
        }
    }

    /// Place la vidéo (pixels, coordonnées client du parent) ; `cuts` = zones d'interface à laisser visibles.
    /// Renvoie vrai si la taille a changé (le moteur doit alors ajuster sa sortie).
    pub fn place(&mut self, r: (i32, i32, i32, i32), cuts: &[(i32, i32, i32, i32)]) -> bool {
        let resized = self.placed.map(|p| (p.2, p.3)) != Some((r.2, r.3));
        unsafe {
            if self.placed != Some(r) || !self.visible {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), r.0, r.1, r.2.max(2), r.3.max(2), SWP_NOACTIVATE | SWP_SHOWWINDOW);
                self.placed = Some(r);
                self.visible = true;
            }
            let local: Vec<(i32, i32, i32, i32)> = cuts
                .iter()
                .map(|c| (c.0 - r.0, c.1 - r.1, c.2, c.3))
                .filter(|c| c.0 < r.2 && c.1 < r.3 && c.0 + c.2 > 0 && c.1 + c.3 > 0)
                .collect();
            if local != self.cuts {
                if local.is_empty() {
                    SetWindowRgn(self.hwnd, None, true);
                } else {
                    let rgn = CreateRectRgn(0, 0, r.2, r.3);
                    for c in &local {
                        let o = CreateRectRgn(c.0, c.1, c.0 + c.2, c.1 + c.3);
                        CombineRgn(Some(rgn), Some(rgn), Some(o), RGN_DIFF);
                        let _ = DeleteObject(o.into());
                    }
                    SetWindowRgn(self.hwnd, Some(rgn), true);
                }
                self.cuts = local;
            }
        }
        resized
    }

    /// Image actuellement affichée par la vidéo (capture de la fenêtre principale recadrée),
    /// réduite à 1920 px de large au plus. Sert à afficher la pause en grisé.
    pub fn snapshot(&self, max_w: i32) -> Option<egui::ColorImage> {
        if !self.visible {
            return None;
        }
        unsafe {
            let root = GetAncestor(self.hwnd, GA_ROOT);
            let (mut rr, mut vr) = (RECT::default(), RECT::default());
            GetWindowRect(root, &mut rr).ok()?;
            GetWindowRect(self.hwnd, &mut vr).ok()?;
            let (w, h) = (rr.right - rr.left, rr.bottom - rr.top);
            if w <= 0 || h <= 0 {
                return None;
            }
            let sdc = GetDC(None);
            let mdc = CreateCompatibleDC(Some(sdc));
            let bmp = CreateCompatibleBitmap(sdc, w, h);
            let old = SelectObject(mdc, bmp.into());
            let ok = windows::Win32::Storage::Xps::PrintWindow(root, mdc, windows::Win32::Storage::Xps::PRINT_WINDOW_FLAGS(2)).as_bool();
            let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
            let mut px = vec![0u8; (w * h * 4) as usize];
            GetDIBits(mdc, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
            SelectObject(mdc, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mdc);
            ReleaseDC(None, sdc);
            if !ok {
                return None;
            }
            let (cx, cy) = ((vr.left - rr.left).max(0), (vr.top - rr.top).max(0));
            let cw = (vr.right - vr.left).min(w - cx);
            let ch = (vr.bottom - vr.top).min(h - cy);
            if cw <= 4 || ch <= 4 {
                return None;
            }
            let mut out = Vec::with_capacity((cw * ch * 4) as usize);
            for y in cy..cy + ch {
                let row = &px[((y * w + cx) * 4) as usize..((y * w + cx + cw) * 4) as usize];
                for c in row.chunks_exact(4) {
                    out.extend_from_slice(&[c[2], c[1], c[0], 255]);
                }
            }
            let mut img = image::RgbaImage::from_raw(cw as u32, ch as u32, out)?;
            if cw > max_w {
                img = image::imageops::resize(&img, max_w as u32, (max_w * ch / cw) as u32, image::imageops::FilterType::Triangle);
            }
            Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img))
        }
    }

    /// Mode cinéma : couleurs moyennes, par bande, des deux bords de l'image affichée.
    /// `vertical` : bords gauche/droit (bandes empilées) ; sinon haut/bas (bandes côte à côte).
    /// Petite capture de l'écran composé (quelques colonnes de pixels), très peu coûteuse.
    pub fn edge_colors(&self, vertical: bool, bands: usize) -> Option<(Vec<[f32; 3]>, Vec<[f32; 3]>)> {
        if !self.visible || bands == 0 {
            return None;
        }
        unsafe {
            let mut r = RECT::default();
            GetWindowRect(self.hwnd, &mut r).ok()?;
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w < 32 || h < 32 {
                return None;
            }
            let depth = 10;
            let (sw, sh) = if vertical { (depth, h) } else { (w, depth) };
            let sdc = GetDC(None);
            let mdc = CreateCompatibleDC(Some(sdc));
            let bmp = CreateCompatibleBitmap(sdc, sw, sh);
            let mut out = (Vec::new(), Vec::new());
            for side in 0..2 {
                let old = SelectObject(mdc, bmp.into());
                let (x, y) = match (vertical, side) {
                    (true, 0) => (r.left + 2, r.top),
                    (true, _) => (r.right - depth - 2, r.top),
                    (false, 0) => (r.left, r.top + 2),
                    (false, _) => (r.left, r.bottom - depth - 2),
                };
                let _ = BitBlt(mdc, 0, 0, sw, sh, Some(sdc), x, y, SRCCOPY);
                let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: sw, biHeight: -sh, biPlanes: 1, biBitCount: 32, ..Default::default() }, ..Default::default() };
                let mut px = vec![0u8; (sw * sh * 4) as usize];
                GetDIBits(mdc, bmp, 0, sh as u32, Some(px.as_mut_ptr() as *mut _), &mut bi, DIB_RGB_COLORS);
                SelectObject(mdc, old);
                let len = if vertical { sh } else { sw } as usize;
                let mut v = Vec::with_capacity(bands);
                for b in 0..bands {
                    let (a, z) = (b * len / bands, ((b + 1) * len / bands).max(b * len / bands + 1));
                    let mut acc = [0f32; 3];
                    let mut n = 0f32;
                    for i in (a..z).step_by(2) {
                        for d in (0..depth as usize).step_by(3) {
                            let (xx, yy) = if vertical { (d, i) } else { (i, d) };
                            let o = (yy * sw as usize + xx) * 4;
                            acc[0] += px[o + 2] as f32;
                            acc[1] += px[o + 1] as f32;
                            acc[2] += px[o] as f32;
                            n += 1.0;
                        }
                    }
                    v.push([acc[0] / n.max(1.0), acc[1] / n.max(1.0), acc[2] / n.max(1.0)]);
                }
                if side == 0 { out.0 = v } else { out.1 = v }
            }
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mdc);
            ReleaseDC(None, sdc);
            Some(out)
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.placed.map(|p| (p.2, p.3)).unwrap_or((0, 0))
    }

    pub fn client_rect(&self) -> RECT {
        let (w, h) = self.size();
        RECT { left: 0, top: 0, right: w, bottom: h }
    }
}

impl Drop for VideoWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
