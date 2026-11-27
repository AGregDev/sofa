//! Drawing every screen from the state in `ui.rs`. Sizes are in rem (screen height / 67.5),
//! with the values of Sofa 1.x's `renderer/style.css`.

use crate::gfx::{Align, Color, Fit, Rect};
use crate::icons;
use crate::input::PadKind;
use crate::library::{Item, Launch};
use crate::platform;
use crate::ui::*;
use std::path::PathBuf;

const SCRIM: Color = Color(0.04, 0.035, 0.03, 0.62);

/// What the hero shows for the focused tile.
struct Hero {
    hero: Option<PathBuf>,
    capsule: Option<PathBuf>,
    icon: Option<PathBuf>,
    logo: Option<PathBuf>,
    name: String,
    /// Streaming services: their name in their colour stands in for art.
    web: Option<Color>,
    glyph: Option<&'static str>,
    meta: String,
    verb: &'static str,
    options: bool,
}

impl App {
    pub fn draw(&mut self, now: f64) {
        self.dirty = false;
        self.gfx.begin(now);
        self.gfx.clear(BG);
        // The first-run screen covers Home completely once it has faded in.
        let covered = self.layers.iter().any(|l| matches!(l, Layer::Welcome { at, .. } if now - at > 700.0));
        if !covered {
            self.draw_hero(now);
            self.draw_top(now);
            self.draw_shelf(now);
            self.draw_hints();
        }
        self.draw_layers(now);
        self.draw_toast(now);
        self.draw_splash(now);
        if self.gfx.end().is_err() {
            self.draw_errors += 1;
        } else {
            self.draw_errors = 0;
        }
    }

    // ---------- Keycaps ----------

    fn key_label(&self, k: Key) -> &'static str {
        let kind = if self.input.layout() == "nintendo" { 2 } else if self.input.kind == PadKind::Xbox { 1 } else { 0 };
        let t: [&str; 3] = match k {
            Key::Cross => ["", "A", "A"],
            Key::Circle => ["", "B", "B"],
            Key::Square => ["", "X", "Y"],
            Key::Triangle => ["", "Y", "X"],
            Key::Options => ["", "", "+"],
            Key::L1 => ["L1", "LB", "L"],
            Key::R1 => ["R1", "RB", "R"],
            Key::L2 => ["L2", "LT", "ZL"],
            Key::R2 => ["R2", "RT", "ZR"],
        };
        t[kind]
    }

    fn key_width(&mut self, k: Key) -> f32 {
        let r = self.rem();
        let label = self.key_label(k);
        if label.is_empty() { 2.25 * r } else { (self.gfx.measure(label, 0.875 * r, 600).0 + r).max(2.25 * r) }
    }

    /// Draws a keycap with its left edge at x, vertically centred on cy. Returns its width.
    fn keycap(&mut self, k: Key, x: f32, cy: f32) -> f32 {
        let r = self.rem();
        let label = self.key_label(k);
        let h = 2.25 * r;
        let w = self.key_width(k);
        let rect = Rect::new(x, cy - h / 2.0, w, h);
        self.gfx.fill(Rect::new(rect.x, rect.y + 2.0, rect.w, rect.h), DEEP, 6.0);
        self.gfx.fill(rect, RAISED, 6.0);
        self.gfx.stroke(rect.inset(0.5), LINE, 5.5, 1.0);
        if label.is_empty() {
            let name = match k {
                Key::Cross => "cross",
                Key::Circle => "circle",
                Key::Square => "square",
                Key::Triangle => "triangle",
                _ => "options",
            };
            if let Some((n, svg)) = icons::icon(name) {
                self.gfx.icon(n, svg, Rect::new(x + (w - 1.25 * r) / 2.0, cy - 0.625 * r, 1.25 * r, 1.25 * r), T1, false, 2.0);
            }
        } else {
            let th = 0.875 * r * 1.3;
            self.gfx.text(label, x, cy - th / 2.0, w, 0.875 * r, 600, T1, Align::Center, 0, 0.0);
        }
        w
    }

    /// A keycap and its label; returns the x after it.
    fn hint(&mut self, k: Key, label: &str, x: f32, cy: f32, c: Color) -> f32 {
        let r = self.rem();
        let x = x + self.keycap(k, x, cy) + 0.75 * r;
        self.gfx.text(label, x, cy - 0.73 * r, 20.0 * r, 1.125 * r, 400, c, Align::Left, 0, 0.0);
        x + self.gfx.measure(label, 1.125 * r, 400).0 + 2.25 * r
    }

    // ---------- Top bar ----------

    fn draw_top(&mut self, now: f64) {
        let r = self.rem();
        let (w, pad) = (self.gfx.width, 4.0 * r);
        let cy = 3.0 * r;
        let icon = self.icon_png.clone();
        self.gfx.image(&icon, Rect::new(pad, cy - r, 2.0 * r, 2.0 * r), Fit::Contain, 0.0, 1.0);
        self.gfx.text("Sofa", pad + 2.75 * r, cy - 0.8 * r, 10.0 * r, 1.25 * r, 600, T1, Align::Left, 0, 0.0);
        // Tabs, centred, between the L1 and R1 keycaps.
        let tabs = self.tabs();
        let size = 1.375 * r;
        let widths: Vec<f32> = tabs.iter().map(|(_, l)| self.gfx.measure(l, size, 500).0).collect();
        let gap = 2.5 * r;
        let total = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0);
        let kw = self.key_width(Key::L1);
        let x0 = (w - total) / 2.0;
        self.keycap(Key::L1, x0 - 1.5 * r - kw, cy);
        let mut x = x0;
        let dur = 360.0 * self.motion();
        for (i, (k, l)) in tabs.iter().enumerate() {
            let on = *k == self.tab;
            self.gfx.text(l, x, cy - size * 0.65, widths[i] + 4.0, size, 500, if on { T1 } else { T2 }, Align::Left, 0, 0.0);
            if on {
                if self.tab_line.1.to == 0.0 {
                    self.tab_line = (Tween::at(x), Tween::at(widths[i]));
                }
                self.tab_line.0.go(x, now, dur);
                self.tab_line.1.go(widths[i], now, dur);
            }
            x += widths[i] + gap;
        }
        self.keycap(Key::R1, x - gap + 1.5 * r, cy);
        let (lx, lw) = (self.tab_line.0.value(now), self.tab_line.1.value(now));
        self.gfx.fill(Rect::new(lx, cy + 1.05 * r, lw, 2.0), ACCENT, 1.0);
        // Clock and controller battery, right.
        let clock = clock_text();
        let (cw, _) = self.gfx.measure(&clock, size, 400);
        self.gfx.text(&clock, w - pad - cw, cy - size * 0.65, cw + 4.0, size, 400, T2, Align::Left, 0, 0.0);
        let info = self.input.info();
        if let (true, Some(b)) = (info.connected, info.battery) {
            let text = format!("{b}%");
            let (tw, _) = self.gfx.measure(&text, 1.25 * r, 400);
            let low = b <= 20 && !info.charging;
            let mut bx = w - pad - cw - 1.5 * r - tw;
            self.gfx.text(&text, bx, cy - 0.8 * r, tw + 4.0, 1.25 * r, 400, if low { ACCENT } else { T2 }, Align::Left, 0, 0.0);
            if info.charging
                && let Some((n, svg)) = icons::icon("zap")
            {
                bx -= 1.625 * r;
                self.gfx.icon(n, svg, Rect::new(bx, cy - 0.5625 * r, 1.125 * r, 1.125 * r), T2, false, 1.5);
            }
            if let Some((n, path)) = icons::brand("playstation") {
                self.gfx.icon(n, path, Rect::new(bx - 1.75 * r, cy - 0.625 * r, 1.25 * r, 1.25 * r), T3, true, 0.0);
            }
        }
    }

    // ---------- Hero ----------

    fn source_label(&self, it: &Item) -> String {
        match it.source {
            "steam" => "Steam".into(),
            "recomp" => match &it.project {
                Some(p) if *p != it.name => format!("{p} · RecompLauncher"),
                _ => "RecompLauncher".into(),
            },
            "web" if it.id == "web:browser" => platform::chosen_browser(self.store.text("browser")).map_or("Browser".into(), |b| b.name.into()),
            "web" => match &it.launch {
                Launch::Web { tv: true, .. } => "TV mode in your browser".into(),
                _ => format!("Opens full screen in {}", platform::chosen_browser(self.store.text("browser")).map_or("your browser", |b| b.name)),
            },
            "system" => system_desc(it.glyph.unwrap_or("")).into(),
            _ => match it.category.as_str() {
                "games" => "Game",
                "media" => "Media",
                "hidden" => "App",
                _ => "App",
            }
            .into(),
        }
    }

    fn item_hero(&self, i: usize) -> Hero {
        let it = &self.items[i];
        let running = self.item_running(i);
        let source = self.source_label(it);
        let meta = if running {
            format!("Running  ·  {source}")
        } else if it.last_played > 0.0 {
            format!("{source}  ·  {} {}", if it.category == "games" { "Played" } else { "Opened" }, ago(it.last_played))
        } else {
            source
        };
        Hero {
            hero: it.art.hero.clone(),
            capsule: it.art.capsule.clone(),
            icon: it.art.icon.clone(),
            logo: it.art.logo.clone(),
            name: it.name.clone(),
            web: (it.source == "web").then(|| it.color.as_deref().and_then(Color::parse).unwrap_or(T1)),
            glyph: it.glyph,
            meta,
            verb: if running {
                "Resume"
            } else if it.source == "system" {
                "Select"
            } else if it.category == "games" {
                "Play"
            } else {
                "Open"
            },
            options: it.source != "system",
        }
    }

    fn hero_info(&self, id: &str) -> Option<Hero> {
        if let Some(h) = id.strip_prefix("run:").and_then(|h| h.parse::<isize>().ok()) {
            let run = self.run_of(h)?;
            if let Some(i) = run.item.filter(|&i| i < self.items.len()) {
                return Some(self.item_hero(i));
            }
            return Some(Hero {
                hero: None,
                capsule: None,
                icon: run.icon.clone(),
                logo: None,
                name: self.run_name(h),
                web: None,
                glyph: Some("app"),
                meta: "Running".into(),
                verb: "Resume",
                options: true,
            });
        }
        let i = self.items.iter().position(|i| i.id == id)?;
        Some(self.item_hero(i))
    }

    fn draw_hero(&mut self, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height * 0.64);
        let layers: Vec<(String, f64, bool)> =
            [self.hero_prev.clone().map(|(i, t)| (i, t, false)), self.hero.clone().map(|(i, t)| (i, t, true))].into_iter().flatten().collect();
        let current_start = self.hero.as_ref().map_or(0.0, |(_, t)| *t);
        let m = self.motion();
        for (id, start, current) in &layers {
            if id.starts_with("nav:") {
                continue;
            }
            let Some(hero) = self.hero_info(id) else { continue };
            // Incoming art fades in over 520 ms; the outgoing one fades out under it.
            let t = if m > 0.0 { ((now - current_start) / 520.0).clamp(0.0, 1.0) } else { 1.0 };
            let opacity = if *current { ease(t) as f32 } else { 1.0 - ease(t) as f32 };
            if opacity <= 0.01 {
                continue;
            }
            let zoom = 1.03 - 0.03 * if m > 0.0 { ease(((now - start) / 1200.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
            if let Some(art) = &hero.hero {
                self.gfx.image_sized(art, Rect::new(0.0, 0.0, w, h).scaled(zoom), (w * 1.03, h * 1.03), Fit::CoverAt(0.3), 0.0, opacity);
            } else {
                // The icon stage: poster, icon or the service's name, right of centre.
                let cy = h / 2.0 + r;
                let drawn = if let Some(cap) = &hero.capsule {
                    let ph = 30.0 * r;
                    let pw = (ph * 2.0 / 3.0).min(22.0 * r);
                    self.gfx.image_sized(cap, Rect::new(w - 12.0 * r - pw, cy - ph / 2.0, pw, ph).scaled(zoom), (pw * 1.03, ph * 1.03), Fit::Cover, 16.0, opacity).is_some()
                } else if let Some(icon) = &hero.icon {
                    let s = 13.0 * r;
                    self.gfx.image_sized(icon, Rect::new(w - 12.0 * r - s, cy - s / 2.0, s, s).scaled(zoom), (s * 1.03, s * 1.03), Fit::Contain, 0.0, opacity).is_some()
                } else {
                    false
                };
                if !drawn {
                    if let Some(c) = hero.web {
                        let tw = 40.0 * r;
                        self.gfx.text(&hero.name, w - 12.0 * r - tw, cy - 4.5 * r, tw, 7.0 * r, 600, c.alpha(opacity), Align::Right, 0, -0.03);
                    } else if let Some((n, svg)) = hero.glyph.and_then(icons::icon) {
                        let s = 11.0 * r;
                        self.gfx.icon(n, svg, Rect::new(w - 12.0 * r - s, cy - s / 2.0, s, s), T2.alpha(opacity), false, 1.0);
                    }
                }
            }
        }
        // Scrims keep the text readable over any art.
        let bgc = |a: f32| BG.alpha(a);
        self.gfx.gradient(Rect::new(0.0, 0.0, w, h), &[(0.0, bgc(0.88)), (0.13, bgc(0.5)), (0.3, bgc(0.0)), (0.55, bgc(0.0)), (1.0, bgc(1.0))], true);
        self.gfx.gradient(Rect::new(0.0, 0.0, w, h), &[(0.0, bgc(0.85)), (0.45, bgc(0.35)), (0.7, bgc(0.0))], false);
        self.draw_hero_text(now, h);
    }

    fn draw_hero_text(&mut self, now: f64, hero_h: f32) {
        let r = self.rem();
        let Some((id, start)) = self.hero.clone() else { return };
        let (x, width) = (4.0 * r, 52.0 * r);
        let m = self.motion();
        let rise = |i: usize| {
            let p = if m > 0.0 { ease(((now - start - i as f64 * 50.0) / 420.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
            (p, (1.0 - p) * 8.0)
        };
        let bottom = hero_h - 9.0 * r;
        if let Some(cat) = id.strip_prefix("nav:") {
            let n = self.rows.iter().flat_map(|r| &r.tiles).find_map(|t| if let Tile::Nav { cat: c, count } = t { (c == cat).then_some(*count) } else { None }).unwrap_or(0);
            let title = format!("All {}", cat_label(cat).to_lowercase());
            let (p0, d0) = rise(0);
            self.gfx.text(&title, x, bottom - 9.0 * r + d0, width, 4.0 * r, 500, T1.alpha(p0), Align::Left, 2, -0.03);
            let (p1, d1) = rise(1);
            self.gfx.text(&format!("{n} in your library"), x, bottom - 3.5 * r + d1, width, 1.375 * r, 400, T2.alpha(p1), Align::Left, 0, 0.0);
            return;
        }
        let Some(hero) = self.hero_info(&id) else { return };
        // Layout from the bottom up: actions, meta, then the title or logo above them.
        let actions_y = bottom - 2.25 * r;
        let meta_y = actions_y - 1.5 * r - 1.375 * r * 1.35;
        let (p2, d2) = rise(2);
        if p2 > 0.0 {
            let cy = actions_y + 1.125 * r + d2;
            let c = T2.alpha(p2);
            let ax = self.hint(Key::Cross, hero.verb, x, cy, c);
            if hero.options {
                self.hint(Key::Triangle, "Options", ax - 0.25 * r, cy, c);
            }
        }
        let (p1, d1) = rise(1);
        self.gfx.text(&hero.meta, x, meta_y + d1, width, 1.375 * r, 400, T2.alpha(p1), Align::Left, 0, 0.0);
        let (p0, d0) = rise(0);
        let title_bottom = meta_y - 1.25 * r;
        let logo_drawn = hero.logo.as_ref().is_some_and(|logo| self.gfx.image(logo, Rect::new(x, title_bottom - 10.0 * r + d0, 34.0 * r, 10.0 * r), Fit::ContainLeftBottom, 0.0, p0).is_some());
        if !logo_drawn {
            let th = self.gfx.text_height(&hero.name, 4.0 * r, 500, width, 2);
            self.gfx.text(&hero.name, x, title_bottom - th + d0, width, 4.0 * r, 500, T1.alpha(p0), Align::Left, 2, -0.03);
        }
    }

    // ---------- Shelf ----------

    fn draw_shelf(&mut self, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let top = h * 0.58;
        let area = Rect::new(0.0, top, w, h - 5.0 * r - top);
        self.gfx.clip(area);
        let (tw, th, gap) = self.tile_size();
        let pad = 4.0 * r;
        let sy = self.scroll_y.value(now);
        if self.rows.is_empty() {
            let label = cat_label(&self.tab);
            self.gfx.text(&format!("Nothing in {label} yet"), pad, top + r, 50.0 * r, 1.375 * r, 500, T2, Align::Left, 0, 0.0);
            self.gfx.text("Move games and apps here from their options.", pad, top + 3.25 * r, 50.0 * r, 1.25 * r, 400, T3, Align::Left, 0, 0.0);
        }
        let cols = self.grid_cols();
        for ri in 0..self.rows.len() {
            let (title, count, key) = (self.rows[ri].title.clone(), self.rows[ri].count, self.rows[ri].key.clone());
            let y0 = top - sy + if self.grid { 0.0 } else { self.row_top(ri) };
            if !self.grid && (y0 > top + area.h || y0 + 20.0 * r < top) {
                continue;
            }
            self.gfx.text(&title, pad, y0, 30.0 * r, 1.375 * r, 500, T2, Align::Left, 0, 0.0);
            let (tw_title, _) = self.gfx.measure(&title, 1.375 * r, 500);
            self.gfx.text(&count.to_string(), pad + tw_title + 0.75 * r, y0 + 0.2 * r, 6.0 * r, 1.125 * r, 400, T3, Align::Left, 0, 0.0);
            let ty = y0 + 1.925 * r + 1.0 * r;
            let sx = self.scroll_x.get(ri).map_or(0.0, |t| t.value(now));
            for ci in 0..self.rows[ri].tiles.len() {
                let rect = if self.grid {
                    Rect::new(pad + (ci % cols) as f32 * (tw + gap), ty + (ci / cols) as f32 * (th + gap), tw, th)
                } else {
                    Rect::new(pad + ci as f32 * (tw + gap) - sx, ty, tw, th)
                };
                if rect.right() < -tw || rect.x > w + tw || rect.bottom() < top - th || rect.y > top + area.h + th {
                    continue;
                }
                let tile = self.rows[ri].tiles[ci].clone();
                let focused = self.layers.is_empty() && self.focus == (ri, ci);
                let sk = format!("{key}:{}", self.tile_id(&tile));
                let s = self.scale.get(&sk).map_or(if focused { 1.06 } else { 1.0 }, |t| t.value(now));
                self.draw_tile(&tile, rect.scaled(s), focused);
            }
        }
        self.gfx.unclip();
    }

    /// A tile's face: its poster, or a card with the program's icon and name.
    /// `base` is the size art is decoded at (the focused size, so focusing does not decode
    /// again).
    fn tile_face(&mut self, rect: Rect, base: (f32, f32), it: Option<&Item>, icon: Option<PathBuf>, name: &str, focused: bool) {
        let r = self.rem();
        let radius = 0.75 * r;
        let mut drawn = false;
        if let Some(cap) = it.and_then(|i| i.art.capsule.clone()) {
            self.gfx.fill(rect, CARD, radius);
            drawn = self.gfx.image_sized(&cap, rect, base, Fit::Cover, radius, 1.0).is_some();
        }
        if drawn {
            return;
        }
        self.gfx.fill(rect, CARD, radius);
        self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
        let icon_rect = Rect::new(rect.x + (rect.w - 4.5 * r) / 2.0, rect.y + 3.0 * r, 4.5 * r, 4.5 * r);
        let mut label = name.to_owned();
        let icon = it.and_then(|i| i.art.icon.clone()).or(icon);
        let s = base.0 / rect.w.max(1.0);
        let icon_ok = icon.is_some_and(|p| self.gfx.image_sized(&p, icon_rect, (icon_rect.w * s, icon_rect.h * s), Fit::Contain, 0.0, 1.0).is_some());
        if !icon_ok {
            match it {
                Some(it) if it.source == "web" => {
                    let c = it.color.as_deref().and_then(Color::parse).unwrap_or(T1);
                    self.gfx.text(&it.name, rect.x + 0.5 * r, rect.y + 5.0 * r, rect.w - r, 1.75 * r, 600, c, Align::Center, 2, -0.01);
                    label = if it.id == "web:browser" { "Browser".into() } else { "Streaming".into() };
                }
                _ => {
                    let glyph = it.and_then(|i| i.glyph).unwrap_or(if it.is_some_and(|i| i.category == "games") { "play" } else { "app" });
                    if let Some((n, svg)) = icons::icon(glyph) {
                        self.gfx.icon(n, svg, Rect::new(rect.x + (rect.w - 3.5 * r) / 2.0, rect.y + 3.5 * r, 3.5 * r, 3.5 * r), T2, false, 1.5);
                    }
                }
            }
        }
        let lh = self.gfx.text_height(&label, 1.125 * r, 400, rect.w - 2.0 * r, 2);
        self.gfx.text(&label, rect.x + r, rect.bottom() - 1.25 * r - lh, rect.w - 2.0 * r, 1.125 * r, 400, if focused { T1 } else { T2 }, Align::Center, 2, 0.0);
    }

    /// "Running", top left of a tile: the amber dot is the state.
    fn running_badge(&mut self, rect: Rect) {
        let r = self.rem();
        let label = "Running";
        let (lw, _) = self.gfx.measure(label, 0.875 * r, 500);
        let b = Rect::new(rect.x + 0.75 * r, rect.y + 0.75 * r, lw + 2.4 * r, 1.75 * r);
        self.gfx.fill(b, DEEP.alpha(0.8), 0.875 * r);
        self.gfx.fill(Rect::new(b.x + 0.7 * r, b.y + 0.625 * r, 0.5 * r, 0.5 * r), ACCENT, 0.25 * r);
        self.gfx.text(label, b.x + 1.55 * r, b.y + 0.3 * r, lw + 4.0, 0.875 * r, 500, T1, Align::Left, 0, 0.0);
    }

    fn draw_tile(&mut self, tile: &Tile, rect: Rect, focused: bool) {
        let r = self.rem();
        let radius = 0.75 * r;
        let (tw, th, _) = self.tile_size();
        let base = (tw * 1.06, th * 1.06);
        if focused {
            self.gfx.stroke(rect.inset(-4.0 - 1.5), T1, radius + 5.5, 3.0);
        }
        match tile {
            Tile::Nav { cat, count } => {
                self.gfx.fill(rect, CARD, radius);
                self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
                if let Some((n, svg)) = icons::icon("move") {
                    self.gfx.icon(n, svg, Rect::new(rect.x + (rect.w - 3.5 * r) / 2.0, rect.y + rect.h * 0.28, 3.5 * r, 3.5 * r), T2, false, 1.5);
                }
                let label = format!("All {}", cat_label(cat).to_lowercase());
                self.gfx.text(&label, rect.x + r, rect.y + rect.h * 0.55, rect.w - 2.0 * r, 1.125 * r, 400, T1, Align::Center, 2, 0.0);
                self.gfx.text(&count.to_string(), rect.x + r, rect.y + rect.h * 0.55 + 1.6 * r, rect.w - 2.0 * r, r, 400, T3, Align::Center, 0, 0.0);
            }
            Tile::Item(i) => {
                let it = self.items[*i].clone();
                self.tile_face(rect, base, Some(&it), None, &it.name, focused);
                if self.item_running(*i) {
                    self.running_badge(rect);
                }
                if it.favorite {
                    let b = Rect::new(rect.right() - 2.5 * r, rect.y + 0.75 * r, 1.75 * r, 1.75 * r);
                    self.gfx.fill(b, DEEP.alpha(0.72), 6.0);
                    if let Some((n, svg)) = icons::icon("star") {
                        self.gfx.icon(n, svg, b.inset(0.375 * r), ACCENT, true, 0.0);
                    }
                }
            }
            Tile::Running(h) => {
                let name = self.run_name(*h);
                let (item, icon) = match self.run_of(*h) {
                    Some(run) => (run.item.and_then(|i| self.items.get(i)).cloned(), run.icon.clone()),
                    None => (None, None),
                };
                self.tile_face(rect, base, item.as_ref(), icon, &name, focused);
                self.running_badge(rect);
            }
        }
    }

    // ---------- Hint bar ----------

    fn draw_hints(&mut self) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let bar = Rect::new(0.0, h - 5.0 * r, w, 5.0 * r);
        self.gfx.fill(bar, BG, 0.0);
        self.gfx.fill(Rect::new(0.0, bar.y, w, 1.0), LINE, 0.0);
        if !self.layers.is_empty() {
            return;
        }
        let cy = bar.y + bar.h / 2.0;
        let (accept, options) = match self.focused_tile() {
            Some(Tile::Running(_)) => ("Resume", true),
            Some(Tile::Item(i)) => (if self.item_running(*i) { "Resume" } else { "Select" }, self.items[*i].source != "system"),
            Some(Tile::Nav { .. }) => ("Select", false),
            None => ("", false),
        };
        let mut x = 4.0 * r;
        if !accept.is_empty() {
            x = self.hint(Key::Cross, accept, x, cy, T2);
        }
        if options {
            x = self.hint(Key::Triangle, "Options", x, cy, T2);
        }
        x = self.hint(Key::Square, "Search", x, cy, T2);
        self.hint(Key::Options, "Settings", x, cy, T2);
        // Volume, right.
        let label = "Volume";
        let lw = self.gfx.measure(label, 1.125 * r, 400).0;
        let k2 = self.key_width(Key::R2);
        let k1 = self.key_width(Key::L2);
        let mut rx = w - 4.0 * r - lw - 0.75 * r - k2 - 0.5 * r - k1;
        rx += self.keycap(Key::L2, rx, cy) + 0.5 * r;
        rx += self.keycap(Key::R2, rx, cy) + 0.75 * r;
        self.gfx.text(label, rx, cy - 0.73 * r, lw + 4.0, 1.125 * r, 400, T3, Align::Left, 0, 0.0);
    }

    // ---------- Layers ----------

    fn draw_options(&mut self, opts: &[Opt], focus: usize, x: f32, mut y: f32, width: f32) {
        let r = self.rem();
        for (i, o) in opts.iter().enumerate() {
            let rect = Rect::new(x, y, width, 4.0 * r);
            let on = i == focus;
            if on {
                self.gfx.fill(rect, RAISED, 10.0);
                self.gfx.stroke(rect.inset(0.5), LINE, 10.0, 1.0);
            }
            let mut tx = x + r;
            if let Some((n, svg)) = o.icon.and_then(icons::icon) {
                self.gfx.icon(n, svg, Rect::new(tx, y + 1.25 * r, 1.5 * r, 1.5 * r), if on { T1 } else { T3 }, false, 1.5);
                tx += 2.5 * r;
            }
            let room = if o.value.is_some() { 13.0 * r } else if o.switch.is_some() { 5.5 * r } else { r };
            self.gfx.text(&o.label, tx, y + 1.1 * r, width - (tx - x) - room, 1.375 * r, 400, if on { T1 } else { T2 }, Align::Left, 0, 0.0);
            if let Some(v) = &o.value {
                self.gfx.text(v, x + width - 13.0 * r, y + 1.2 * r, 12.0 * r, 1.25 * r, 400, T3, Align::Right, 0, 0.0);
            }
            if let Some(sw) = o.switch {
                let s = Rect::new(x + width - r - 3.25 * r, y + r, 3.25 * r, 2.0 * r);
                self.gfx.fill(s, if sw { ACCENT } else { RAISED }, r);
                if !sw {
                    self.gfx.stroke(s.inset(0.5), LINE, r, 1.0);
                }
                let k = Rect::new(s.x + 0.25 * r + if sw { 1.25 * r } else { 0.0 }, s.y + 0.25 * r, 1.5 * r, 1.5 * r);
                self.gfx.fill(k, if sw { INK } else { T2 }, 0.75 * r);
            }
            y += 4.25 * r;
        }
    }

    fn draw_layers(&mut self, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        for li in 0..self.layers.len() {
            let layer = self.layers[li].clone();
            match layer {
                Layer::Sheet { title, sub, opts, focus, .. } => {
                    self.gfx.fill(Rect::new(0.0, 0.0, w, h), SCRIM, 0.0);
                    let panel = Rect::new(w - 36.0 * r, 0.0, 36.0 * r, h);
                    self.gfx.fill(panel, DEEP, 0.0);
                    self.gfx.fill(Rect::new(panel.x, 0.0, 1.0, h), LINE, 0.0);
                    let x = panel.x + 2.0 * r;
                    self.gfx.text(&title, x + r, 3.0 * r, panel.w - 6.0 * r, 2.0 * r, 500, T1, Align::Left, 2, -0.02);
                    let th = self.gfx.text_height(&title, 2.0 * r, 500, panel.w - 6.0 * r, 2);
                    self.gfx.text(&sub, x + r, 3.0 * r + th + 0.5 * r, panel.w - 6.0 * r, 1.25 * r, 400, T3, Align::Left, 2, 0.0);
                    // Long lists scroll so the focused option stays above the hints.
                    let start = 3.0 * r + th + 3.5 * r;
                    let limit = h - 6.0 * r;
                    let off = ((focus + 1) as f32 * 4.25 * r - (limit - start)).max(0.0);
                    self.gfx.clip(Rect::new(panel.x, start - 0.5 * r, panel.w, limit - start + 0.5 * r));
                    self.draw_options(&opts, focus, x, start - off, panel.w - 4.0 * r);
                    self.gfx.unclip();
                    let fy = h - 3.0 * r - 1.125 * r;
                    let fx = self.hint(Key::Cross, "Select", x + r, fy, T2);
                    self.hint(Key::Circle, "Back", fx, fy, T2);
                }
                Layer::Dialog { title, body, opts, focus } => {
                    self.gfx.fill(Rect::new(0.0, 0.0, w, h), SCRIM, 0.0);
                    let pw = 42.0 * r;
                    let tw = pw - 5.0 * r;
                    let tth = self.gfx.text_height(&title, 2.0 * r, 500, tw, 2);
                    let bh = self.gfx.text_height(&body, 1.375 * r, 400, tw, 6);
                    let ph = 2.5 * r + tth + 0.75 * r + bh + 2.0 * r + opts.len() as f32 * 4.25 * r + 1.75 * r;
                    let p = Rect::new((w - pw) / 2.0, (h - ph) / 2.0, pw, ph);
                    self.gfx.fill(p, CARD, 20.0);
                    self.gfx.stroke(p.inset(0.5), LINE, 20.0, 1.0);
                    self.gfx.text(&title, p.x + 2.5 * r, p.y + 2.5 * r, tw, 2.0 * r, 500, T1, Align::Left, 2, -0.02);
                    let by = p.y + 2.5 * r + tth + 0.75 * r;
                    self.gfx.text(&body, p.x + 2.5 * r, by, tw, 1.375 * r, 400, T2, Align::Left, 6, 0.0);
                    self.draw_options(&opts, focus, p.x + 1.5 * r, by + bh + 2.0 * r, pw - 3.0 * r);
                }
                Layer::Launch { item, at } => self.draw_launch(item, at, now),
                Layer::Keyboard(kb) => self.draw_keyboard(&kb),
                Layer::Welcome { opts, focus, at } => self.draw_welcome(&opts, focus, at, now),
            }
        }
    }

    fn draw_launch(&mut self, item: usize, at: f64, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let Some(it) = self.items.get(item).cloned() else { return };
        let m = self.motion();
        let t = if m > 0.0 { ((now - at) / 240.0).clamp(0.0, 1.0) as f32 } else { 1.0 };
        self.gfx.fill(Rect::new(0.0, 0.0, w, h), DEEP.alpha(t), 0.0);
        let ph = h * 0.36;
        let pw = ph * 2.0 / 3.0;
        let p = Rect::new((w - pw) / 2.0, h / 2.0 - ph / 2.0 - h * 0.06, pw, ph);
        let grow = 0.9 + 0.1 * if m > 0.0 { ease(((now - at) / 560.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
        let drawn = it.art.capsule.as_ref().is_some_and(|cap| self.gfx.image_sized(cap, p.scaled(grow), (p.w, p.h), Fit::Cover, 12.0, t).is_some());
        if !drawn {
            self.gfx.fill(p.scaled(grow), CARD.alpha(t), 12.0);
            let icon_ok = it.art.icon.as_ref().is_some_and(|icon| self.gfx.image_sized(icon, p.scaled(grow * 0.45), (p.w * 0.45, p.h * 0.45), Fit::Contain, 0.0, t).is_some());
            if !icon_ok && it.source == "web" {
                let c = it.color.as_deref().and_then(Color::parse).unwrap_or(T1).alpha(t);
                let q = p.scaled(grow);
                self.gfx.text(&it.name, q.x + r, q.y + q.h / 2.0 - 1.6 * r, q.w - 2.0 * r, 2.5 * r, 600, c, Align::Center, 2, -0.02);
            }
        }
        let cy = h / 2.0 + 15.0 * r;
        let c = if m > 0.0 { ease(((now - at - 220.0) / 420.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
        self.gfx.text(&it.name, 0.0, cy + (1.0 - c) * 8.0, w, 2.5 * r, 500, T1.alpha(c), Align::Center, 0, -0.02);
        let bar = Rect::new((w - 12.0 * r) / 2.0, cy + 4.5 * r, 12.0 * r, 2.0);
        self.gfx.fill(bar, RAISED, 1.0);
        let fill = 0.92 * ease(((now - at) / 6000.0).clamp(0.0, 1.0)) as f32;
        self.gfx.fill(Rect::new(bar.x, bar.y, bar.w * fill, bar.h), ACCENT, 1.0);
    }

    fn draw_keyboard(&mut self, kb: &Keyboard) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        self.gfx.fill(Rect::new(0.0, 0.0, w, h), SCRIM, 0.0);
        let sheet = Rect::new(0.0, h - 33.0 * r, w, 33.0 * r);
        self.gfx.fill(sheet, DEEP, 0.0);
        self.gfx.fill(Rect::new(0.0, sheet.y, w, 1.0), LINE, 0.0);
        let x0 = 4.0 * r;
        let (kw, kh, kg) = (4.5 * r, 3.5 * r, 0.5 * r);
        let kb_w = 10.0 * kw + 9.0 * kg;
        let search = kb.purpose == KbPurpose::Search;
        // The text field.
        let field = Rect::new(x0, sheet.y + 2.5 * r, kb_w, 3.75 * r);
        self.gfx.fill(field, CARD, 0.75 * r);
        self.gfx.stroke(field.inset(0.5), LINE, 0.75 * r, 1.0);
        let size = 1.5 * r;
        let ty = field.y + (field.h - size * 1.3) / 2.0;
        let mut tx = field.x + 1.25 * r;
        if search && let Some((n, svg)) = icons::icon("search") {
            self.gfx.icon(n, svg, Rect::new(tx, field.y + (field.h - 1.5 * r) / 2.0, 1.5 * r, 1.5 * r), T3, false, 1.5);
            tx += 2.5 * r;
        }
        let room = field.right() - tx - 1.25 * r;
        let caret_x = if kb.text.is_empty() {
            self.gfx.text(if search { "Search games and apps" } else { "Name" }, tx + 0.4 * r, ty, room, size, 400, T3, Align::Left, 0, 0.0);
            tx
        } else {
            self.gfx.text(&kb.text, tx, ty, room, size, 400, T1, Align::Left, 0, 0.0);
            tx + self.gfx.measure(&kb.text, size, 400).0.min(room)
        };
        if kb.result.is_none() {
            self.gfx.fill(Rect::new(caret_x + 1.0, field.y + 0.85 * r, 2.0, field.h - 1.7 * r), ACCENT, 0.0);
        }
        // The keys.
        let ky = field.bottom() + 1.5 * r;
        let on_keys = kb.result.is_none();
        for (row, chars) in KB_ROWS.iter().enumerate() {
            for (col, c) in chars.chars().enumerate() {
                let rect = Rect::new(x0 + col as f32 * (kw + kg), ky + row as f32 * (kh + kg), kw, kh);
                let focused = on_keys && kb.key == (row, col);
                self.gfx.fill(rect, if focused { T1 } else { CARD }, 0.5 * r);
                let label = if kb.shift { c.to_ascii_uppercase().to_string() } else { c.to_string() };
                self.gfx.text(&label, rect.x, rect.y + (kh - 1.375 * r * 1.3) / 2.0, kw, 1.375 * r, 500, if focused { DEEP } else { T1 }, Align::Center, 0, 0.0);
            }
        }
        let by = ky + 4.0 * (kh + kg);
        for (key, c0, span) in KB_BOTTOM {
            let rect = Rect::new(x0 + c0 as f32 * (kw + kg), by, span as f32 * kw + (span - 1) as f32 * kg, kh);
            let focused = on_keys && kb.key.0 == 4 && kb.key.1 >= c0 && kb.key.1 < c0 + span;
            let lit = key == "shift" && kb.shift;
            self.gfx.fill(rect, if focused { T1 } else if lit { RAISED } else { CARD }, 0.5 * r);
            let c = if focused { DEEP } else if lit { ACCENT } else { T1 };
            let icon = match key {
                "space" => Some("space"),
                "erase" => Some("backspace"),
                _ => None,
            };
            if let Some((n, svg)) = icon.and_then(icons::icon) {
                self.gfx.icon(n, svg, Rect::new(rect.x + (rect.w - 1.5 * r) / 2.0, rect.y + (kh - 1.5 * r) / 2.0, 1.5 * r, 1.5 * r), c, false, 1.75);
            } else {
                let label = if key == "shift" { "Aa" } else { key };
                self.gfx.text(label, rect.x, rect.y + (kh - 1.125 * r * 1.3) / 2.0, rect.w, 1.125 * r, 500, c, Align::Center, 0, 0.0);
            }
        }
        // Results (search) or what is being renamed, to the right.
        let rx = x0 + kb_w + 4.0 * r;
        let rw = w - rx - 4.0 * r;
        if rw > 12.0 * r {
            if let KbPurpose::Rename(i) = kb.purpose {
                let name = self.items.get(i).map_or(String::new(), |it| it.name.clone());
                self.gfx.text("Renaming", rx, field.y + 0.2 * r, rw, 1.25 * r, 400, T3, Align::Left, 0, 0.0);
                self.gfx.text(&name, rx, field.y + 2.0 * r, rw, 2.0 * r, 500, T1, Align::Left, 2, -0.02);
                self.gfx.text("Done saves the name. Clear it and press Done to get the original name back.", rx, ky + 4.5 * r, rw.min(36.0 * r), 1.25 * r, 400, T3, Align::Left, 3, 0.0);
            } else {
                let title = if kb.text.trim().is_empty() {
                    "Type to find a game or app".to_owned()
                } else if kb.hits.is_empty() {
                    "No matches".to_owned()
                } else {
                    "Results".to_owned()
                };
                self.gfx.text(&title, rx, field.y + (field.h - 1.375 * r * 1.3) / 2.0, rw, 1.375 * r, 500, T2, Align::Left, 0, 0.0);
                // Results as a list: poster or icon, name, where it comes from. Rows line up
                // with the keys, so Right from a key lands on the result beside it.
                let colw = ((rw - 1.0 * r) / 2.0).min(30.0 * r);
                let hits = kb.hits.clone();
                for (n, &i) in hits.iter().enumerate() {
                    let (c, row) = (n % KB_RESULT_COLS, n / KB_RESULT_COLS);
                    let rect = Rect::new(rx + c as f32 * (colw + 1.0 * r), ky + row as f32 * (kh + kg), colw, kh);
                    self.search_result(rect, i, kb.result == Some(n));
                }
            }
        }
        // Hints.
        let hy = sheet.bottom() - 2.75 * r;
        let mut x = x0;
        x = self.hint(Key::Cross, "Select", x, hy, T2);
        x = self.hint(Key::Square, "Delete", x, hy, T2);
        x = self.hint(Key::Triangle, "Space", x, hy, T2);
        x = self.hint(Key::Options, "Done", x, hy, T2);
        self.hint(Key::Circle, "Close", x, hy, T2);
    }

    fn search_result(&mut self, rect: Rect, i: usize, focused: bool) {
        let r = self.rem();
        let it = self.items[i].clone();
        self.gfx.fill(rect, if focused { RAISED } else { CARD }, 0.5 * r);
        if focused {
            self.gfx.stroke(rect.inset(-3.5), T1, 0.5 * r + 3.5, 2.5);
        }
        let thumb = Rect::new(rect.x + 0.4 * r, rect.y + 0.4 * r, (rect.h - 0.8 * r) * 2.0 / 3.0, rect.h - 0.8 * r);
        let shown = it.art.capsule.as_ref().is_some_and(|cap| self.gfx.image(cap, thumb, Fit::Cover, 0.3 * r, 1.0).is_some())
            || it.art.icon.as_ref().is_some_and(|icon| self.gfx.image(icon, thumb.inset(0.2 * r), Fit::Contain, 0.0, 1.0).is_some());
        if !shown && let Some((n, svg)) = icons::icon(it.glyph.unwrap_or(if it.category == "games" { "play" } else { "app" })) {
            self.gfx.icon(n, svg, thumb.inset(0.35 * r), T3, false, 1.5);
        }
        let tx = thumb.right() + 0.9 * r;
        let tw = rect.right() - tx - 0.75 * r;
        self.gfx.text(&it.name, tx, rect.y + 0.45 * r, tw, 1.125 * r, 500, if focused { T1 } else { T2 }, Align::Left, 0, 0.0);
        let meta = self.source_label(&it);
        self.gfx.text(&meta, tx, rect.y + 1.95 * r, tw, 0.95 * r, 400, T3, Align::Left, 0, 0.0);
    }

    fn draw_welcome(&mut self, opts: &[Opt], focus: usize, at: f64, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let m = self.motion();
        let fade = if m > 0.0 { ease(((now - at) / 600.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
        self.gfx.fill(Rect::new(0.0, 0.0, w, h), DEEP.alpha(fade), 0.0);
        let img = self.welcome_jpg.clone();
        self.gfx.image(&img, Rect::new(0.0, 0.0, w, h), Fit::CoverAt(0.5), 0.0, fade);
        let d = |c: f32| DEEP.alpha(c * fade);
        self.gfx.gradient(Rect::new(0.0, 0.0, w, h), &[(0.0, d(0.96)), (0.42, d(0.8)), (0.75, d(0.15)), (1.0, d(0.0))], false);
        self.gfx.gradient(Rect::new(0.0, h * 0.6, w, h * 0.4), &[(0.0, d(0.0)), (1.0, d(0.7))], true);
        let rise = |i: usize| {
            let p = if m > 0.0 { ease(((now - at - 150.0 - i as f64 * 70.0) / 520.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
            (p, (1.0 - p) * 10.0)
        };
        let x = 7.0 * r;
        let width = 44.0 * r;
        let mut y = h * 0.2;
        let (p0, d0) = rise(0);
        let icon = self.icon_png.clone();
        self.gfx.image(&icon, Rect::new(x, y + d0, 3.0 * r, 3.0 * r), Fit::Contain, 0.0, p0);
        self.gfx.text("Sofa", x + 4.0 * r, y + 0.55 * r + d0, 20.0 * r, 1.75 * r, 600, T1.alpha(p0), Align::Left, 0, 0.0);
        y += 6.0 * r;
        let (p1, d1) = rise(1);
        let title = "Your PC, ready for the couch";
        let tth = self.gfx.text_height(title, 4.0 * r, 500, width, 2);
        self.gfx.text(title, x, y + d1, width, 4.0 * r, 500, T1.alpha(p1), Align::Left, 2, -0.03);
        y += tth + 1.25 * r;
        let (p2, d2) = rise(2);
        let body = "Your games and apps in one place, made for a controller. Press the PS button in any game to come back here.";
        let bh = self.gfx.text_height(body, 1.5 * r, 400, width, 4);
        self.gfx.text(body, x, y + d2, width, 1.5 * r, 400, T2.alpha(p2), Align::Left, 4, 0.0);
        y += bh + 3.0 * r;
        let (p3, d3) = rise(3);
        if p3 > 0.0 {
            self.draw_options(opts, focus, x - r, y + d3, 40.0 * r);
            let hy = h - 4.0 * r;
            self.hint(Key::Cross, "Select", x, hy, T2.alpha(p3));
        }
    }

    fn draw_toast(&mut self, now: f64) {
        let r = self.rem();
        let Some((title, body, at)) = self.toast.clone() else { return };
        let age = if at < 0.0 { 0.0 } else { now - at };
        let p = ease((age / 260.0).clamp(0.0, 1.0)) as f32 * (1.0 - ease(((age - 5000.0) / 260.0).clamp(0.0, 1.0)) as f32);
        let w = 28.0 * r;
        let bh = if body.is_empty() { 0.0 } else { self.gfx.text_height(&body, r, 400, w - 2.5 * r, 3) + 0.25 * r };
        let rect = Rect::new(self.gfx.width - 4.0 * r - w, 6.0 * r + (1.0 - p) * 8.0, w, 2.0 * r + 1.5 * r + bh);
        self.gfx.fill(Rect::new(rect.x, rect.y + 4.0, rect.w, rect.h), Color(0.0, 0.0, 0.0, 0.2 * p), 14.0);
        self.gfx.fill(rect, CARD.alpha(p), 14.0);
        self.gfx.stroke(rect.inset(0.5), LINE.alpha(p), 14.0, 1.0);
        self.gfx.text(&title, rect.x + 1.25 * r, rect.y + r, w - 2.5 * r, 1.125 * r, 500, T1.alpha(p), Align::Left, 0, 0.0);
        self.gfx.text(&body, rect.x + 1.25 * r, rect.y + 2.75 * r, w - 2.5 * r, r, 400, T2.alpha(p), Align::Left, 3, 0.0);
    }

    /// Start-up: Sofa's name on black, then Home fades in under it.
    fn draw_splash(&mut self, now: f64) {
        let t = now - self.boot_at;
        if t > 1000.0 || self.motion() == 0.0 {
            return;
        }
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let a = 1.0 - ease(((t - 350.0) / 550.0).clamp(0.0, 1.0)) as f32;
        self.gfx.fill(Rect::new(0.0, 0.0, w, h), DEEP.alpha(a), 0.0);
        let name = 1.0 - ease(((t - 250.0) / 350.0).clamp(0.0, 1.0)) as f32;
        self.gfx.text("Sofa", 0.0, h / 2.0 - 1.9 * r, w, 2.75 * r, 600, T1.alpha(name.min(a)), Align::Center, 0, -0.02);
    }
}

fn system_desc(key: &str) -> &'static str {
    match key {
        "sleep" => "The PC sleeps until you wake it",
        "restart" => "Restarts the PC",
        "shutdown" => "Turns the PC off",
        "desktop" => "Shows the Windows desktop. Press the PS button to come back",
        "quit" => "Closes Sofa completely",
        "bigpicture" => "Opens Steam in Big Picture mode",
        "settings" => "Start with Windows, buttons, browser and sounds",
        "rescan" => "Looks for newly installed games and apps",
        "display" => "Resolution, HDR and scaling",
        "sound" => "TV, headphones or the controller",
        "bluetooth" => "Pair a controller or headphones",
        _ => "",
    }
}

fn ago(t: f64) -> String {
    let d = ((epoch_ms() - t) / 86_400_000.0).floor() as i64;
    match d {
        i64::MIN..=0 => "today".into(),
        1 => "yesterday".into(),
        2..=6 => format!("{d} days ago"),
        7..=29 => {
            let w = (d as f64 / 7.0).round() as i64;
            format!("{w} week{} ago", if w == 1 { "" } else { "s" })
        }
        _ => {
            // "in May", or "in May 2025" for older dates.
            let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
            let days = (t / 86_400_000.0).floor() as i64;
            let (y, m) = civil_from_days(days);
            if d > 300 { format!("in {} {y}", months[m]) } else { format!("in {}", months[m]) }
        }
    }
}

/// Year and month (0-based) from days since 1970-01-01 (UTC).
fn civil_from_days(z: i64) -> (i64, usize) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), (m - 1) as usize)
}

/// The time in the person's own format, without seconds.
fn clock_text() -> String {
    use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    let mut buf = [0u16; 64];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetTimeFormatEx(windows::core::PCWSTR::null(), TIME_NOSECONDS, None, windows::core::PCWSTR::null(), Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}
