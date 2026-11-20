//! Interface (mêmes composants que l'Explorateur et Vigie : barre d'outils, volet latéral,
//! barre d'état, cartes arrondies, thèmes crème / bleu nuit).
use crate::api::{Item, Kind};
use crate::app::{App, Login, Mode, Setup, View, WinOp, fmt_time};
use socle::icons::{self, Icon as Ic};
use crate::store::Quality;
use egui::{Align, Align2, Color32, CornerRadius, CursorIcon, FontId, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RED: Color32 = Color32::from_rgb(0xE5, 0x3B, 0x3B);
const SB: Color32 = Color32::from_rgb(0x00, 0xD4, 0x00);

#[derive(Clone, Copy, PartialEq)]
pub enum Menu {
    /// menu principal de la roue ⚙
    Gear,
    Quality,
    Speed,
    Audio,
    Subs,
    Sleep,
}

/// Ligne de menu : libellé à gauche, valeur (et chevron) à droite.
fn menu_row(ui: &mut Ui, p: &socle::theme::Palette, label: &str, value: &str, chevron: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover_bg);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), p.text);
    let vx = r.right() - if chevron { 24.0 } else { 10.0 };
    ui.painter().text(pos2(vx, r.center().y), Align2::RIGHT_CENTER, value, FontId::proportional(12.5), sub_text(p));
    if chevron {
        ui.painter().text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, "›", FontId::proportional(16.0), p.muted);
    }
    resp
}

/// Ligne à bascule (case à droite).
fn menu_toggle(ui: &mut Ui, p: &socle::theme::Palette, label: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover_bg);
    }
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), p.text);
    let sw = Rect::from_center_size(pos2(r.right() - 26.0, r.center().y), vec2(30.0, 16.0));
    ui.painter().rect_filled(sw, CornerRadius::same(8), if on { p.accent } else { p.border });
    let kx = if on { sw.right() - 8.0 } else { sw.left() + 8.0 };
    ui.painter().circle_filled(pos2(kx, sw.center().y), 6.0, Color32::WHITE);
    resp
}

/// Masque les coins d'une image par 4 petits éventails de la couleur du fond (coins arrondis
/// sans passer l'image entière par le chemin lent du rendu logiciel).
fn round_corners(pt: &egui::Painter, r: Rect, rad: f32, bg: Color32) {
    let mut mesh = egui::Mesh::default();
    for (c, a0) in [(r.left_top(), 180.0f32), (r.right_top(), 270.0), (r.right_bottom(), 0.0), (r.left_bottom(), 90.0)] {
        let center = c + vec2(if c.x == r.left() { rad } else { -rad }, if c.y == r.top() { rad } else { -rad });
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(c, bg);
        let n = 6;
        for i in 0..=n {
            let a = (a0 + 90.0 * i as f32 / n as f32).to_radians();
            mesh.colored_vertex(center + vec2(a.cos(), a.sin()) * rad, bg);
        }
        for i in 0..n {
            mesh.add_triangle(base, base + 1 + i, base + 2 + i);
        }
    }
    pt.add(mesh);
}

fn galley(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32, rows: usize) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max_w.max(8.0));
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(8.0), max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

fn section(ui: &mut Ui, p: &socle::theme::Palette, t: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(t.to_uppercase()).size(11.0).color(p.muted).strong());
    ui.add_space(2.0);
}

/// Bouton-icône dessiné à la main (contrôles du lecteur).
/// Contenu d'un bouton : icône vectorielle ou court texte (qualité, vitesse).
pub enum Gl<'a> {
    I(Ic),
    T(&'a str),
}

fn icon_btn(ui: &mut Ui, rect: Rect, id: &str, g: Gl, tip: &str, fg: Color32, hover: Color32) -> egui::Response {
    let r = ui.interact(rect, Id::new(id), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect.shrink(2.0), CornerRadius::same(7), hover);
    }
    match g {
        Gl::I(ic) => icons::paint(ui.painter(), rect, ic, fg),
        Gl::T(t) => {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, t, FontId::proportional((rect.height() * 0.36).min(14.0)), fg);
        }
    }
    if tip.is_empty() { r } else { r.on_hover_text(tip) }
}

/// Bouton d'icône de la barre d'outils (s'insère dans la mise en page).
fn tool_btn(ui: &mut Ui, id: &str, ic: Ic, tip: &str, enabled: bool, fg: Color32, hover: Color32) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(34.0, 32.0), Sense::hover());
    if !enabled {
        icons::paint(ui.painter(), rect.shrink(4.0), ic, fg.gamma_multiply(0.35));
        return ui.interact(rect, Id::new(id), Sense::hover());
    }
    let r = ui.interact(rect, Id::new(id), Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(7), hover);
    }
    icons::paint(ui.painter(), rect.shrink(4.0), ic, fg);
    r.on_hover_text(tip)
}

/// Texte secondaire des vignettes (chaîne, vues, date) : plus contrasté que `muted`.
fn sub_text(p: &socle::theme::Palette) -> Color32 {
    p.text.lerp_to_gamma(p.muted, 0.4)
}

/// Agrandit un peu le texte des grandes vignettes.
fn card_scale(w: f32) -> f32 {
    (w / 300.0).clamp(1.0, 1.3)
}

fn to_px(r: Rect, ppp: f32) -> (i32, i32, i32, i32) {
    ((r.left() * ppp).round() as i32, (r.top() * ppp).round() as i32, (r.width() * ppp).round() as i32, (r.height() * ppp).round() as i32)
}

impl App {
    pub fn draw(&mut self, ui: &mut Ui) {
        self.draw_main(ui);
        // palette de commandes Ctrl+K (socle), par-dessus tout
        let ctx = ui.ctx().clone();
        self.palette.handle_shortcut(&ctx);
        if self.palette.is_open() {
            // elle passe sur la vidéo : la fenêtre vidéo est entièrement découpée le temps de la palette
            self.video_cuts.push(to_px(ctx.content_rect(), ctx.pixels_per_point()));
            let cmds = self.commands();
            if let Some(id) = self.palette.show(&ctx, &self.pal.clone(), &cmds) {
                self.run_command(&id);
            }
        }
    }

    /// Commandes de la palette Ctrl+K : tout ce que font les menus et les raccourcis.
    fn commands(&self) -> Vec<socle::ui::Command> {
        use socle::ui::Command as C;
        let v = self.watch.is_some();
        let mut c = vec![
            C::new("home", "Accueil").group("Aller à").keys("Ctrl+1").icon(Ic::Home),
            C::new("subs", "Abonnements").group("Aller à").keys("Ctrl+2").icon(Ic::Subs),
            C::new("later", "À regarder plus tard").group("Aller à").keys("Ctrl+3").icon(Ic::Clock),
            C::new("playlists", "Playlists").group("Aller à").keys("Ctrl+4").icon(Ic::Playlist),
            C::new("history", "Historique").group("Aller à").keys("Ctrl+5").icon(Ic::History),
            C::new("search", "Rechercher sur YouTube").group("Aller à").keys("Ctrl+F").icon(Ic::Search),
            C::new("settings", "Réglages").group("Aller à").icon(Ic::Settings),
            C::new("tab.new", "Nouvel onglet").group("Onglets").keys("Ctrl+T").icon(Ic::Plus),
            C::new("tab.close", "Fermer l'onglet").group("Onglets").keys("Ctrl+W").icon(Ic::Close),
            C::new("tab.reopen", "Rouvrir l'onglet fermé").group("Onglets").keys("Ctrl+Maj+T").enabled(!self.closed_tabs.is_empty()),
            C::new("refresh", "Actualiser la page").group("Page").keys("F5").icon(Ic::Refresh),
            C::new("theme", if self.s.dark { "Thème clair" } else { "Thème sombre" }).group("Affichage").icon(if self.s.dark { Ic::Sun } else { Ic::Moon }),
            C::new("hide_shorts", if self.s.hide_shorts { "Afficher les Shorts" } else { "Masquer les Shorts" }).group("Affichage"),
            C::new("hide_watched", if self.s.hide_watched { "Afficher les vidéos déjà vues" } else { "Masquer les vidéos déjà vues" }).group("Affichage"),
            C::new("help", "Raccourcis clavier").group("Aide").keys("F1").icon(Ic::Help),
        ];
        let pv = [
            C::new("play", "Lecture / pause").group("Lecture").keys("Espace").icon(Ic::Play),
            C::new("full", "Plein écran").group("Lecture").keys("F").icon(Ic::Fullscreen),
            C::new("half", "Demi-écran").group("Lecture").keys("H").icon(Ic::Half),
            C::new("mini", "Mini-lecteur").group("Lecture").keys("I").icon(Ic::Mini),
            C::new("cinema", "Mode cinéma").group("Lecture").keys("C"),
            C::new("subs.toggle", "Sous-titres oui / non").group("Lecture").keys("V").icon(Ic::Subs),
            C::new("audio", "Audio seul").group("Lecture").icon(Ic::Audio),
            C::new("loop", "Lecture en boucle").group("Lecture").icon(Ic::Loop),
            C::new("shot", "Capture d'image").group("Lecture").keys("S").icon(Ic::Camera),
            C::new("copy", "Copier le lien à l'instant").group("Lecture").keys("Ctrl+C").icon(Ic::Link),
            C::new("next", "Vidéo suivante").group("Lecture").keys("Maj+N").icon(Ic::Next),
            C::new("chap.next", "Chapitre suivant").group("Lecture").keys("Ctrl+→"),
            C::new("chap.prev", "Chapitre précédent").group("Lecture").keys("Ctrl+←"),
            C::new("q.auto", "Qualité automatique").group("Qualité"),
            C::new("q.2160", "Qualité 2160p (4K)").group("Qualité"),
            C::new("q.1440", "Qualité 1440p").group("Qualité"),
            C::new("q.1080", "Qualité 1080p").group("Qualité"),
            C::new("q.720", "Qualité 720p").group("Qualité"),
            C::new("sp.1", "Vitesse normale").group("Vitesse"),
            C::new("sp.1.25", "Vitesse ×1,25").group("Vitesse"),
            C::new("sp.1.5", "Vitesse ×1,5").group("Vitesse"),
            C::new("sp.2", "Vitesse ×2").group("Vitesse"),
            C::new("sleep.30", "Minuterie : pause dans 30 min").group("Minuterie"),
            C::new("sleep.60", "Minuterie : pause dans 1 h").group("Minuterie"),
            C::new("sleep.end", "Minuterie : pause à la fin de la vidéo").group("Minuterie"),
            C::new("sleep.off", "Minuterie : désactiver").group("Minuterie").enabled(self.sleep.is_some()),
        ];
        c.extend(pv.into_iter().map(|x| x.enabled(v)));
        c
    }

    fn run_command(&mut self, id: &str) {
        use crate::app::Sleep;
        let mins = |m: u64| Some(Sleep::At(Instant::now() + Duration::from_secs(m * 60)));
        match id {
            "home" => self.go(View::Home),
            "subs" => self.go(View::Subs),
            "later" => self.go(View::Later),
            "playlists" => self.go(View::Playlists),
            "history" => self.go(View::History),
            "search" => self.focus_search = true,
            "settings" => self.go(View::Settings),
            "tab.new" => self.new_tab(View::Home, None, true),
            "tab.close" => self.close_tab(self.tab),
            "tab.reopen" => self.reopen_tab(),
            "refresh" => self.refresh(),
            "theme" => {
                let d = !self.s.dark;
                self.set_dark(d);
            }
            "hide_shorts" => {
                self.s.hide_shorts = !self.s.hide_shorts;
                self.refresh();
            }
            "hide_watched" => {
                self.s.hide_watched = !self.s.hide_watched;
                self.refresh();
            }
            "help" => self.help = true,
            "play" => self.toggle(),
            "full" => self.toggle_full(),
            "half" => self.toggle_half(),
            "mini" => self.toggle_mini(),
            "cinema" => self.s.cinema = !self.s.cinema,
            "subs.toggle" => self.toggle_subs(),
            "audio" => {
                let on = !self.watch.as_ref().is_some_and(|w| w.audio_only);
                self.set_audio_only(on);
            }
            "loop" => self.loop_on = !self.loop_on,
            "shot" => self.screenshot(),
            "copy" => self.copy_time_link(),
            "next" => {
                self.next_in_queue();
            }
            "chap.next" => self.chapter_jump(1),
            "chap.prev" => self.chapter_jump(-1),
            "q.auto" => self.set_quality(None),
            "q.2160" => self.set_quality(Some(2160)),
            "q.1440" => self.set_quality(Some(1440)),
            "q.1080" => self.set_quality(Some(1080)),
            "q.720" => self.set_quality(Some(720)),
            "sp.1" => self.set_speed(1.0),
            "sp.1.25" => self.set_speed(1.25),
            "sp.1.5" => self.set_speed(1.5),
            "sp.2" => self.set_speed(2.0),
            "sleep.30" => self.set_sleep(mins(30)),
            "sleep.60" => self.set_sleep(mins(60)),
            "sleep.end" => self.set_sleep(Some(Sleep::EndOfVideo)),
            "sleep.off" => self.set_sleep(None),
            _ => {}
        }
    }

    fn draw_main(&mut self, ui: &mut Ui) {
        self.thumbs.pump(ui.ctx());
        // aperçu au survol : arrêté dès que sa vignette n'est plus survolée
        if let Some(pv) = self.preview.as_mut() {
            if pv.touched {
                pv.touched = false;
            } else {
                self.preview = None;
            }
        }
        self.video_px = None;
        self.video_cuts.clear();
        let p = self.pal;
        if self.mode != Mode::Normal {
            egui::CentralPanel::default().frame(Frame::new().fill(Color32::BLACK)).show(ui, |ui| self.draw_overlay_mode(ui));
            self.draw_toast(ui);
            return;
        }
        if let Setup::Running(_) | Setup::Failed(_) = self.setup {
            egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| self.draw_setup(ui));
            return;
        }
        self.draw_toolbar(ui);
        self.draw_tabs(ui);
        self.draw_reconnect(ui);
        self.draw_status(ui);
        if matches!(self.login, Login::Need | Login::Importing) && self.view != View::Settings {
            egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| self.draw_login(ui));
            return;
        }
        if self.view != View::Watch || ui.available_width() < 900.0 {
            self.draw_sidebar(ui);
        }
        egui::CentralPanel::default().frame(Frame::new().fill(p.surface)).show(ui, |ui| match self.view.clone() {
            View::Watch => self.draw_watch(ui),
            View::Settings => self.draw_settings(ui),
            v => self.draw_feed(ui, &v),
        });
        self.draw_menu(ui);
        self.draw_toast(ui);
        self.draw_help(ui);
    }

    // ---------- Installation / connexion ----------
    fn draw_setup(&mut self, ui: &mut Ui) {
        let p = self.pal;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.3);
            ui.label(RichText::new("Tube").size(30.0).strong());
            ui.add_space(12.0);
            match &self.setup {
                Setup::Running(s) => {
                    ui.add(egui::Spinner::new().size(22.0));
                    ui.add_space(8.0);
                    ui.label(RichText::new(s).color(p.muted));
                    ui.label(RichText::new("Première installation : environ une minute (yt-dlp + deno).").size(12.0).color(p.muted));
                }
                Setup::Failed(e) => {
                    ui.label(RichText::new("Installation impossible").color(p.del).strong());
                    ui.label(RichText::new(e).color(p.muted));
                }
                Setup::Done => {}
            }
        });
    }

    fn draw_login(&mut self, ui: &mut Ui) {
        let p = self.pal;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.18);
            Frame::new().fill(p.side).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(22)).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.label(RichText::new("Connecter ton compte YouTube").size(20.0).strong());
                ui.add_space(6.0);
                ui.label(RichText::new("Tube réutilise la session YouTube de ton navigateur pour afficher tes recommandations, abonnements et playlists, et remonter ton historique. Aucun mot de passe n'est demandé ni lu.").color(p.muted));
                ui.add_space(6.0);
                ui.label(RichText::new("Si le navigateur est ouvert, son service réseau redémarre (coupure d'environ 1 s, les téléchargements en cours dans le navigateur peuvent être interrompus).").size(12.0).color(p.muted));
                ui.add_space(12.0);
                if self.login == Login::Importing {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label("Import des cookies…");
                    });
                } else {
                    let profiles = crate::cookies::browser_profiles();
                    if profiles.is_empty() {
                        ui.label(RichText::new("Aucun navigateur Chromium trouvé (Opera GX, Opera, Chrome, Edge).").color(p.del));
                    }
                    for (name, path) in profiles {
                        if ui.add(egui::Button::new(RichText::new(format!("Se connecter avec {name}")).strong().color(p.on_accent)).fill(p.accent).min_size(vec2(260.0, 34.0))).clicked() {
                            self.import_cookies(path, true);
                        }
                        ui.add_space(4.0);
                    }
                }
                if let Some(m) = &self.login_msg {
                    ui.add_space(8.0);
                    ui.label(RichText::new(m).color(p.del));
                }
                ui.add_space(10.0);
                if ui.add(egui::Button::new(RichText::new("Continuer sans compte").color(p.muted)).frame(false)).clicked() {
                    self.login = Login::Skipped;
                    self.load_feed(&View::Home, false);
                }
            });
        });
    }

    // ---------- Barre d'outils ----------
    fn draw_toolbar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("toolbar").frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(12, 8))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let can_back = !self.back.is_empty();
                if tool_btn(ui, "t_back", Ic::Back, "Retour (Alt+← ou bouton arrière de la souris)", can_back, p.text, p.hover_bg).clicked() {
                    self.go_back();
                }
                if tool_btn(ui, "t_refresh", Ic::Refresh, "Actualiser (F5)", true, p.text, p.hover_bg).clicked() {
                    self.refresh();
                }
                ui.add_space(6.0);
                // logo : pastille rouge arrondie + triangle blanc
                let logo = ui
                    .scope(|ui| {
                        let (lr, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                        let ctx = ui.ctx().clone();
                        let tex = self
                            .logo
                            .get_or_insert_with(|| {
                                let img = image::load_from_memory(include_bytes!("../assets/icon64.png")).map(|i| i.to_rgba8()).unwrap_or_default();
                                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img);
                                ctx.load_texture("logo", ci, egui::TextureOptions::LINEAR)
                            })
                            .id();
                        ui.painter().image(tex, lr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        ui.label(RichText::new("Tube").size(17.0).strong().color(p.text));
                    })
                    .response;
                let logo = ui.interact(logo.rect, Id::new("logo"), Sense::click());
                if logo.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if logo.on_hover_text("Accueil").clicked() {
                    if self.view == View::Home {
                        self.scroll_to_top = true;
                    }
                    self.go(View::Home);
                }
                ui.add_space(14.0);
                // recherche avec autocomplétion floue (socle) : historique local d'abord, puis les
                // suggestions YouTube (arrivées en ~200 ms) ; Tab complète, ↑↓ choisit, Entrée lance
                let sid = Id::new("search");
                if self.focus_search {
                    ui.memory_mut(|m| m.request_focus(sid));
                    self.focus_search = false;
                }
                let w = (ui.available_width() - 260.0).clamp(240.0, 600.0);
                let ac = std::mem::take(&mut self.ac);
                let r = socle::ui::autocomplete_field(ui, &p, sid, &mut self.search_state, &ac, &mut self.search, "Rechercher sur YouTube, ou coller un lien (Ctrl+F)", w);
                self.ac = ac;
                if r.response.changed() {
                    self.suggest();
                    self.local_first();
                }
                if r.response.has_focus() && !self.search_state.suggestions.is_empty() {
                    // la liste peut passer sur la vidéo : on la découpe de la fenêtre vidéo
                    let lr = Rect::from_min_size(r.response.rect.left_bottom() + vec2(-40.0, 0.0), vec2(w + 60.0, self.search_state.suggestions.len() as f32 * 28.0 + 40.0));
                    self.video_cuts.push(to_px(lr, ui.ctx().pixels_per_point()));
                }
                if let Some(q) = r.submitted.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()) {
                    // Y4 : « ▶ titre » = une vidéo déjà vue → lecture directe
                    let local = q.strip_prefix("▶ ").and_then(|t| self.local_videos().into_iter().find(|x| x.1.title == t).map(|x| x.1));
                    if let Some(it) = local {
                        self.search.clear();
                        self.open(&it, None);
                    } else if let Some(id) = video_id_from(&q) {
                        let it = Item { kind: Kind::Video, id, ..Default::default() };
                        self.open(&it, None);
                    } else {
                        self.ac.history.record(&q);
                        self.go(View::Search(q));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if tool_btn(ui, "t_set", Ic::Settings, "Réglages", true, p.text, p.hover_bg).clicked() {
                        self.go(View::Settings);
                    }
                    let theme_icon = if self.s.dark { Ic::Sun } else { Ic::Moon };
                    if tool_btn(ui, "t_theme", theme_icon, "Thème clair / sombre", true, p.text, p.hover_bg).clicked() {
                        let d = !self.s.dark;
                        self.set_dark(d);
                    }
                    if tool_btn(ui, "t_help", Ic::Help, "Raccourcis (F1)", true, p.text, p.hover_bg).clicked() {
                        self.help = true;
                    }
                    let (txt, c) = match self.login {
                        Login::Ok => ("● connecté", p.add),
                        Login::Skipped => ("○ sans compte", p.muted),
                        _ => ("○ non connecté", p.live),
                    };
                    ui.label(RichText::new(txt).size(12.0).color(c));
                });
            });
        });
    }

    /// Bandeau discret quand la session YouTube a expiré et que les cookies du navigateur sont
    /// verrouillés : l'import (qui coupe ~1 s le réseau du navigateur) ne se fait que sur clic.
    fn draw_reconnect(&mut self, ui: &mut Ui) {
        if !self.reconnect {
            return;
        }
        let p = self.pal;
        let mut go = false;
        let mut later = false;
        egui::Panel::top("reconnect").frame(Frame::new().fill(p.live_bg).inner_margin(Margin::symmetric(14, 6))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Session YouTube expirée : accueil et abonnements sont anonymes.").color(p.text));
                if ui.button("Reconnecter").on_hover_text("Relit les cookies d'Opera : son réseau est coupé environ 1 s (un téléchargement en cours peut échouer)").clicked() {
                    go = true;
                }
                ui.label(RichText::new("coupe le réseau d'Opera ~1 s").size(11.5).color(p.muted));
                if ui.button("Réappliquer le réglage d'Opera").on_hover_text("Après une mise à jour d'Opera : remet le drapeau qui laisse Tube relire ses cookies sans le couper (effet au prochain démarrage d'Opera)").clicked() {
                    self.opera_flag();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Plus tard").clicked() {
                        later = true;
                    }
                });
            });
        });
        if go {
            if let Some((_, path)) = crate::cookies::browser_profiles().into_iter().next() {
                self.import_cookies(path, true);
            }
            self.reconnect = false;
        }
        if later {
            self.reconnect = false;
        }
    }

    // ---------- Onglets ----------
    fn draw_tabs(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::top("tabs").frame(Frame::new().fill(p.chrome).inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 6 })).show(ui, |ui| {
            let n = self.tabs.len();
            let full = ui.available_width() - 40.0;
            let tw = (full / n as f32 - 4.0).clamp(90.0, 230.0);
            let mut switch = None;
            let mut close = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for i in 0..n {
                    let (rect, r) = ui.allocate_exact_size(vec2(tw, 30.0), Sense::click());
                    let on = i == self.tab;
                    let bg = if on { p.sel_bg } else if r.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                    let mut x = rect.left() + 10.0;
                    if i == self.play_tab && self.watch.is_some() && self.tabs[i].video.as_ref().zip(self.watch.as_ref()).is_some_and(|(v, w)| v.id == w.item.id) {
                        let playing = self.watch.as_ref().and_then(|w| w.player.as_ref()).is_some_and(|p| p.wants_play());
                        icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(14.0, 14.0)), if playing { Ic::VolumeLevel(2) } else { Ic::Pause }, p.accent);
                        x += 18.0;
                    }
                    let title = self.tab_title(i);
                    let g = galley(ui, &title, FontId::proportional(12.5), if on { p.text } else { sub_text(&p) }, rect.right() - x - 28.0, 1);
                    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, p.text);
                    let cr = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(20.0, 20.0));
                    if on || r.hovered() || ui.rect_contains_pointer(cr) {
                        let c = ui.interact(cr, Id::new(("tab_x", i)), Sense::click());
                        if c.hovered() {
                            ui.painter().rect_filled(cr, CornerRadius::same(5), p.border);
                        }
                        icons::paint(ui.painter(), cr.shrink(4.0), Ic::Close, p.muted);
                        if c.on_hover_text("Fermer l'onglet (Ctrl+W, clic molette)").clicked() {
                            close = Some(i);
                            continue;
                        }
                    }
                    if r.clicked() {
                        switch = Some(i);
                    }
                    if r.middle_clicked() {
                        close = Some(i);
                    }
                    r.on_hover_text(&title);
                }
                if tool_btn(ui, "tab_new", Ic::Plus, "Nouvel onglet (Ctrl+T) — clic molette ou clic droit sur une vignette pour l'y ouvrir", true, p.text, p.hover_bg).clicked() {
                    self.new_tab(View::Home, None, true);
                }
            });
            if let Some(i) = close {
                self.close_tab(i);
            } else if let Some(i) = switch {
                self.switch_tab(i);
            }
        });
    }

    /// Menu du clic droit sur une vignette (grilles et listes latérales).
    fn item_menu(&mut self, ui: &mut Ui, it: &Item, list: Option<(&[Item], usize)>) {
        let video = it.kind == Kind::Video;
        let q = || list.map(|(l, i)| (l.to_vec(), i));
        if ui.button("Lire").clicked() {
            self.open(it, q());
            ui.close();
        }
        if ui.button("Ouvrir dans un nouvel onglet").clicked() {
            self.open_in_tab(it);
            ui.close();
        }
        if video {
            if ui.button("Lire dans le mini-lecteur").clicked() {
                self.open(it, q());
                self.ops.push(WinOp::Mini(true));
                ui.close();
            }
            if self.watch.is_some() {
                ui.separator();
                if ui.button("Lire ensuite").clicked() {
                    self.enqueue(it, true);
                    ui.close();
                }
                if ui.button("Ajouter à la file d'attente").clicked() {
                    self.enqueue(it, false);
                    ui.close();
                }
            }
            if self.login == Login::Ok && ui.button("Enregistrer dans « À regarder plus tard »").clicked() {
                self.add_watch_later(it);
                ui.close();
            }
        }
        if !it.channel_id.is_empty() && ui.button(format!("Aller à la chaîne « {} »", it.channel)).clicked() {
            self.open_channel(&it.channel_id, &it.channel, false);
            ui.close();
        }
        if !it.not_interested.is_empty() || !it.no_channel.is_empty() {
            ui.separator();
            if !it.not_interested.is_empty() && ui.button("Pas intéressé").clicked() {
                self.not_interested(it, false);
                ui.close();
            }
            if !it.no_channel.is_empty() && ui.button("Ne pas recommander la chaîne").clicked() {
                self.not_interested(it, true);
                ui.close();
            }
        }
        ui.separator();
        let url = if video { format!("https://www.youtube.com/watch?v={}", it.id) } else { format!("https://www.youtube.com/playlist?list={}", it.id) };
        if ui.button("Copier le lien").clicked() {
            ui.ctx().copy_text(url.clone());
            ui.close();
        }
        if ui.button("Ouvrir dans le navigateur").clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
            ui.close();
        }
        if self.s.positions.contains_key(&it.id) && ui.button("Oublier la reprise").clicked() {
            self.s.positions.remove(&it.id);
            ui.close();
        }
    }

    /// Pause ou attente réseau : la vidéo est remplacée par sa dernière image, grisée, avec un
    /// symbole au centre — on voit tout de suite si c'est en pause ou en train de charger.
    fn draw_still(&mut self, ui: &mut Ui, area: Rect) {
        let state = self.watch.as_ref().and_then(|w| {
            let pl = w.player.as_ref()?;
            if !pl.ready || w.error.is_some() {
                return None;
            }
            if !pl.wants_play() {
                Some((w.item.id.clone(), false))
            } else if w.stalled {
                Some((w.item.id.clone(), true))
            } else {
                None
            }
        });
        let Some((id, waiting)) = state else {
            self.still = None;
            return;
        };
        // autre vidéo, ou zone redimensionnée (plein écran, mini-lecteur…) : on revoit la vraie vidéo puis on recapture
        if self.still.as_ref().is_some_and(|s| s.0 != id || (s.2 - area.size()).length() > 2.0 || s.3 == waiting) {
            self.unfreeze();
        }
        if self.still.is_none() {
            let wait = self.still_after.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                ui.ctx().request_repaint_after(wait);
                return;
            }
            match self.vidwin.as_ref().and_then(|v| v.snapshot(1920)) {
                Some(mut img) => {
                    if !waiting {
                        // pause : image en noir et blanc assombrie, impossible à confondre avec un chargement
                        for px in img.pixels.iter_mut() {
                            let g = (0.299 * px.r() as f32 + 0.587 * px.g() as f32 + 0.114 * px.b() as f32) * 0.62;
                            let g = g as u8;
                            *px = Color32::from_rgb(g, g, g);
                        }
                    }
                    self.still = Some((id, ui.ctx().load_texture("still", img, egui::TextureOptions::LINEAR), area.size(), !waiting));
                }
                None => {
                    self.still_after = Instant::now() + Duration::from_secs(1);
                    return;
                }
            }
        }
        let Some((_, tex, _, _)) = &self.still else { return };
        self.video_px = None;
        let pt = ui.painter();
        pt.image(tex.id(), area, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        if waiting {
            pt.rect_filled(area, CornerRadius::ZERO, Color32::from_black_alpha(90));
        }
        let c = area.center();
        let rad = (area.height() * 0.09).clamp(26.0, 46.0);
        pt.circle_filled(c, rad, Color32::from_black_alpha(150));
        if waiting {
            egui::Spinner::new().size(rad * 1.1).color(Color32::WHITE).paint_at(ui, Rect::from_center_size(c, Vec2::splat(rad * 1.1)));
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        } else {
            icons::paint(pt, Rect::from_center_size(c, Vec2::splat(rad * 1.15)), Ic::Pause, Color32::WHITE);
        }
    }

    fn draw_status(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::bottom("status").frame(Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 5))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let left = match self.view.feed_key().and_then(|k| self.feeds.get(&k)) {
                    Some(f) if f.loading && f.items.is_empty() => "Chargement…".to_string(),
                    Some(f) => format!("{} élément(s){}", f.items.len(), if f.cont.is_some() { " · défile pour la suite" } else { "" }),
                    None => String::new(),
                };
                ui.label(RichText::new(left).size(11.5).color(p.muted));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(v) = &self.ytdlp_version {
                        ui.label(RichText::new(format!("yt-dlp {v}")).size(11.5).color(p.muted));
                    }
                    if self.stats {
                        if let Some(l) = self.stats_line() {
                            ui.label(RichText::new(format!("{l}  ·  ")).size(11.5).color(p.accent));
                        }
                    } else if let Some(w) = &self.watch {
                        if let Some(pl) = &w.player {
                            let (r, d) = pl.stats();
                            ui.label(RichText::new(format!("{} · {} images, {} perdues  ·  ", self.quality_label(), r, d)).size(11.5).color(p.muted));
                        }
                    }
                    if !self.extracting.is_empty() {
                        ui.add(egui::Spinner::new().size(11.0));
                    }
                });
            });
        });
    }

    // ---------- Volet latéral ----------
    fn draw_sidebar(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::Panel::left("side")
            .resizable(true)
            .default_size(self.s.side_w)
            .min_size(170.0)
            .max_size(300.0)
            .frame(Frame::new().fill(p.side).inner_margin(Margin::symmetric(10, 14)))
            .show(ui, |ui| {
                self.s.side_w = ui.available_width() + 20.0;
                section(ui, &p, "YouTube");
                let entries: [(View, Ic, &str); 5] = [
                    (View::Home, Ic::Home, "Accueil"),
                    (View::Subs, Ic::Subs, "Abonnements"),
                    (View::Later, Ic::Clock, "À regarder plus tard"),
                    (View::Playlists, Ic::Playlist, "Playlists"),
                    (View::History, Ic::History, "Historique"),
                ];
                for (i, (v, ic, label)) in entries.into_iter().enumerate() {
                    let on = self.view == v || (matches!(self.view, View::Playlist(..)) && v == View::Playlists);
                    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                    let bg = if on { p.sel_bg } else if r.hovered() { p.hover_bg } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(7), bg);
                    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(20.0, 20.0)), ic, if on { p.accent } else { p.muted });
                    ui.painter().text(pos2(rect.left() + 38.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.5), p.text);
                    if r.on_hover_text(format!("Ctrl+{}", i + 1)).clicked() {
                        self.go(v);
                    }
                }
                if let View::Search(q) = &self.view {
                    let q = q.clone();
                    section(ui, &p, "Recherche");
                    ui.label(RichText::new(format!("« {q} »")).color(p.text));
                }
                // vidéo en cours (lecture en fond pendant la navigation)
                if self.watch.is_some() && self.view != View::Watch {
                    section(ui, &p, "En cours");
                    let (title, thumb, playing, t, d) = {
                        let w = self.watch.as_ref().unwrap();
                        let pl = w.player.as_ref();
                        (w.item.title.clone(), w.item.thumb.clone(), pl.is_some_and(|x| x.wants_play()), pl.map(|x| x.time()).unwrap_or(0.0), pl.map(|x| x.duration).unwrap_or(0.0))
                    };
                    let width = ui.available_width();
                    let (rect, r) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0 + 46.0), Sense::click());
                    let bg = if r.hovered() { p.hover_bg } else { p.soft };
                    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
                    let th = Rect::from_min_size(rect.min, vec2(width, width * 9.0 / 16.0));
                    let th = self.paint_thumb(ui, th, &thumb, Some(bg));
                    if d > 0.0 {
                        let pr = Rect::from_min_size(pos2(th.left(), th.bottom() - 3.0), vec2(width * (t / d) as f32, 3.0));
                        ui.painter().rect_filled(pr, CornerRadius::ZERO, RED);
                    }
                    let g = galley(ui, &title, FontId::proportional(12.5), p.text, width - 44.0, 2);
                    ui.painter().galley(pos2(rect.left() + 8.0, th.bottom() + 6.0), g, p.text);
                    let pb = Rect::from_center_size(pos2(rect.right() - 20.0, th.bottom() + 22.0), vec2(30.0, 30.0));
                    if icon_btn(ui, pb, "side_pp", Gl::I(if playing { Ic::Pause } else { Ic::Play }), "Lecture / pause", p.text, p.hover_bg).clicked() {
                        self.toggle();
                    } else if r.on_hover_text("Revenir à la vidéo").clicked() {
                        self.go(View::Watch);
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Mini-lecteur").clicked() {
                            self.toggle_mini();
                        }
                        if ui.small_button("Arrêter").clicked() {
                            self.stop();
                        }
                    });
                }
            });
    }

    // ---------- Grilles de vidéos ----------
    /// Miniature 16:9. Avec `bg` (couleur du fond sous la carte), elle est dessinée au pixel près
    /// (recopie directe, rapide) et ses coins arrondis sont masqués ; sinon taille libre.
    /// Renvoie le rectangle réellement occupé.
    fn paint_thumb(&mut self, ui: &mut Ui, rect: Rect, url: &str, bg: Option<Color32>) -> Rect {
        let p = self.pal;
        let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let Some(bg) = bg else {
            if let Some((t, _)) = self.thumbs.get(url, 0) {
                let sz = t.size_vec2();
                // miniatures 4:3 avec bandes noires (hqdefault) : on recadre en 16:9
                let uv = if sz.x / sz.y.max(1.0) < 1.5 { Rect::from_min_max(pos2(0.0, 0.125), pos2(1.0, 0.875)) } else { full_uv };
                egui::Image::new(egui::load::SizedTexture::new(t.id(), sz)).uv(uv).corner_radius(CornerRadius::same(8)).paint_at(ui, rect);
            } else {
                ui.painter().rect_filled(rect, CornerRadius::same(8), p.soft);
            }
            return rect;
        };
        let ppp = ui.ctx().pixels_per_point();
        let wpx = (((rect.width() * ppp) as u32) / 4 * 4).max(16);
        let hpx = (wpx as f32 * 9.0 / 16.0).round();
        let r = Rect::from_min_size(pos2((rect.left() * ppp).round() / ppp, (rect.top() * ppp).round() / ppp), vec2(wpx as f32 / ppp, hpx / ppp));
        match self.thumbs.get(url, wpx) {
            Some((t, _)) => {
                // texture plus grande d'un pixel (voir thumbs.rs) : on n'en montre que w × h
                let sz = t.size_vec2();
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((sz.x - 1.0) / sz.x, (sz.y - 1.0) / sz.y));
                ui.painter().image(t.id(), r, uv, Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(r, CornerRadius::ZERO, p.soft);
            }
        }
        round_corners(ui.painter(), r, 8.0, bg);
        r
    }

    /// Aperçu vidéo muet dans la vignette survolée (comme sur le site), après ~0,7 s de survol.
    fn preview_at(&mut self, ui: &mut Ui, id: &str, r: Rect) {
        if !self.s.hover_preview || self.mode != Mode::Normal || self.menu.is_some() {
            return;
        }
        let px = to_px(r, ui.ctx().pixels_per_point());
        if let Some(pv) = self.preview.as_mut().filter(|p| p.id == id) {
            pv.touched = true;
            if let Some(pl) = pv.player.as_mut() {
                pl.poll();
                // la fenêtre n'apparaît qu'une fois la vidéo réellement lancée (pas de flash noir)
                if pv.shown || (pl.ready && pl.time() > 0.15) {
                    if !pv.shown && std::env::var_os("TUBE_SCRIPT").is_some() {
                        crate::store::log(&format!("aperçu {} affiché en {px:?}", pv.id));
                    }
                    pv.win.place(px, &[]);
                    if !pv.shown {
                        pl.set_output(px.2, px.3);
                    }
                    pv.shown = true;
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(if pv.shown { 500 } else { 120 }));
            return;
        }
        let long_enough = self.hover.as_ref().is_some_and(|h| h.0 == id && h.1.elapsed() > Duration::from_millis(650));
        let Some(info) = self.cached(id).filter(|_| long_enough) else {
            ui.ctx().request_repaint_after(Duration::from_millis(150));
            return;
        };
        let (v, _, _, _) = crate::app::pick(&info, 480, None);
        let Some(v) = v else { return };
        let Some(win) = crate::vidwin::VideoWindow::new(self.hwnd) else { return };
        let c2 = ui.ctx().clone();
        let player = crate::player::Player::open(win.hwnd.0 as isize, (&v.url, &v.ext), ("", ""), 0.0, true, 0.0, 1.0, Arc::new(move || c2.request_repaint())).ok();
        if let Some(pl) = &player {
            pl.set_output(px.2, px.3);
        }
        self.preview = Some(crate::app::Preview { id: id.to_string(), player, win, touched: true, shown: false });
        ui.ctx().request_repaint_after(Duration::from_millis(120));
    }

    fn card(&mut self, ui: &mut Ui, rect: Rect, it: &Item, idx: usize, list: &[Item]) {
        let p = self.pal;
        let r = ui.interact(rect, Id::new(("card", &it.id)), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect.expand(4.0), CornerRadius::same(10), p.hover_bg);
        }
        let th = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0));
        let th = self.paint_thumb(ui, th, &it.thumb, Some(if r.hovered() { p.hover_bg } else { p.surface }));
        if r.hovered() && it.kind == Kind::Video && !it.live {
            self.preview_at(ui, &it.id, th);
        }
        if it.kind == Kind::Playlist {
            let band = Rect::from_min_max(pos2(th.right() - th.width() * 0.32, th.top()), th.max);
            ui.painter().rect_filled(band, CornerRadius { nw: 0, ne: 8, sw: 0, se: 8 }, Color32::from_black_alpha(170));
            ui.painter().text(band.center(), Align2::CENTER_CENTER, format!("☰\n{}", it.duration), FontId::proportional(13.0), Color32::WHITE);
        } else if !it.duration.is_empty() {
            let g = galley(ui, &it.duration, FontId::proportional(11.5), Color32::WHITE, 120.0, 1);
            let b = Rect::from_min_size(pos2(th.right() - g.size().x - 12.0, th.bottom() - g.size().y - 10.0), g.size() + vec2(8.0, 4.0));
            ui.painter().rect_filled(b, CornerRadius::same(4), if it.live { RED } else { Color32::from_black_alpha(200) });
            ui.painter().galley(b.min + vec2(4.0, 2.0), g, Color32::WHITE);
        }
        let prog = self.s.positions.get(&it.id).map(|x| (x.0 / x.1.max(1.0)) as f32).unwrap_or(it.progress);
        if prog > 0.0 {
            let bar = Rect::from_min_size(pos2(th.left(), th.bottom() - 4.0), vec2(th.width(), 4.0));
            ui.painter().rect_filled(bar, CornerRadius::ZERO, Color32::from_white_alpha(70));
            ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(th.width() * prog.min(1.0), 4.0)), CornerRadius::ZERO, RED);
        }
        let x = rect.left() + 2.0;
        let mut y = th.bottom() + 8.0;
        let k = card_scale(rect.width());
        let sub = sub_text(&p);
        let g = galley(ui, &it.title, FontId::new(14.0 * k, egui::FontFamily::Name("bold".into())), p.text, rect.width() - 4.0, 2);
        let h = g.size().y;
        ui.painter().galley(pos2(x, y), g, p.text);
        y += h + 3.0;
        if !it.channel.is_empty() {
            let g = galley(ui, &it.channel, FontId::proportional(12.5 * k), sub, rect.width() - 4.0, 1);
            let cr = Rect::from_min_size(pos2(x, y), g.size());
            if !it.channel_id.is_empty() {
                let c = ui.interact(cr, Id::new(("chan", &it.id)), Sense::click());
                let col = if c.hovered() { p.text } else { sub };
                if c.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    ui.painter().hline(cr.x_range(), cr.bottom(), Stroke::new(1.0, col));
                }
                ui.painter().galley(cr.min, g, col);
                if c.clicked() {
                    self.open_channel(&it.channel_id, &it.channel, false);
                } else if c.middle_clicked() {
                    self.open_channel(&it.channel_id, &it.channel, true);
                }
            } else {
                ui.painter().galley(pos2(x, y), g, sub);
            }
            y += 17.0 * k;
        }
        if !it.meta.is_empty() {
            let g = galley(ui, &it.meta, FontId::proportional(12.0 * k), sub, rect.width() - 4.0, 1);
            ui.painter().galley(pos2(x, y), g, sub);
        }
        if r.hovered() {
            self.hovering(Some(&it.id));
        }
        r.context_menu(|ui| self.item_menu(ui, it, Some((list, idx))));
        if r.middle_clicked() {
            self.open_in_tab(it);
        }
        if r.clicked() {
            self.open(it, Some((list.to_vec(), idx)));
        }
    }

    fn draw_feed(&mut self, ui: &mut Ui, v: &View) {
        let Some(key) = v.feed_key() else { return };
        if !self.feeds.contains_key(&key) {
            self.load_feed(v, false);
        }
        // les éléments sont empruntés le temps de l'image (pas de copie de toute la liste à chaque image)
        let (items, loading, error, has_more) = {
            let f = self.feeds.get_mut(&key).unwrap();
            (std::mem::take(&mut f.items), f.loading, f.error.clone(), f.cont.is_some())
        };
        self.draw_feed_items(ui, v, &key, &items, loading, error, has_more);
        if let Some(f) = self.feeds.get_mut(&key) {
            if f.items.is_empty() {
                f.items = items;
            } else {
                // une page est arrivée pendant l'image : elle s'ajoute après
                let more = std::mem::take(&mut f.items);
                f.items = items;
                f.items.extend(more);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_feed_items(&mut self, ui: &mut Ui, v: &View, key: &str, items: &Vec<Item>, loading: bool, error: Option<String>, has_more: bool) {
        let p = self.pal;
        let title = match v {
            View::Home => "Accueil".to_string(),
            View::Subs => "Abonnements".into(),
            View::Later => "À regarder plus tard".into(),
            View::Playlists => "Playlists".into(),
            View::History => "Historique".into(),
            View::Search(q) => format!("Résultats pour « {q} »"),
            View::Playlist(_, t) => t.clone(),
            View::Channel(_, n) => n.clone(),
            _ => String::new(),
        };
        Frame::new().inner_margin(Margin { left: 22, right: 22, top: 14, bottom: 4 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&title).size(20.0).strong());
                if matches!(v, View::Playlist(..) | View::Later) && !items.is_empty() {
                    ui.add_space(10.0);
                    if ui.button("Tout lire").clicked() {
                        let first = items[0].clone();
                        self.open(&first, Some((items.clone(), 0)));
                    }
                }
                if loading {
                    ui.add(egui::Spinner::new().size(16.0));
                }
            });
        });
        if let Some(e) = &error {
            Frame::new().inner_margin(Margin::symmetric(22, 6)).show(ui, |ui| {
                ui.label(RichText::new(format!("Erreur : {e}")).color(p.del));
                if ui.button("Réessayer").clicked() {
                    self.load_feed(v, false);
                }
            });
        }
        if items.is_empty() && !loading && error.is_none() {
            Frame::new().inner_margin(Margin::symmetric(22, 10)).show(ui, |ui| {
                ui.label(RichText::new("Rien à afficher ici.").color(p.muted));
            });
            return;
        }
        let gap = 18.0;
        let avail = ui.available_width() - 44.0;
        // Ctrl+molette (ou Ctrl+/Ctrl−) : vignettes plus grandes ou plus petites
        let zoom = ui.ctx().input(|i| i.zoom_delta());
        if zoom != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
            self.s.card_w = (self.s.card_w * zoom.powf(1.5)).clamp(220.0, 900.0);
        }
        let cols = ((avail + gap) / (self.s.card_w + gap)).round().max(1.0) as usize;
        let cw = (avail - gap * (cols - 1) as f32) / cols as f32;
        let ch = cw * 9.0 / 16.0 + 82.0 * card_scale(cw);
        let rows = items.len().div_ceil(cols);
        let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt(("feed", key));
        if self.scroll_to_top {
            sa = sa.vertical_scroll_offset(0.0);
            self.scroll_to_top = false;
        }
        if self.dbg_scroll.0 > 0.0 {
            self.dbg_scroll.1 += self.dbg_scroll.0;
            sa = sa.vertical_scroll_offset(self.dbg_scroll.1);
            ui.ctx().request_repaint();
        }
        let mut need_more = false;
        let mut hovered_any = false;
        // accueil : rangée « Reprendre » (vidéos commencées, de la plus récente à la plus ancienne)
        let shelf: Vec<Item> = if *v == View::Home {
            let mut r: Vec<(u64, Item)> = self.s.recent.iter().filter_map(|it| self.s.positions.get(&it.id).map(|p| (p.2, it.clone()))).collect();
            r.sort_by(|a, b| b.0.cmp(&a.0));
            r.into_iter().map(|x| x.1).take(cols).collect()
        } else {
            Vec::new()
        };
        let extra = usize::from(!shelf.is_empty());
        sa.show_rows(ui, ch + gap, rows + extra + 1, |ui, range| {
            for row in range {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ch + gap), Sense::hover());
                if extra == 1 && row == 0 {
                    for (c, it) in shelf.iter().enumerate() {
                        let r = Rect::from_min_size(pos2(rect.left() + 22.0 + c as f32 * (cw + gap), rect.top()), vec2(cw, ch));
                        if ui.rect_contains_pointer(r) {
                            hovered_any = true;
                        }
                        self.card(ui, r, it, c, &shelf);
                        let g = galley(ui, "Reprendre", FontId::proportional(11.5), Color32::WHITE, 200.0, 1);
                        let b = Rect::from_min_size(r.min + vec2(8.0, 8.0), g.size() + vec2(12.0, 6.0));
                        ui.painter().rect_filled(b, CornerRadius::same(5), RED);
                        ui.painter().galley(b.min + vec2(6.0, 3.0), g, Color32::WHITE);
                    }
                    continue;
                }
                let row = row - extra;
                if row >= rows {
                    if has_more {
                        need_more = true;
                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Chargement de la suite…", FontId::proportional(13.0), p.muted);
                    }
                    continue;
                }
                if row + 2 >= rows {
                    need_more |= has_more;
                }
                for c in 0..cols {
                    let i = row * cols + c;
                    let Some(it) = items.get(i) else { break };
                    let r = Rect::from_min_size(pos2(rect.left() + 22.0 + c as f32 * (cw + gap), rect.top()), vec2(cw, ch));
                    if ui.rect_contains_pointer(r) {
                        hovered_any = true;
                    }
                    self.card(ui, r, it, i, items);
                }
            }
        });
        if !hovered_any {
            self.hovering(None);
        }
        if need_more && !loading {
            self.load_feed(v, true);
        }
    }

    // ---------- Lecture ----------
    fn draw_watch(&mut self, ui: &mut Ui) {
        let p = self.pal;
        if self.watch.is_none() {
            ui.label("Aucune vidéo.");
            return;
        }
        let full = ui.available_rect_before_wrap();
        let ppp = ui.ctx().pixels_per_point();
        let side_w = if full.width() > 1250.0 { 400.0 } else { 0.0 };
        let left = Rect::from_min_max(full.min + vec2(18.0, 14.0), pos2(full.right() - side_w - 18.0, full.bottom()));
        let (vw, vh) = self.watch.as_ref().and_then(|w| w.player.as_ref()).map(|pl| pl.size).filter(|s| s.0 > 0).unwrap_or((16, 9));
        let aspect = (vw as f32 / vh as f32).clamp(0.5, 3.0);
        let max_h = (left.height() - 150.0).max(200.0);
        let mut w = left.width();
        let mut h = w / aspect;
        if h > max_h {
            h = max_h;
            w = h * aspect;
        }
        let video = Rect::from_min_size(pos2(left.left() + (left.width() - w) / 2.0, left.top()), vec2(w, h));
        let stage = Rect::from_min_max(pos2(left.left(), video.top()), pos2(left.right(), video.bottom()));
        ui.painter().rect_filled(stage, CornerRadius::same(10), Color32::BLACK);
        self.video_px = Some(to_px(video, ppp));
        self.draw_still(ui, video);
        let vr = ui.interact(stage, Id::new("stage"), Sense::click());
        if std::env::var_os("TUBE_SCRIPT").is_some() {
            let (pr, rel, pos) = ui.input(|i| (i.pointer.any_pressed(), i.pointer.any_released(), i.pointer.interact_pos()));
            if pr || rel {
                crate::store::log(&format!("souris : appui {pr} relâché {rel} pos {pos:?} scène {stage:?} survol {} clic {}", vr.hovered(), vr.clicked()));
            }
        }
        if vr.double_clicked() {
            self.toggle_full();
        } else if vr.clicked() {
            self.toggle();
        }
        self.draw_video_state(ui, video);
        self.draw_subs(ui, video, 0.0);
        let bar = Rect::from_min_size(pos2(left.left(), video.bottom() + 6.0), vec2(left.width(), 46.0));
        self.draw_controls(ui, bar, false);
        // titre, chaîne, description
        let info = Rect::from_min_max(pos2(left.left(), bar.bottom() + 6.0), left.max);
        let mut cui = ui.new_child(egui::UiBuilder::new().max_rect(info).layout(Layout::top_down(Align::Min)));
        egui::ScrollArea::vertical().id_salt("watch_info").auto_shrink([false, false]).show(&mut cui, |ui| {
            let mut chan = None;
            let Some(wt) = self.watch.as_mut() else { return };
            ui.label(RichText::new(&wt.item.title).size(19.0).strong());
            ui.horizontal_wrapped(|ui| {
                if !wt.item.channel.is_empty() {
                    let cid = if wt.item.channel_id.is_empty() { wt.info.as_ref().and_then(|i| i["channel_id"].as_str()).unwrap_or("").to_string() } else { wt.item.channel_id.clone() };
                    let r = ui.add(egui::Label::new(RichText::new(&wt.item.channel).strong().color(p.accent)).sense(Sense::click()));
                    if !cid.is_empty() {
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                        if r.clicked() || r.middle_clicked() {
                            chan = Some((cid, wt.item.channel.clone(), r.middle_clicked()));
                        }
                    }
                    ui.label(RichText::new("·").color(p.muted));
                }
                if let Some(info) = &wt.info {
                    if let Some(v) = info["views"].as_u64() {
                        ui.label(RichText::new(format!("{} vues", group(v))).color(sub_text(&p)));
                    }
                    if let Some(d) = info["upload_date"].as_str().filter(|d| d.len() == 8) {
                        ui.label(RichText::new(format!("· {}/{}/{}", &d[6..8], &d[4..6], &d[0..4])).color(p.muted));
                    }
                }
                if !wt.sponsors.is_empty() {
                    ui.label(RichText::new(format!("· SponsorBlock : {} segment(s)", wt.sponsors.len())).color(SB));
                }
            });
            if let Some(e) = &wt.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(p.del));
            }
            let desc = wt.info.as_ref().and_then(|i| i["description"].as_str()).unwrap_or("").to_string();
            let chapters: Vec<(f64, String)> = wt
                .info
                .as_ref()
                .and_then(|i| i["chapters"].as_array().cloned())
                .unwrap_or_default()
                .iter()
                .map(|c| (c["start"].as_f64().unwrap_or(0.0), c["title"].as_str().unwrap_or("").to_string()))
                .collect();
            let mut seek = None;
            if !desc.is_empty() || !chapters.is_empty() {
                ui.add_space(8.0);
                Frame::new().fill(p.side).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if !chapters.is_empty() {
                        ui.label(RichText::new("Chapitres").strong());
                        ui.horizontal_wrapped(|ui| {
                            for (t, name) in &chapters {
                                if ui.small_button(format!("{} {}", fmt_time(*t), name)).clicked() {
                                    seek = Some(*t);
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                    let short: String = if wt.desc_open { desc.clone() } else { desc.lines().take(4).collect::<Vec<_>>().join("\n") };
                    ui.label(RichText::new(short).size(13.0).color(p.text));
                    if desc.lines().count() > 4 && ui.add(egui::Button::new(RichText::new(if wt.desc_open { "Moins" } else { "Plus…" }).color(p.accent)).frame(false)).clicked() {
                        wt.desc_open = !wt.desc_open;
                    }
                });
            }
            if let Some(t) = seek {
                self.seek_to(t);
            }
            if let Some((id, name, tab)) = chan {
                self.open_channel(&id, &name, tab);
            }
        });
        // suggestions / file d'attente
        if side_w > 0.0 {
            let side = Rect::from_min_max(pos2(full.right() - side_w, full.top() + 14.0), pos2(full.right() - 14.0, full.bottom()));
            let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(side).layout(Layout::top_down(Align::Min)));
            self.draw_related(&mut sui);
        }
    }

    fn draw_related(&mut self, ui: &mut Ui) {
        let p = self.pal;
        let (queue, qpos) = (self.queue.clone(), self.qpos);
        let related = self.watch.as_ref().map(|w| w.related.clone()).unwrap_or_default();
        egui::ScrollArea::vertical().id_salt("related").auto_shrink([false, false]).show(ui, |ui| {
            if queue.len() > 1 {
                ui.label(RichText::new(format!("File d'attente · {}/{}", qpos + 1, queue.len())).strong());
                ui.add_space(4.0);
                for (i, it) in queue.iter().enumerate().skip(qpos).take(30) {
                    self.compact(ui, it, Some((&queue, i)), i == qpos);
                }
                ui.add_space(10.0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("À suivre").strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.checkbox(&mut self.s.autoplay_next, RichText::new("Lecture auto").size(12.0).color(p.muted));
                });
            });
            ui.add_space(4.0);
            if related.is_empty() {
                ui.add(egui::Spinner::new());
            }
            for it in &related {
                self.compact(ui, it, None, false);
            }
        });
    }

    fn compact(&mut self, ui: &mut Ui, it: &Item, q: Option<(&Vec<Item>, usize)>, current: bool) {
        let p = self.pal;
        let width = ui.available_width();
        let (rect, r) = ui.allocate_exact_size(vec2(width, 96.0), Sense::click());
        if r.hovered() || current {
            ui.painter().rect_filled(rect, CornerRadius::same(8), if current { p.sel_bg } else { p.hover_bg });
        }
        let th = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(164.0, 92.0));
        let th = self.paint_thumb(ui, th, &it.thumb, Some(if current { p.sel_bg } else if r.hovered() { p.hover_bg } else { p.surface }));
        if !it.duration.is_empty() {
            let g = galley(ui, &it.duration, FontId::proportional(11.0), Color32::WHITE, 100.0, 1);
            let b = Rect::from_min_size(pos2(th.right() - g.size().x - 10.0, th.bottom() - g.size().y - 8.0), g.size() + vec2(6.0, 3.0));
            ui.painter().rect_filled(b, CornerRadius::same(3), Color32::from_black_alpha(200));
            ui.painter().galley(b.min + vec2(3.0, 1.5), g, Color32::WHITE);
        }
        let x = th.right() + 10.0;
        let tw = rect.right() - x - 4.0;
        let g = galley(ui, &it.title, FontId::new(13.0, egui::FontFamily::Name("bold".into())), p.text, tw, 2);
        let h = g.size().y;
        ui.painter().galley(pos2(x, rect.top() + 4.0), g, p.text);
        let sub = sub_text(&p);
        let g = galley(ui, &it.channel, FontId::proportional(12.0), sub, tw, 1);
        ui.painter().galley(pos2(x, rect.top() + 8.0 + h), g, sub);
        let g = galley(ui, &it.meta, FontId::proportional(11.5), sub, tw, 1);
        ui.painter().galley(pos2(x, rect.top() + 25.0 + h), g, sub);
        if r.hovered() {
            self.hovering(Some(&it.id));
        }
        r.context_menu(|ui| self.item_menu(ui, it, q.map(|(l, i)| (l.as_slice(), i))));
        if r.middle_clicked() {
            self.open_in_tab(it);
        }
        if r.clicked() {
            self.open(it, q.map(|(l, i)| (l.clone(), i)));
        }
    }

    /// Chargement / mise en mémoire tampon / erreur, dessinés autour de la vidéo.
    fn draw_video_state(&mut self, ui: &mut Ui, video: Rect) {
        let Some(w) = &self.watch else { return };
        if w.audio_only && w.error.is_none() {
            // audio seul : la miniature en fond, et le spectre du son joué par-dessus
            self.video_px = None;
            self.still = None;
            let playing = w.player.as_ref().is_some_and(|p| p.wants_play());
            let thumb = w.item.thumb.clone();
            self.paint_thumb(ui, video, &thumb, None);
            if self.viz.is_none() && !self.viz_failed && playing {
                self.viz = crate::viz::Viz::start();
                self.viz_failed = self.viz.is_none();
            }
            let bands = ((video.width() / 14.0) as usize).clamp(24, 96);
            let target = match &self.viz {
                Some(v) if playing => v.spectrum(bands),
                _ => vec![0.0; bands],
            };
            if self.viz_bars.len() != bands {
                self.viz_bars = vec![0.0; bands];
            }
            // montée rapide, descente douce
            for (b, t) in self.viz_bars.iter_mut().zip(&target) {
                *b = if *t > *b { *b + (*t - *b) * 0.6 } else { *b + (*t - *b) * 0.18 };
            }
            let pt = ui.painter();
            // dégradé sombre en bas pour la lisibilité (l'image reste en couleurs)
            let fade = Rect::from_min_max(pos2(video.left(), video.bottom() - video.height() * 0.45), video.max);
            let mut m = egui::Mesh::default();
            m.colored_vertex(fade.left_top(), Color32::TRANSPARENT);
            m.colored_vertex(fade.right_top(), Color32::TRANSPARENT);
            m.colored_vertex(fade.right_bottom(), Color32::from_black_alpha(190));
            m.colored_vertex(fade.left_bottom(), Color32::from_black_alpha(190));
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            pt.add(m);
            let base = video.bottom() - 14.0;
            let maxh = video.height() * 0.38;
            let slot = (video.width() - 40.0) / bands as f32;
            for (i, h) in self.viz_bars.iter().enumerate() {
                let x = video.left() + 20.0 + i as f32 * slot;
                let hh = (h * maxh).max(2.0);
                let a = (150.0 + 105.0 * h) as u8;
                let r = Rect::from_min_max(pos2(x + slot * 0.18, base - hh), pos2(x + slot * 0.82, base));
                pt.rect_filled(r, CornerRadius::same(2), Color32::from_white_alpha(a));
            }
            if self.viz_failed {
                pt.text(pos2(video.center().x, video.bottom() - 24.0), Align2::CENTER_CENTER, "Audio seul", FontId::proportional(14.0), Color32::WHITE);
            }
            if playing {
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            return;
        }
        let msg = match (&w.player, &w.error) {
            (_, Some(e)) => Some(e.lines().next().unwrap_or("").to_string()),
            (None, None) => Some("Préparation des flux…".to_string()),
            (Some(pl), None) if !pl.ready => Some("Connexion…".to_string()),
            _ => None,
        };
        if let Some(m) = msg {
            // tant que rien n'est affiché, la sous-fenêtre vidéo est masquée : on dessine sur le fond noir
            if w.player.as_ref().is_none_or(|pl| !pl.ready) {
                self.video_px = None;
                self.paint_thumb(ui, video, &w.item.thumb.clone(), None);
                ui.painter().rect_filled(video, CornerRadius::same(8), Color32::from_black_alpha(150));
                ui.painter().text(video.center() + vec2(0.0, 28.0), Align2::CENTER_CENTER, m, FontId::proportional(15.0), Color32::WHITE);
                if self.watch.as_ref().is_some_and(|w| w.error.is_none()) {
                    egui::Spinner::new().size(28.0).color(Color32::WHITE).paint_at(ui, Rect::from_center_size(video.center() - vec2(0.0, 10.0), vec2(28.0, 28.0)));
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                }
            }
        }
    }

    /// Mode cinéma : dégradés dans les bandes noires, des couleurs du bord de l'image vers le noir.
    fn draw_ambient(&mut self, ui: &mut Ui, full: Rect, vr: Rect) {
        let vertical = full.width() - vr.width() > full.height() - vr.height();
        let bands = 24;
        // relevé 4 fois par seconde, lissé dans le temps (pas de clignotement)
        let due = self.ambient.as_ref().is_none_or(|a| a.2.elapsed() > Duration::from_millis(240));
        let paused = self.watch.as_ref().and_then(|w| w.player.as_ref()).is_none_or(|p| !p.wants_play());
        if due && self.still.is_none() {
            if let Some((l, r)) = self.vidwin.as_ref().and_then(|v| v.edge_colors(vertical, bands)) {
                let mix = |old: &mut Vec<[f32; 3]>, new: Vec<[f32; 3]>| {
                    if old.len() != new.len() {
                        *old = new;
                    } else {
                        for (o, n) in old.iter_mut().zip(new) {
                            for k in 0..3 {
                                o[k] = o[k] * 0.55 + n[k] * 0.45;
                            }
                        }
                    }
                };
                match self.ambient.as_mut() {
                    Some(a) => {
                        mix(&mut a.0, l);
                        mix(&mut a.1, r);
                        a.2 = Instant::now();
                    }
                    None => self.ambient = Some((l, r, Instant::now())),
                }
            }
        }
        if !paused {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        let Some((l, r, _)) = &self.ambient else { return };
        // lissage le long du bord (moyenne avec les voisines)
        let smooth = |v: &Vec<[f32; 3]>| -> Vec<[f32; 3]> {
            (0..v.len())
                .map(|i| {
                    let a = v[i.saturating_sub(1)];
                    let b = v[i];
                    let c = v[(i + 1).min(v.len() - 1)];
                    [(a[0] + 2.0 * b[0] + c[0]) / 4.0, (a[1] + 2.0 * b[1] + c[1]) / 4.0, (a[2] + 2.0 * b[2] + c[2]) / 4.0]
                })
                .collect()
        };
        let (l, r) = (smooth(l), smooth(r));
        let dim = if paused { 0.35 } else { 1.0 };
        let col = |c: [f32; 3], f: f32| Color32::from_rgb((c[0] * f * dim) as u8, (c[1] * f * dim) as u8, (c[2] * f * dim) as u8);
        let mut mesh = egui::Mesh::default();
        // grille : une ligne de sommets par bande ; trois colonnes (bord de l'image, milieu, bord de l'écran)
        let falloff = [0.85f32, 0.38, 0.06];
        let mut quad_grid = |cols: &Vec<[f32; 3]>, pos: &dyn Fn(usize, f32) -> egui::Pos2| {
            let n = cols.len();
            let base = mesh.vertices.len() as u32;
            for i in 0..=n {
                let c = cols[i.min(n - 1)];
                let c = if i > 0 && i < n { let p = cols[i - 1]; [(p[0] + c[0]) / 2.0, (p[1] + c[1]) / 2.0, (p[2] + c[2]) / 2.0] } else { c };
                for (k, f) in falloff.iter().enumerate() {
                    mesh.colored_vertex(pos(i, k as f32 / 2.0), col(c, *f));
                }
            }
            for i in 0..n as u32 {
                for k in 0..2u32 {
                    let a = base + i * 3 + k;
                    mesh.add_triangle(a, a + 1, a + 3);
                    mesh.add_triangle(a + 1, a + 4, a + 3);
                }
            }
        };
        let nb = l.len().max(1) as f32;
        if vertical {
            let lw = vr.left() - full.left();
            quad_grid(&l, &|i, t| pos2(vr.left() - lw * t, full.top() + full.height() * i as f32 / nb));
            let rw = full.right() - vr.right();
            quad_grid(&r, &|i, t| pos2(vr.right() + rw * t, full.top() + full.height() * i as f32 / nb));
        } else {
            let th = vr.top() - full.top();
            quad_grid(&l, &|i, t| pos2(full.left() + full.width() * i as f32 / nb, vr.top() - th * t));
            let bh = full.bottom() - vr.bottom();
            quad_grid(&r, &|i, t| pos2(full.left() + full.width() * i as f32 / nb, vr.bottom() + bh * t));
        }
        ui.painter().add(mesh);
    }

    /// Planche d'aperçu et zone (en pixels de la planche) de l'image à l'instant `t`.
    fn storyboard_at(&self, t: f64) -> Option<(String, Rect)> {
        let sb = &self.watch.as_ref()?.info.as_ref()?["storyboard"];
        let (w, h) = (sb["w"].as_f64()? as f32, sb["h"].as_f64()? as f32);
        let (rows, cols) = (sb["rows"].as_u64()?.max(1), sb["cols"].as_u64()?.max(1));
        let fps = sb["fps"].as_f64().filter(|f| *f > 0.0)?;
        let urls = sb["urls"].as_array()?;
        let idx = (t * fps) as u64;
        let per = rows * cols;
        let sheet = ((idx / per) as usize).min(urls.len().saturating_sub(1));
        let k = if (idx / per) as usize > sheet { per - 1 } else { idx % per };
        let (c, r) = ((k % cols) as f32, (k / cols) as f32);
        Some((urls[sheet].as_str()?.to_string(), Rect::from_min_size(pos2(c * w, r * h), vec2(w, h))))
    }

    /// Barre de contrôles : lecture, sauts, temps, barre de progression, volume, vitesse, qualité, mini-lecteur, plein écran.
    fn draw_controls(&mut self, ui: &mut Ui, bar: Rect, overlay: bool) {
        let p = self.pal;
        let (fg, hover, bg) = if overlay { (Color32::WHITE, Color32::from_white_alpha(40), Color32::from_black_alpha(190)) } else { (p.text, p.hover_bg, p.side) };
        ui.painter().rect_filled(bar, CornerRadius::same(if overlay { 0 } else { 10 }), bg);
        let Some(w) = self.watch.as_ref() else { return };
        let (t, d, playing, buf) = w.player.as_ref().map(|pl| (pl.time(), pl.duration, pl.wants_play(), pl.buffered_ahead())).unwrap_or((0.0, 0.0, false, 0.0));
        let sponsors = w.sponsors.clone();
        let chapters: Vec<f64> = w.info.as_ref().and_then(|i| i["chapters"].as_array().cloned()).unwrap_or_default().iter().filter_map(|c| c["start"].as_f64()).collect();
        let compact = bar.width() < 560.0;
        let bh = bar.height().min(46.0);
        let mut x = bar.left() + 4.0;
        let cy = bar.center().y;
        let btn = |ui: &mut Ui, x: &mut f32, id: &str, ic: Ic, tip: &str| {
            let r = Rect::from_center_size(pos2(*x + bh * 0.42, cy), vec2(bh * 0.84, bh * 0.84));
            *x += bh * 0.86;
            icon_btn(ui, r, id, Gl::I(ic), tip, fg, hover)
        };
        let live = !d.is_finite() || self.watch.as_ref().and_then(|w| w.info.as_ref()).is_some_and(|i| i["is_live"].as_bool() == Some(true));
        if btn(ui, &mut x, "c_pp", if playing { Ic::Pause } else { Ic::Play }, "Lecture / pause (Espace, K, clic sur la vidéo)").clicked() {
            self.toggle();
        }
        if !compact {
            if btn(ui, &mut x, "c_back", Ic::Back10, "−10 s (J)").clicked() {
                self.seek_by(-10.0);
            }
            if btn(ui, &mut x, "c_fwd", Ic::Fwd10, "+10 s (L)").clicked() {
                self.seek_by(10.0);
            }
            if self.queue.len() > self.qpos + 1 && btn(ui, &mut x, "c_next", Ic::Next, "Vidéo suivante (Maj+N)").clicked() {
                self.next_in_queue();
                return;
            }
        }
        let behind = self.live_behind();
        let time = match behind {
            Some(b) if b > 15.0 => format!("−{}", fmt_time(b)),
            _ if live => "●  EN DIRECT".to_string(),
            _ => format!("{} / {}", fmt_time(t), fmt_time(d)),
        };
        let tg = galley(ui, &time, FontId::monospace(12.5), fg, 200.0, 1);
        ui.painter().galley(pos2(x + 6.0, cy - tg.size().y / 2.0), tg.clone(), fg);
        x += tg.size().x + 16.0;
        // à droite : volume, vitesse, qualité, mini, plein écran
        let mut rx = bar.right() - 4.0;
        let rbtn = |ui: &mut Ui, rx: &mut f32, id: &str, g: Gl, tip: &str, wdt: f32| {
            let r = Rect::from_center_size(pos2(*rx - wdt / 2.0, cy), vec2(wdt, bh * 0.84));
            *rx -= wdt + 2.0;
            icon_btn(ui, r, id, g, tip, fg, hover)
        };
        let fs = if self.mode == Mode::Full { Ic::Unfullscreen } else { Ic::Fullscreen };
        if rbtn(ui, &mut rx, "c_full", Gl::I(fs), "Plein écran (F, double-clic)", bh * 0.84).clicked() {
            self.toggle_full();
        }
        if !compact {
            let half_tip = if self.mode == Mode::Half { "Quitter le demi-écran (H)" } else { "Demi-écran : toute la hauteur, moitié de la largeur (H, Win+Alt+H)" };
            if rbtn(ui, &mut rx, "c_half", Gl::I(Ic::Half), half_tip, bh * 0.84).clicked() {
                self.toggle_half();
            }
        }
        let mini_tip = if self.mode == Mode::Mini { "Revenir à la fenêtre (I)" } else { "Mini-lecteur (I, Win+Alt+M)" };
        if rbtn(ui, &mut rx, "c_mini", Gl::I(if self.mode == Mode::Mini { Ic::ExitMini } else { Ic::Mini }), mini_tip, bh * 0.84).clicked() {
            self.toggle_mini();
        }
        // roue ⚙ : qualité, vitesse, langue, sous-titres, minuterie, boucle… (pastille 4K / HD)
        let r = rbtn(ui, &mut rx, "c_gear", Gl::I(Ic::Settings), "Paramètres de lecture", bh * 0.84);
        let h = self.watch.as_ref().filter(|w| !w.audio_only).and_then(|w| w.video_fmt.as_ref()).map(|f| f.height).unwrap_or(0);
        let badge = if h >= 2160 { "4K" } else if h >= 1440 { "QHD" } else if h >= 720 { "HD" } else { "" };
        if !badge.is_empty() {
            let g = galley(ui, badge, FontId::proportional(8.5), Color32::WHITE, 40.0, 1);
            let b = Rect::from_min_size(pos2(r.rect.right() - g.size().x - 6.0, r.rect.top() + 3.0), g.size() + vec2(4.0, 1.0));
            ui.painter().rect_filled(b, CornerRadius::same(2), RED);
            ui.painter().galley(b.min + vec2(2.0, 0.5), g, Color32::WHITE);
        }
        if r.clicked() {
            self.menu = if self.menu.is_some() { None } else { Some((Menu::Gear, r.rect)) };
        }
        if self.watch.as_ref().is_some_and(|w| w.sub_sel.is_some()) || self.s.subs_lang.is_some() {
            let on = self.watch.as_ref().is_some_and(|w| w.sub_sel.is_some());
            let r = rbtn(ui, &mut rx, "c_cc", Gl::T("CC"), "Sous-titres (touche V)", bh * 0.84);
            if on {
                ui.painter().hline(r.rect.shrink2(vec2(10.0, 0.0)).x_range(), r.rect.bottom() - 7.0, Stroke::new(2.0, RED));
            }
            if r.clicked() {
                self.toggle_subs();
            }
        }
        if !compact {
            // volume : icône + curseur
            let vr = Rect::from_min_max(pos2(rx - 90.0, cy - 10.0), pos2(rx - 6.0, cy + 10.0));
            let vresp = ui.interact(vr, Id::new("c_vol"), Sense::click_and_drag());
            let vol = if self.s.muted { 0.0 } else { self.s.volume } as f32;
            let track = Rect::from_min_max(pos2(vr.left(), cy - 2.0), pos2(vr.right(), cy + 2.0));
            ui.painter().rect_filled(track, CornerRadius::same(2), if overlay { Color32::from_white_alpha(70) } else { p.border });
            ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(track.left() + track.width() * vol, track.bottom())), CornerRadius::same(2), fg);
            ui.painter().circle_filled(pos2(track.left() + track.width() * vol, cy), 6.0, fg);
            if let Some(pos) = vresp.interact_pointer_pos() {
                if vresp.dragged() || vresp.clicked() {
                    let v = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
                    self.s.volume = v;
                    self.s.muted = false;
                    if let Some(pl) = self.player() {
                        pl.set_volume(v);
                    }
                }
            }
            rx = vr.left() - 4.0;
            let vi = if self.s.muted || self.s.volume == 0.0 { Ic::Mute } else if self.s.volume < 0.5 { Ic::VolumeLevel(1) } else { Ic::VolumeLevel(2) };
            if rbtn(ui, &mut rx, "c_mute", Gl::I(vi), "Couper le son (M)", bh * 0.84).clicked() {
                self.toggle_mute();
            }
        }
        // barre de progression
        let seek = Rect::from_min_max(pos2(x, cy - 12.0), pos2(rx - 8.0, cy + 12.0));
        if live {
            if seek.width() >= 80.0 {
                self.draw_live_bar(ui, seek, overlay, behind.unwrap_or(0.0));
            }
            return;
        }
        if seek.width() < 40.0 || d <= 0.0 {
            return;
        }
        let sr = ui.interact(seek, Id::new("c_seek"), Sense::click_and_drag());
        let track = Rect::from_min_max(pos2(seek.left(), cy - 2.5), pos2(seek.right(), cy + 2.5));
        let tx = |v: f64| track.left() + track.width() * (v / d).clamp(0.0, 1.0) as f32;
        let hot = sr.hovered() || sr.dragged();
        let track = if hot { track.expand2(vec2(0.0, 1.5)) } else { track };
        ui.painter().rect_filled(track, CornerRadius::same(3), if overlay { Color32::from_white_alpha(60) } else { p.border });
        ui.painter().rect_filled(Rect::from_min_max(pos2(tx(t), track.top()), pos2(tx(t + buf), track.bottom())), CornerRadius::same(3), if overlay { Color32::from_white_alpha(110) } else { p.muted.gamma_multiply(0.6) });
        for s in &sponsors {
            ui.painter().rect_filled(Rect::from_min_max(pos2(tx(s.start), track.top()), pos2(tx(s.end), track.bottom())), CornerRadius::ZERO, SB.gamma_multiply(0.85));
        }
        // « les plus revus » : courbe au-dessus de la barre, au survol
        if hot {
            let hm = self.watch.as_ref().and_then(|w| w.info.as_ref()).and_then(|i| i["heatmap"].as_array().cloned()).unwrap_or_default();
            if hm.len() > 2 {
                let base = track.top() - 2.0;
                let hgt = 16.0;
                let mut pts = vec![pos2(track.left(), base)];
                for h in &hm {
                    let (a, b, v) = (h["start"].as_f64().unwrap_or(0.0), h["end"].as_f64().unwrap_or(0.0), h["value"].as_f64().unwrap_or(0.0) as f32);
                    pts.push(pos2(tx((a + b) / 2.0), base - v * hgt));
                }
                pts.push(pos2(track.right(), base));
                let col = if overlay { Color32::from_white_alpha(70) } else { p.muted.gamma_multiply(0.5) };
                // remplissage en bandes verticales (forme non convexe)
                for w2 in pts.windows(2) {
                    let (a, b) = (w2[0], w2[1]);
                    let mut m = egui::Mesh::default();
                    m.colored_vertex(pos2(a.x, base), col);
                    m.colored_vertex(a, col);
                    m.colored_vertex(b, col);
                    m.colored_vertex(pos2(b.x, base), col);
                    m.add_triangle(0, 1, 2);
                    m.add_triangle(0, 2, 3);
                    ui.painter().add(m);
                }
                if overlay {
                    self.video_cuts.push(to_px(Rect::from_min_max(pos2(track.left(), base - hgt - 1.0), pos2(track.right(), base)), ui.ctx().pixels_per_point()));
                }
            }
        }
        ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(tx(t), track.bottom())), CornerRadius::same(3), RED);
        // Y2 : moments forts (sans chapitres) : petits losanges au-dessus de la barre
        if chapters.is_empty() {
            if let Some(w) = self.watch.as_ref() {
                for m in crate::app::highlights(w) {
                    let c = pos2(tx(m), track.top() - 5.0);
                    let s = 3.5;
                    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(0.0, -s), c + vec2(s, 0.0), c + vec2(0.0, s), c + vec2(-s, 0.0)], if overlay { Color32::WHITE } else { p.accent }, Stroke::NONE));
                }
            }
        }
        for c in chapters.iter().skip(1) {
            ui.painter().rect_filled(Rect::from_center_size(pos2(tx(*c), track.center().y), vec2(2.0, track.height() + 1.0)), CornerRadius::ZERO, bg);
        }
        let mut knob = tx(t);
        if let Some(pos) = sr.hover_pos().or(sr.interact_pointer_pos()) {
            let at = (((pos.x - track.left()) / track.width()) as f64 * d).clamp(0.0, d);
            if sr.dragged() {
                knob = pos.x.clamp(track.left(), track.right());
            }
            let label = match chapter_at(self.watch.as_ref(), at) {
                Some(c) => format!("{}  ·  {c}", fmt_time(at)),
                None => fmt_time(at),
            };
            let g = galley(ui, &label, FontId::proportional(12.0), Color32::WHITE, 300.0, 1);
            let tip = Rect::from_center_size(pos2(pos.x.clamp(seek.left() + g.size().x / 2.0, seek.right() - g.size().x / 2.0), seek.top() - 14.0), g.size() + vec2(10.0, 6.0));
            // vignette de l'instant survolé (planches d'aperçu de YouTube)
            if let Some((url, uv)) = self.storyboard_at(at) {
                let (iw, ih) = (192.0, 108.0);
                let cx = pos.x.clamp(seek.left() + iw / 2.0 + 4.0, seek.right() - iw / 2.0 - 4.0);
                let img = Rect::from_min_size(pos2(cx - iw / 2.0, tip.top() - ih - 6.0), vec2(iw, ih));
                if let Some((t, _)) = self.thumbs.get(&url, 0) {
                    let sz = t.size_vec2();
                    let uv = Rect::from_min_max(pos2(uv.min.x / sz.x, uv.min.y / sz.y), pos2(uv.max.x / sz.x, uv.max.y / sz.y));
                    let id = t.id();
                    ui.painter().rect_filled(img.expand(2.0), CornerRadius::same(6), Color32::from_black_alpha(230));
                    ui.painter().image(id, img, uv, Color32::WHITE);
                    self.video_cuts.push(to_px(img.expand(3.0), ui.ctx().pixels_per_point()));
                }
            }
            ui.painter().rect_filled(tip, CornerRadius::same(5), Color32::from_black_alpha(220));
            ui.painter().galley(tip.min + vec2(5.0, 3.0), g, Color32::WHITE);
            self.video_cuts.push(to_px(tip.expand(2.0), ui.ctx().pixels_per_point()));
            if sr.drag_stopped() || sr.clicked() {
                self.seek_to(at);
            }
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        ui.painter().circle_filled(pos2(knob, track.center().y), if hot { 7.0 } else { 5.0 }, RED);
    }

    fn draw_menu(&mut self, ui: &mut Ui) {
        let Some((kind, anchor)) = self.menu else { return };
        let p = self.pal;
        let ctx = ui.ctx().clone();
        let Some(w) = self.watch.as_ref() else {
            self.menu = None;
            return;
        };
        let heights = w.heights.clone();
        let forced = w.forced_height;
        let langs = w.langs.clone();
        let cur_lang = w.audio_fmt.as_ref().map(|a| a.lang.clone()).unwrap_or_default();
        let audio_only = w.audio_only;
        let subs = w.info.as_ref().and_then(|i| i["subs"].as_array().cloned()).unwrap_or_default();
        let sub_sel = w.sub_sel;
        let is_live = w.info.as_ref().is_some_and(|i| i["is_live"].as_bool() == Some(true));
        let qlabel = self.quality_label();
        const SPEEDS: [f64; 11] = [0.25, 0.5, 0.75, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
        let sleep_opts: [(&str, Option<crate::app::Sleep>); 7] = [
            ("Désactivée", None),
            ("15 minutes", Some(crate::app::Sleep::At(Instant::now() + Duration::from_secs(15 * 60)))),
            ("30 minutes", Some(crate::app::Sleep::At(Instant::now() + Duration::from_secs(30 * 60)))),
            ("45 minutes", Some(crate::app::Sleep::At(Instant::now() + Duration::from_secs(45 * 60)))),
            ("1 heure", Some(crate::app::Sleep::At(Instant::now() + Duration::from_secs(3600)))),
            ("2 heures", Some(crate::app::Sleep::At(Instant::now() + Duration::from_secs(7200)))),
            ("Fin de la vidéo", Some(crate::app::Sleep::EndOfVideo)),
        ];
        let sub_name = |s: &serde_json::Value| -> String {
            let n = s["name"].as_str().unwrap_or("?").to_string();
            if s["translated"].as_bool() == Some(true) {
                format!("{n} (traduction auto)")
            } else if s["auto"].as_bool() == Some(true) {
                format!("{n} (auto)")
            } else {
                n
            }
        };
        let sleep_label = match self.sleep {
            None => "Désactivée".to_string(),
            Some(crate::app::Sleep::EndOfVideo) => "Fin de la vidéo".to_string(),
            Some(crate::app::Sleep::At(t)) => format!("{} min", (t.saturating_duration_since(Instant::now()).as_secs() + 59) / 60),
        };
        let n = match kind {
            Menu::Gear => 6 + usize::from(langs.len() > 1) + usize::from(!is_live) * 2,
            Menu::Quality => heights.len() + 3,
            Menu::Speed => SPEEDS.len() + 1,
            Menu::Audio => langs.len().min(13) + 1,
            Menu::Subs => subs.len().min(13) + 2,
            Menu::Sleep => sleep_opts.len() + 1,
        };
        let h = n as f32 * 30.0 + 18.0;
        let pos = pos2((anchor.center().x - 150.0).max(4.0), anchor.top() - h - 8.0);
        let mut close = false;
        let mut page = None;
        let area = egui::Area::new(Id::new("player_menu")).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
            Frame::new().fill(p.surface).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.set_width(290.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let back = |ui: &mut Ui, title: &str| -> bool {
                    let r = menu_row(ui, &p, &format!("‹  {title}"), "", false);
                    r.clicked()
                };
                match kind {
                    Menu::Gear => {
                        if menu_row(ui, &p, "Qualité", &qlabel, true).clicked() {
                            page = Some(Menu::Quality);
                        }
                        if !is_live && menu_row(ui, &p, "Vitesse", &format!("×{}", self.s.speed), true).clicked() {
                            page = Some(Menu::Speed);
                        }
                        if langs.len() > 1 {
                            let l = langs.iter().find(|l| l.code == cur_lang).map(|l| if l.original { format!("{} (VO)", l.label) } else { l.label.clone() }).unwrap_or_default();
                            if menu_row(ui, &p, "Langue de l'audio", &l, true).clicked() {
                                page = Some(Menu::Audio);
                            }
                        }
                        let sl = sub_sel.and_then(|i| subs.get(i)).map(sub_name).unwrap_or_else(|| if subs.is_empty() { "Aucun".into() } else { "Désactivés".into() });
                        if menu_row(ui, &p, "Sous-titres", &sl, true).clicked() {
                            page = Some(Menu::Subs);
                        }
                        if menu_row(ui, &p, "Minuterie", &sleep_label, true).clicked() {
                            page = Some(Menu::Sleep);
                        }
                        if menu_toggle(ui, &p, "Audio seul", audio_only).clicked() {
                            self.set_audio_only(!audio_only);
                        }
                        if !is_live && menu_toggle(ui, &p, "Lecture en boucle", self.loop_on).clicked() {
                            self.loop_on = !self.loop_on;
                        }
                        if menu_toggle(ui, &p, "Mode cinéma (plein écran)", self.s.cinema).clicked() {
                            self.s.cinema = !self.s.cinema;
                        }
                        if menu_row(ui, &p, "Capture d'image", "S", false).clicked() {
                            close = true;
                            self.screenshot();
                        }
                    }
                    Menu::Quality => {
                        if back(ui, "Qualité") {
                            page = Some(Menu::Gear);
                        }
                        if menu_row(ui, &p, &format!("Auto ({})", self.s.quality.label()), if forced.is_none() && !audio_only { "✓" } else { "" }, false).clicked() {
                            self.set_quality(None);
                            close = true;
                        }
                        for hh in heights {
                            let tag = if hh >= 2160 { "4K" } else if hh >= 1440 { "QHD" } else if hh >= 720 { "HD" } else { "" };
                            if menu_row(ui, &p, &format!("{hh}p  {tag}"), if forced == Some(hh) && !audio_only { "✓" } else { "" }, false).clicked() {
                                self.set_quality(Some(hh));
                                close = true;
                            }
                        }
                        if menu_row(ui, &p, "Audio seul", if audio_only { "✓" } else { "" }, false).clicked() {
                            self.set_audio_only(true);
                            close = true;
                        }
                    }
                    Menu::Speed => {
                        if back(ui, "Vitesse (mémorisée pour la chaîne)") {
                            page = Some(Menu::Gear);
                        }
                        for r in SPEEDS {
                            let lbl = if r == 1.0 { "Normale".to_string() } else { format!("×{r}") };
                            if menu_row(ui, &p, &lbl, if self.s.speed == r { "✓" } else { "" }, false).clicked() {
                                self.set_speed(r);
                                close = true;
                            }
                        }
                    }
                    Menu::Audio => {
                        if back(ui, "Langue de l'audio") {
                            page = Some(Menu::Gear);
                        }
                        egui::ScrollArea::vertical().max_height(13.0 * 30.0).show(ui, |ui| {
                            for l in &langs {
                                let t = if l.original { format!("{} (originale)", l.label) } else { l.label.clone() };
                                if menu_row(ui, &p, &t, if cur_lang == l.code { "✓" } else { "" }, false).clicked() {
                                    self.set_lang(&l.code);
                                    close = true;
                                }
                            }
                        });
                    }
                    Menu::Subs => {
                        if back(ui, "Sous-titres") {
                            page = Some(Menu::Gear);
                        }
                        if menu_row(ui, &p, "Désactivés", if sub_sel.is_none() { "✓" } else { "" }, false).clicked() {
                            self.load_subs(None, true);
                            close = true;
                        }
                        egui::ScrollArea::vertical().max_height(13.0 * 30.0).show(ui, |ui| {
                            for (i, s) in subs.iter().enumerate() {
                                if menu_row(ui, &p, &sub_name(s), if sub_sel == Some(i) { "✓" } else { "" }, false).clicked() {
                                    self.load_subs(Some(i), true);
                                    close = true;
                                }
                            }
                        });
                    }
                    Menu::Sleep => {
                        if back(ui, "Minuterie de mise en pause") {
                            page = Some(Menu::Gear);
                        }
                        for (lbl, v) in sleep_opts {
                            let cur = match (self.sleep, v) {
                                (None, None) => true,
                                (Some(crate::app::Sleep::EndOfVideo), Some(crate::app::Sleep::EndOfVideo)) => true,
                                _ => false,
                            };
                            if menu_row(ui, &p, lbl, if cur { "✓" } else { "" }, false).clicked() {
                                self.set_sleep(v);
                                close = true;
                            }
                        }
                    }
                }
            });
        });
        // le menu peut recouvrir la vidéo : on le découpe de la sous-fenêtre
        self.video_cuts.push(to_px(area.response.rect.expand(2.0), ctx.pixels_per_point()));
        if let Some(pg) = page {
            self.menu = Some((pg, anchor));
        } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (ctx.input(|i| i.pointer.any_click()) && !area.response.hovered() && !anchor.contains(ctx.pointer_interact_pos().unwrap_or_default())) {
            self.menu = None;
        }
    }

    /// Direct : barre de la fenêtre de retour arrière (≈ 1 h) ; à droite, « EN DIRECT » pour y revenir.
    fn draw_live_bar(&mut self, ui: &mut Ui, seek: Rect, overlay: bool, behind: f64) {
        let p = self.pal;
        let win = crate::live::window().max(60.0);
        let cy = seek.center().y;
        let at_live = behind < 15.0;
        // pastille « EN DIRECT » (cliquable quand on est en retard)
        let pill = Rect::from_min_max(pos2(seek.right() - 92.0, cy - 11.0), pos2(seek.right(), cy + 11.0));
        let pr = ui.interact(pill, Id::new("c_golive"), Sense::click());
        ui.painter().rect_filled(pill, CornerRadius::same(11), if at_live { RED } else if pr.hovered() { p.hover_bg } else { Color32::TRANSPARENT });
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, "● EN DIRECT", FontId::proportional(11.5), if at_live { Color32::WHITE } else if overlay { Color32::WHITE } else { p.text });
        if pr.on_hover_text("Revenir au direct").clicked() && !at_live {
            self.live_seek(0.0);
        }
        let bar = Rect::from_min_max(seek.min, pos2(pill.left() - 10.0, seek.max.y));
        let sr = ui.interact(bar, Id::new("c_liveseek"), Sense::click_and_drag());
        let track = Rect::from_min_max(pos2(bar.left(), cy - 2.5), pos2(bar.right(), cy + 2.5));
        let x_of = |b: f64| track.right() - track.width() * (b / win).clamp(0.0, 1.0) as f32;
        ui.painter().rect_filled(track, CornerRadius::same(3), if overlay { Color32::from_white_alpha(60) } else { p.border });
        ui.painter().rect_filled(Rect::from_min_max(track.min, pos2(x_of(behind), track.bottom())), CornerRadius::same(3), RED);
        ui.painter().circle_filled(pos2(x_of(behind), cy), if sr.hovered() { 7.0 } else { 5.5 }, RED);
        if let Some(pos) = sr.hover_pos().or(sr.interact_pointer_pos()) {
            let b = ((track.right() - pos.x) / track.width()).clamp(0.0, 1.0) as f64 * win;
            let label = if b < 15.0 { "Direct".to_string() } else { format!("−{}", fmt_time(b)) };
            let g = galley(ui, &label, FontId::proportional(12.0), Color32::WHITE, 200.0, 1);
            let tip = Rect::from_center_size(pos2(pos.x.clamp(bar.left() + 30.0, bar.right() - 30.0), seek.top() - 14.0), g.size() + vec2(10.0, 6.0));
            ui.painter().rect_filled(tip, CornerRadius::same(5), Color32::from_black_alpha(220));
            ui.painter().galley(tip.min + vec2(5.0, 3.0), g, Color32::WHITE);
            self.video_cuts.push(to_px(tip.expand(2.0), ui.ctx().pixels_per_point()));
            if sr.clicked() || sr.drag_stopped() {
                self.live_seek(if b < 15.0 { 0.0 } else { b });
            }
        }
        ui.ctx().request_repaint_after(Duration::from_millis(1000));
    }

    /// Sous-titres sur l'image : bandeau sombre en bas de la vidéo (découpé de la fenêtre vidéo).
    fn draw_subs(&mut self, ui: &mut Ui, video: Rect, lift: f32) {
        let Some(w) = self.watch.as_ref() else { return };
        if w.sub_sel.is_none() || w.cues.is_empty() {
            return;
        }
        let t = w.player.as_ref().map(|p| p.time()).unwrap_or(0.0);
        let text = w.cues.iter().rev().find(|c| c.0 <= t && t < c.1).map(|c| c.2.clone());
        let Some(text) = text else { return };
        let size = (video.height() * 0.042).clamp(14.0, 38.0);
        let g = galley(ui, &text, FontId::proportional(size), Color32::WHITE, video.width() * 0.8, 3);
        let b = Rect::from_center_size(pos2(video.center().x, video.bottom() - lift - video.height() * 0.06 - g.size().y / 2.0), g.size() + vec2(size * 0.8, size * 0.35));
        ui.painter().rect_filled(b, CornerRadius::same(4), Color32::from_black_alpha(200));
        ui.painter().galley(pos2(b.center().x - g.size().x / 2.0, b.min.y + size * 0.175), g, Color32::WHITE);
        self.video_cuts.push(to_px(b.expand(1.0), ui.ctx().pixels_per_point()));
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }

    // ---------- Plein écran / mini-lecteur ----------
    fn draw_overlay_mode(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let ppp = ui.ctx().pixels_per_point();
        let mini = self.mode == Mode::Mini;
        let (moved, inside) = ui.ctx().input(|i| (i.pointer.delta() != egui::Vec2::ZERO || i.pointer.any_down(), i.pointer.has_pointer()));
        if moved {
            self.overlay_until = Instant::now() + Duration::from_millis(if mini { 1500 } else { 2500 });
        }
        let show = (inside && (mini || Instant::now() < self.overlay_until)) || self.menu.is_some() || self.watch.as_ref().is_some_and(|w| w.player.as_ref().is_none_or(|p| !p.wants_play()));
        if !show && !mini {
            ui.ctx().set_cursor_icon(CursorIcon::None);
        }
        if show {
            ui.ctx().request_repaint_after(self.overlay_until.saturating_duration_since(Instant::now()).max(Duration::from_millis(250)).min(Duration::from_millis(500)));
        }
        // mode cinéma (plein écran / demi-écran) : la vidéo occupe sa place exacte et les bandes
        // noires reprennent, en dégradé, les couleurs de ses bords
        let mut vrect = full;
        if !mini && self.s.cinema {
            let (vw, vh) = self.watch.as_ref().and_then(|w| w.player.as_ref()).filter(|p| p.ready).map(|p| p.size).filter(|s| s.0 > 0).unwrap_or((0, 0));
            if vw > 0 {
                let a = vw as f32 / vh as f32;
                let fit = if full.width() / full.height() > a { vec2(full.height() * a, full.height()) } else { vec2(full.width(), full.width() / a) };
                let r = Rect::from_center_size(full.center(), fit);
                if full.width() - r.width() > 60.0 || full.height() - r.height() > 60.0 {
                    vrect = r;
                    self.draw_ambient(ui, full, r);
                }
            }
        }
        if vrect == full {
            self.ambient = None;
        }
        self.video_px = Some(to_px(vrect, ppp));
        self.draw_still(ui, vrect);
        // fond : clic = pause (plein écran) ou déplacement (mini), double-clic = plein écran
        let r = ui.interact(full, Id::new("ov_bg"), Sense::click_and_drag());
        if r.double_clicked() {
            let x = r.interact_pointer_pos().map(|p| (p.x - full.left()) / full.width()).unwrap_or(0.5);
            if mini {
                self.ops.push(WinOp::Mini(false));
                self.ops.push(WinOp::Fullscreen(true));
            } else if x < 0.25 || x > 0.75 {
                // double-clic sur un côté : −10 s / +10 s (le premier clic avait mis en pause : on annule)
                self.toggle();
                self.seek_by(if x < 0.25 { -10.0 } else { 10.0 });
                self.toast(if x < 0.25 { "−10 s" } else { "+10 s" });
            } else {
                self.toggle_full();
            }
        } else if r.drag_started() && mini {
            self.ops.push(WinOp::Drag);
        } else if r.clicked() && !mini {
            self.toggle();
        }
        if mini {
            // poignées de redimensionnement sur les bords
            use winit::window::ResizeDirection as D;
            let e = 7.0;
            let zones = [
                (Rect::from_min_max(full.min, pos2(full.left() + e, full.top() + e)), D::NorthWest, CursorIcon::ResizeNwSe),
                (Rect::from_min_max(pos2(full.right() - e, full.top()), pos2(full.right(), full.top() + e)), D::NorthEast, CursorIcon::ResizeNeSw),
                (Rect::from_min_max(pos2(full.left(), full.bottom() - e), pos2(full.left() + e, full.bottom())), D::SouthWest, CursorIcon::ResizeNeSw),
                (Rect::from_min_max(pos2(full.right() - e, full.bottom() - e), full.max), D::SouthEast, CursorIcon::ResizeNwSe),
                (Rect::from_min_max(full.min, pos2(full.left() + e, full.bottom())), D::West, CursorIcon::ResizeHorizontal),
                (Rect::from_min_max(pos2(full.right() - e, full.top()), full.max), D::East, CursorIcon::ResizeHorizontal),
                (Rect::from_min_max(full.min, pos2(full.right(), full.top() + e)), D::North, CursorIcon::ResizeVertical),
                (Rect::from_min_max(pos2(full.left(), full.bottom() - e), full.max), D::South, CursorIcon::ResizeVertical),
            ];
            for (i, (z, dir, cur)) in zones.into_iter().enumerate() {
                let zr = ui.interact(z, Id::new(("rz", i)), Sense::drag());
                if zr.hovered() {
                    ui.ctx().set_cursor_icon(cur);
                }
                if zr.drag_started() {
                    self.ops.push(WinOp::Resize(dir));
                }
            }
            // molette : volume ; Ctrl+molette : opacité
            // (egui transforme Ctrl+molette en zoom : on lit les deux)
            let (scroll, ctrl, zoom) = ui.ctx().input(|i| (i.smooth_scroll_delta.y, i.modifiers.ctrl, i.zoom_delta()));
            if inside && zoom != 1.0 {
                self.s.mini_opacity = (self.s.mini_opacity + if zoom > 1.0 { 0.05 } else { -0.05 }).clamp(0.2, 1.0);
                self.ops.push(WinOp::Opacity(self.s.mini_opacity));
            } else if inside && scroll.abs() > 0.5 {
                if ctrl {
                    self.s.mini_opacity = (self.s.mini_opacity + scroll.signum() * 0.05).clamp(0.2, 1.0);
                    self.ops.push(WinOp::Opacity(self.s.mini_opacity));
                } else {
                    self.change_volume(scroll.signum() as f64 * 0.05);
                }
            }
        }
        self.draw_video_state(ui, full);
        let lift = if show { if mini { 38.0 } else { 50.0 } } else { 0.0 };
        self.draw_subs(ui, vrect, lift);
        if !show {
            return;
        }
        let bh = if mini { 38.0 } else { 50.0 };
        let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - bh), full.max);
        self.video_cuts.push(to_px(bar, ppp));
        self.draw_controls(ui, bar.shrink2(vec2(if mini { 2.0 } else { 14.0 }, if mini { 0.0 } else { 3.0 })), true);
        if mini {
            // barre du haut : opacité, retour, fermer
            let top = Rect::from_min_max(full.min, pos2(full.right(), full.top() + 30.0));
            self.video_cuts.push(to_px(top, ppp));
            ui.painter().rect_filled(top, CornerRadius::ZERO, Color32::from_black_alpha(170));
            let title = self.watch.as_ref().map(|w| w.item.title.clone()).unwrap_or_default();
            let g = galley(ui, &title, FontId::proportional(12.0), Color32::WHITE, top.width() - 200.0, 1);
            ui.painter().galley(pos2(top.left() + 8.0, top.center().y - g.size().y / 2.0), g, Color32::WHITE);
            let close = Rect::from_center_size(pos2(top.right() - 16.0, top.center().y), vec2(28.0, 26.0));
            if icon_btn(ui, close, "m_close", Gl::I(Ic::Close), "Arrêter la vidéo", Color32::WHITE, Color32::from_white_alpha(40)).clicked() {
                self.stop();
            }
            let back = Rect::from_center_size(pos2(top.right() - 46.0, top.center().y), vec2(28.0, 26.0));
            if icon_btn(ui, back, "m_back", Gl::I(Ic::ExitMini), "Revenir à la fenêtre (I)", Color32::WHITE, Color32::from_white_alpha(40)).clicked() {
                self.toggle_mini();
            }
            let op = Rect::from_min_max(pos2(top.right() - 160.0, top.top() + 8.0), pos2(top.right() - 74.0, top.bottom() - 8.0));
            let mut o = self.s.mini_opacity;
            let resp = ui.put(op, egui::Slider::new(&mut o, 0.2..=1.0).show_value(false)).on_hover_text("Opacité (Ctrl+molette)");
            if resp.changed() {
                self.s.mini_opacity = o;
                self.ops.push(WinOp::Opacity(o));
            }
        }
        self.draw_menu(ui);
    }

    // ---------- Réglages ----------
    fn draw_settings(&mut self, ui: &mut Ui) {
        let p = self.pal;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(24, 18)).show(ui, |ui| {
                ui.set_max_width(720.0);
                ui.label(RichText::new("Réglages").size(20.0).strong());
                let card = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
                    ui.add_space(10.0);
                    Frame::new().fill(p.side).stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(12)).inner_margin(Margin::same(16)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        f(ui)
                    });
                };
                card(ui, &mut |ui| {
                    ui.label(RichText::new("Compte").strong());
                    let ok = self.login == Login::Ok;
                    ui.label(RichText::new(if ok { "Connecté via les cookies du navigateur (stockés dans %LOCALAPPDATA%\\Tube\\cookies.txt, YouTube/Google uniquement)." } else { "Non connecté." }).color(p.muted));
                    ui.horizontal(|ui| {
                        for (name, path) in crate::cookies::browser_profiles() {
                            if ui.button(format!("Reconnecter avec {name}")).clicked() {
                                self.import_cookies(path, true);
                            }
                        }
                        if ok && ui.button("Se déconnecter").clicked() {
                            let _ = std::fs::remove_file(crate::store::cookies_path());
                            *self.api.jar.lock().unwrap() = crate::cookies::Jar::default();
                            self.worker.restart();
                            self.login = Login::Need;
                            self.feeds.clear();
                        }
                    });
                    ui.checkbox(&mut self.s.report_history, "Remonter l'historique de visionnage au compte (pour que les recommandations continuent d'apprendre)");
                });
                card(ui, &mut |ui| {
                    ui.label(RichText::new("Lecture").strong());
                    ui.horizontal(|ui| {
                        ui.label("Qualité par défaut :");
                        for q in Quality::ALL {
                            if ui.selectable_label(self.s.quality == q, q.label()).clicked() {
                                self.s.quality = q;
                            }
                        }
                    });
                    ui.label(RichText::new("VP9 est préféré (le mieux testé) ; l'AV1 ou le H.264 ne sont pris que s'ils offrent une définition supérieure. Décodage par la carte graphique.").size(12.0).color(p.muted));
                    ui.checkbox(&mut self.s.autoplay_next, "Lancer la suggestion suivante à la fin d'une vidéo");
                    ui.checkbox(&mut self.s.hide_shorts, "Masquer les Shorts");
                    ui.checkbox(&mut self.s.hover_preview, "Aperçu vidéo au survol des vignettes");
                    ui.checkbox(&mut self.s.hide_watched, "Masquer les vidéos déjà regardées (accueil, abonnements, recherche)");
                    ui.checkbox(&mut self.s.cinema, "Mode cinéma : en plein écran, les bandes noires prennent les couleurs de l'image (touche C)");
                    ui.horizontal(|ui| {
                        ui.label("Taille des vignettes");
                        ui.add(egui::Slider::new(&mut self.s.card_w, 220.0..=900.0).show_value(false));
                        ui.label(RichText::new("(ou Ctrl+molette sur la grille)").size(12.0).color(p.muted));
                    });
                });
                card(ui, &mut |ui| {
                    ui.label(RichText::new("SponsorBlock").strong());
                    ui.checkbox(&mut self.s.sponsorblock, "Passer automatiquement les segments signalés par la communauté");
                    ui.horizontal_wrapped(|ui| {
                        for c in crate::sponsor::CATEGORIES {
                            let mut on = self.s.sb_categories.iter().any(|x| x == c);
                            if ui.checkbox(&mut on, crate::sponsor::label(c)).changed() {
                                if on {
                                    self.s.sb_categories.push(c.to_string());
                                } else {
                                    self.s.sb_categories.retain(|x| x != c);
                                }
                            }
                        }
                    });
                });
                card(ui, &mut |ui| {
                    ui.label(RichText::new("Mini-lecteur et raccourcis globaux").strong());
                    ui.horizontal(|ui| {
                        ui.label("Opacité du mini-lecteur :");
                        ui.add(egui::Slider::new(&mut self.s.mini_opacity, 0.2..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    });
                    ui.add_space(4.0);
                    for (_, _, label) in crate::hotkeys::LIST {
                        let failed = self.hk_failed.iter().any(|f| f == label);
                        ui.label(RichText::new(if failed { format!("✗ {label} (crochet clavier refusé)") } else { format!("✓ {label}") }).color(if failed { p.del } else { p.text }));
                    }
                });
                card(ui, &mut |ui| {
                    ui.label(RichText::new("Outils").strong());
                    ui.label(RichText::new(format!("yt-dlp {} · mis à jour automatiquement une fois par jour", self.ytdlp_version.clone().unwrap_or_else(|| "…".into()))).color(p.muted));
                    ui.horizontal(|ui| {
                        if ui.button("Mettre à jour yt-dlp maintenant").clicked() {
                            self.check_update();
                            self.toast("Mise à jour de yt-dlp…");
                        }
                        if ui.button("Ouvrir le dossier de données").clicked() {
                            let _ = std::process::Command::new("explorer.exe").arg(crate::store::dir()).spawn();
                        }
                        if ui.button("Oublier les reprises").clicked() {
                            self.s.positions.clear();
                        }
                    });
                });
            });
        });
    }

    // ---------- Toast / aide ----------
    fn draw_toast(&mut self, ui: &mut Ui) {
        let Some((t, at)) = &self.toast else { return };
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let a = (1.0 - (at.elapsed().as_secs_f32() - 2.7).max(0.0) / 0.5).clamp(0.0, 1.0);
        let g = galley(ui, t, FontId::proportional(13.5), Color32::WHITE, 600.0, 2);
        let size = g.size() + vec2(28.0, 16.0);
        let y = if self.mode == Mode::Normal { screen.bottom() - 70.0 } else { screen.top() + if self.mode == Mode::Mini { 40.0 } else { 70.0 } };
        let rect = Rect::from_center_size(pos2(screen.center().x, y), size);
        egui::Area::new(Id::new("toast")).order(egui::Order::Tooltip).fixed_pos(rect.min).interactable(false).show(&ctx, |ui| {
            ui.painter().rect_filled(Rect::from_min_size(rect.min, size), CornerRadius::same(10), Color32::from_black_alpha((215.0 * a) as u8));
            ui.painter().galley(rect.min + vec2(14.0, 8.0), g, Color32::WHITE.gamma_multiply(a));
        });
        self.video_cuts.push(to_px(rect, ctx.pixels_per_point()));
        if at.elapsed() > Duration::from_millis(2600) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn draw_help(&mut self, ui: &mut Ui) {
        if !self.help {
            return;
        }
        let ctx = ui.ctx().clone();
        let mut open = true;
        egui::Window::new("Raccourcis").open(&mut open).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(&ctx, |ui| {
            egui::Grid::new("keys").num_columns(2).spacing(vec2(24.0, 4.0)).show(ui, |ui| {
                let mut rows: Vec<(&str, &str)> = vec![
                    ("Espace / K", "Lecture / pause"),
                    ("J / L", "−10 s / +10 s"),
                    ("← / →", "−5 s / +5 s"),
                    ("↑ / ↓", "Volume"),
                    ("M", "Couper le son"),
                    ("F / double-clic", "Plein écran"),
                    ("I", "Mini-lecteur"),
                    ("H", "Demi-écran (toute la hauteur, moitié de la largeur)"),
                    ("C", "Mode cinéma : bandes noires aux couleurs de l'image"),
                    (", / .", "Image précédente / suivante (en pause)"),
                    ("Ctrl+C", "Copier le lien de la vidéo à l'instant courant"),
                    ("V", "Sous-titres oui / non"),
                    ("S", "Capture de l'image (Images\\Tube)"),
                    ("Ctrl+K", "Palette de commandes (tout Tube au clavier)"),
                    ("Ctrl+Maj+T", "Rouvrir le dernier onglet fermé"),
                    ("Ctrl+← / Ctrl+→", "Chapitre précédent / suivant (sans chapitres : moments forts)"),
                    ("Double-clic à gauche / à droite", "−10 s / +10 s (plein écran)"),
                    ("Clic sur la vidéo", "Lecture / pause"),
                    ("Ctrl+molette", "Taille des vignettes (accueil) · opacité (mini-lecteur)"),
                    ("0 … 9", "Aller à 0 %, 10 % … 90 %"),
                    ("Début", "Recommencer"),
                    ("Maj+, / Maj+.", "Vitesse − / +"),
                    ("Maj+N", "Vidéo suivante de la file"),
                    ("Ctrl+Z", "Annuler le dernier saut SponsorBlock"),
                    ("Ctrl+1 … 5", "Accueil, Abonnements, Plus tard, Playlists, Historique"),
                    ("Ctrl+F", "Rechercher (on peut coller un lien YouTube)"),
                    ("Ctrl+T / Ctrl+W", "Nouvel onglet / fermer l'onglet"),
                    ("Clic sur une chaîne", "Ouvrir sa page (clic molette : nouvel onglet)"),
                    ("Ctrl+Tab", "Onglet suivant (Maj : précédent)"),
                    ("Clic molette", "Ouvrir une vignette dans un nouvel onglet"),
                    ("Alt+← / souris arrière", "Retour"),
                    ("F5", "Actualiser"),
                    ("F2", "Statistiques de lecture"),
                ];
                for (_, _, l) in crate::hotkeys::LIST {
                    let (k, v) = l.split_once(" : ").unwrap_or((l, ""));
                    rows.push((k, v));
                }
                for (k, v) in rows {
                    ui.label(RichText::new(k).monospace().strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.help = false;
        }
    }

    // ---------- Clavier ----------
    pub fn handle_keys(&mut self, ctx: &egui::Context) {
        use egui::Key;
        let typing = ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input();
        // touches corrigées pour l'AZERTY (socle : touche physique pour les chiffres)
        let mut pressed: Vec<Key> = socle::keys::pressed(ctx).into_iter().map(|(k, _)| k).collect();
        let (mods, back_btn) = ctx.input(|i| (i.modifiers, i.pointer.button_pressed(egui::PointerButton::Extra1)));
        // Ctrl+C arrive comme « Copier » (et non comme une touche)
        if !typing && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) && !pressed.contains(&Key::C) {
            pressed.push(Key::C);
        }
        if back_btn {
            self.go_back();
        }
        let has_video = self.watch.is_some();
        for k in pressed {
            if mods.ctrl {
                match k {
                    Key::F => self.focus_search = true,
                    Key::T if mods.shift => self.reopen_tab(),
                    Key::ArrowLeft if has_video && !typing => self.chapter_jump(-1),
                    Key::ArrowRight if has_video && !typing => self.chapter_jump(1),
                    Key::T => self.new_tab(View::Home, None, true),
                    Key::C if has_video && !typing => self.copy_time_link(),
                    Key::W => self.close_tab(self.tab),
                    Key::Tab => {
                        let n = self.tabs.len();
                        let i = if mods.shift { (self.tab + n - 1) % n } else { (self.tab + 1) % n };
                        self.switch_tab(i);
                    }
                    Key::Num1 => self.go(View::Home),
                    Key::Num2 => self.go(View::Subs),
                    Key::Num3 => self.go(View::Later),
                    Key::Num4 => self.go(View::Playlists),
                    Key::Num5 => self.go(View::History),
                    Key::Z => {
                        if let Some(t) = self.watch.as_mut().and_then(|w| w.undo_skip.take()) {
                            self.seek_to(t);
                            self.toast("Retour avant le segment");
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if mods.alt {
                if k == Key::ArrowLeft {
                    self.go_back();
                }
                continue;
            }
            if k == Key::F5 {
                self.refresh();
            }
            if k == Key::F1 {
                self.help = !self.help;
            }
            if k == Key::F2 {
                self.stats = !self.stats;
            }
            if typing {
                continue;
            }
            let has = self.watch.is_some();
            match k {
                Key::Space | Key::K if has => self.toggle(),
                Key::J if has => self.seek_by(-10.0),
                Key::L if has => self.seek_by(10.0),
                Key::ArrowLeft if has => self.seek_by(-5.0),
                Key::ArrowRight if has => self.seek_by(5.0),
                Key::ArrowUp if has && self.view == View::Watch || self.mode != Mode::Normal => self.change_volume(0.05),
                Key::ArrowDown if has && self.view == View::Watch || self.mode != Mode::Normal => self.change_volume(-0.05),
                Key::M if has => self.toggle_mute(),
                Key::F if has => self.toggle_full(),
                Key::I if has => self.toggle_mini(),
                Key::H if has => self.toggle_half(),
                Key::C if has => {
                    self.s.cinema = !self.s.cinema;
                    self.toast(if self.s.cinema { "Mode cinéma activé (plein écran)" } else { "Mode cinéma désactivé" });
                }
                Key::S if has => self.screenshot(),
                Key::V if has => self.toggle_subs(),
                Key::Comma if has && !mods.shift => self.frame_step(-1.0),
                Key::Period if has && !mods.shift => self.frame_step(1.0),
                Key::Home if has => self.seek_to(0.0),
                Key::Escape => {
                    self.leave_big();
                }
                Key::N if has && mods.shift => {
                    self.next_in_queue();
                }
                Key::Comma if mods.shift && has => {
                    let s = (self.s.speed - 0.25).max(0.25);
                    self.set_speed(s);
                }
                Key::Period if mods.shift && has => {
                    let s = (self.s.speed + 0.25).min(3.0);
                    self.set_speed(s);
                }
                Key::Slash => self.focus_search = true,
                Key::Backspace => self.go_back(),
                _ if has => {
                    let digits = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
                    if let Some(n) = digits.iter().position(|d| *d == k) {
                        let d = self.player().map(|p| p.duration).unwrap_or(0.0);
                        self.seek_to(d * n as f64 / 10.0);
                    }
                }
                _ => {}
            }
        }
    }

    /// Statistiques de lecture (F2), affichées en surimpression dans la barre d'état et la console.
    pub fn stats_line(&self) -> Option<String> {
        let w = self.watch.as_ref()?;
        let pl = w.player.as_ref()?;
        let (r, d) = pl.stats();
        Some(format!(
            "{} · {}×{} · vidéo {} · son {} · tampon {:.0} s · {} images, {} perdues",
            self.quality_label(),
            pl.size.0,
            pl.size.1,
            w.video_fmt.as_ref().map(|f| format!("{} {:.0} kb/s", f.codec, f.tbr)).unwrap_or_default(),
            w.audio_fmt.as_ref().map(|f| format!("{} {:.0} kb/s", f.codec, f.tbr)).unwrap_or_default(),
            pl.buffered_ahead(),
            r,
            d
        ))
    }
}

fn chapter_at(w: Option<&crate::app::Watch>, t: f64) -> Option<String> {
    let ch = w?.info.as_ref()?["chapters"].as_array()?.clone();
    ch.iter().rev().find(|c| c["start"].as_f64().unwrap_or(0.0) <= t).and_then(|c| c["title"].as_str().map(|s| s.to_string()))
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push('\u{202F}');
        }
        o.push(c);
    }
    o
}

/// Identifiant de vidéo dans un lien collé (watch?v=, youtu.be/, shorts/, live/).
pub fn video_id_from(s: &str) -> Option<String> {
    let s = s.trim();
    let id = if let Some(i) = s.find("v=") {
        &s[i + 2..]
    } else if let Some(i) = s.find("youtu.be/") {
        &s[i + 9..]
    } else if let Some(i) = s.find("/shorts/").or(s.find("/live/")) {
        let rest = &s[i + 1..];
        &rest[rest.find('/')? + 1..]
    } else {
        return None;
    };
    let id: String = id.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (id.len() == 11).then_some(id)
}
