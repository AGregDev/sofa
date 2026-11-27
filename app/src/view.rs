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
const WHITE: Color = Color::hex(0xffffff);

/// A streaming service's own look, standing in for art: its colour and its mark, or its name
/// as a wordmark where Sofa has no mark for it.
#[derive(Clone, Copy)]
struct WebLook {
    bg: Color,
    fg: Color,
    mark: Option<&'static str>,
}

/// What ✕ does on a Discord tile, as the hints say it.
fn soc_verb(s: &Soc) -> &'static str {
    match s {
        Soc::Join(_) => "Join",
        Soc::Leave => "Leave",
        Soc::Server(_) | Soc::Account | Soc::Open => "Open",
        Soc::Here | Soc::Member(_) => "Who's here",
        Soc::Setup => "Set up",
        Soc::Mute | Soc::Deaf => "Switch",
    }
}

fn web_look(id: &str) -> Option<WebLook> {
    let white = Color::hex(0xffffff);
    Some(match id {
        "web:youtube" => WebLook { bg: Color::hex(0xff0033), fg: white, mark: Some("youtube") },
        "web:netflix" => WebLook { bg: Color::hex(0x141414), fg: Color::hex(0xe50914), mark: Some("netflix") },
        "web:twitch" => WebLook { bg: Color::hex(0x9146ff), fg: white, mark: Some("twitch") },
        "web:crunchyroll" => WebLook { bg: Color::hex(0xf47521), fg: white, mark: Some("crunchyroll") },
        "web:prime" => WebLook { bg: Color::hex(0x0f171e), fg: Color::hex(0x1a98ff), mark: None },
        "web:disney" => WebLook { bg: Color::hex(0x0b1b46), fg: white, mark: None },
        _ => return None,
    })
}

/// What the hero shows for the focused tile.
struct Hero {
    hero: Option<PathBuf>,
    capsule: Option<PathBuf>,
    icon: Option<PathBuf>,
    logo: Option<PathBuf>,
    name: String,
    /// Streaming services: their colour and mark stand in for art.
    look: Option<WebLook>,
    glyph: Option<&'static str>,
    /// Square art (a playlist or album): drawn large, its colour washing the stage.
    cover: Option<PathBuf>,
    meta: String,
    verb: &'static str,
    options: bool,
}

impl App {
    pub fn draw(&mut self, now: f64) {
        let started = std::time::Instant::now();
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
        if self.store.flag("perfOverlay") {
            self.draw_perf(now);
        }
        // The time this frame took to draw (not the wait for the screen).
        self.frames.record(now, started.elapsed().as_secs_f32() * 1000.0);
        if self.gfx.end().is_err() {
            self.draw_errors += 1;
        } else {
            self.draw_errors = 0;
        }
    }

    /// The performance overlay: Sofa's frame rate, memory and video memory, top left, over
    /// everything (Performance in the quick menu turns it on).
    fn draw_perf(&mut self, now: f64) {
        let r = self.rem();
        let size = 0.875 * r;
        let mut lines: Vec<(String, String)> = vec![("Sofa".into(), self.fps_text(now))];
        if let Some(u) = self.perf.clone() {
            if u.ram_total > 0 {
                lines.push(("Memory".into(), crate::perf::of(u.ram_used, u.ram_total)));
            }
            for g in u.gpus.iter().flatten().take(2) {
                lines.push(("Video memory".into(), g.used.map_or_else(|| format!("{} in all", crate::perf::gb(g.total)), |used| crate::perf::of(used, g.total))));
            }
            lines.push(("Sofa's memory".into(), crate::perf::mb(u.sofa)));
        }
        let label_w = lines.iter().map(|(l, _)| self.gfx.measure(l, size, 400).0).fold(0.0, f32::max);
        let value_w = lines.iter().map(|(_, v)| self.gfx.measure(v, size, 500).0).fold(0.0, f32::max);
        let line_h = 1.5 * size;
        let pad = 0.75 * r;
        let rect = Rect::new(4.0 * r, 5.25 * r, label_w + value_w + 1.5 * r + 2.0 * pad, line_h * lines.len() as f32 + 2.0 * pad - (line_h - size * 1.33));
        self.gfx.fill(rect, DEEP.alpha(0.88), 0.5 * r);
        self.gfx.stroke(rect.inset(0.5), LINE, 0.5 * r, 1.0);
        for (i, (label, value)) in lines.iter().enumerate() {
            let y = rect.y + pad + i as f32 * line_h;
            self.gfx.text(label, rect.x + pad, y, label_w + 4.0, size, 400, T2, Align::Left, 0, 0.0);
            self.gfx.text(value, rect.x + pad + label_w + 1.5 * r, y, value_w + 4.0, size, 500, T1, Align::Left, 0, 0.0);
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
            Key::Home => ["", "Guide", "Home"],
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
        if k == Key::Home && label.is_empty() {
            // The PS button: PlayStation's mark, filled.
            if let Some((n, path)) = icons::brand("playstation") {
                self.gfx.icon(n, path, Rect::new(x + (w - 1.25 * r) / 2.0, cy - 0.625 * r, 1.25 * r, 1.25 * r), T1, true, 0.0);
            }
        } else if label.is_empty() {
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
        // Right: the clock, then leftwards the controller's battery, the sleep timer and a muted
        // microphone (amber: states that are on). While L2 or R2 changes the volume, its level
        // takes their place for a moment.
        let since = now - self.vol_at;
        let quick = self.layers.iter().any(|l| matches!(l, Layer::Quick { .. }));
        let hud = if quick || since >= 1800.0 {
            0.0
        } else if self.motion() == 0.0 {
            1.0
        } else {
            ease((since / 140.0).clamp(0.0, 1.0)) as f32 * (1.0 - ease(((since - 1500.0) / 300.0).clamp(0.0, 1.0)) as f32)
        };
        let status = 1.0 - hud;
        if status > 0.0 {
            let clock = clock_text();
            let (cw, _) = self.gfx.measure(&clock, size, 400);
            let mut rx = w - pad - cw;
            self.gfx.text(&clock, rx, cy - size * 0.65, cw + 4.0, size, 400, T2.alpha(status), Align::Left, 0, 0.0);
            let info = self.input.info();
            if let (true, Some(b)) = (info.connected, info.battery) {
                let text = format!("{b}%");
                let (tw, _) = self.gfx.measure(&text, 1.25 * r, 400);
                let low = b <= 20 && !info.charging;
                rx -= 1.5 * r + tw;
                self.gfx.text(&text, rx, cy - 0.8 * r, tw + 4.0, 1.25 * r, 400, (if low { ACCENT } else { T2 }).alpha(status), Align::Left, 0, 0.0);
                if info.charging
                    && let Some((n, svg)) = icons::icon("zap")
                {
                    rx -= 1.625 * r;
                    self.gfx.icon(n, svg, Rect::new(rx, cy - 0.5625 * r, 1.125 * r, 1.125 * r), T2.alpha(status), false, 1.5);
                }
                if let Some((n, path)) = icons::brand("playstation") {
                    rx -= 1.75 * r;
                    self.gfx.icon(n, path, Rect::new(rx, cy - 0.625 * r, 1.25 * r, 1.25 * r), T3.alpha(status), true, 0.0);
                }
            }
            if let Some(ms) = self.sleep_left() {
                let text = duration_text((ms / 60_000.0).ceil() as u32);
                let (tw, _) = self.gfx.measure(&text, 1.25 * r, 400);
                rx -= 1.75 * r + tw;
                self.gfx.text(&text, rx, cy - 0.8 * r, tw + 4.0, 1.25 * r, 400, T2.alpha(status), Align::Left, 0, 0.0);
                if let Some((n, svg)) = icons::icon("sleep") {
                    rx -= 1.75 * r;
                    self.gfx.icon(n, svg, Rect::new(rx, cy - 0.625 * r, 1.25 * r, 1.25 * r), ACCENT.alpha(status), false, 1.5);
                }
            }
            if self.mic_muted()
                && let Some((n, svg)) = icons::icon("micOff")
            {
                rx -= 3.0 * r;
                self.gfx.icon(n, svg, Rect::new(rx, cy - 0.625 * r, 1.25 * r, 1.25 * r), ACCENT.alpha(status), false, 1.5);
            }
        }
        if hud > 0.0 {
            let (v, muted) = (self.audio.volume.unwrap_or(0.0), self.audio.muted);
            let text = if muted { "Muted".to_owned() } else { format!("{}%", (v * 100.0).round()) };
            let (tw, _) = self.gfx.measure(&text, size, 400);
            let tx = w - pad - tw;
            self.gfx.text(&text, tx, cy - size * 0.65, tw + 4.0, size, 400, T1.alpha(hud), Align::Left, 0, 0.0);
            let bar = Rect::new(tx - 1.25 * r - 10.0 * r, cy - 0.1875 * r, 10.0 * r, 0.375 * r);
            self.gfx.fill(bar, RAISED.alpha(hud), 0.1875 * r);
            if !muted {
                self.gfx.fill(Rect::new(bar.x, bar.y, bar.w * v, bar.h), ACCENT.alpha(hud), 0.1875 * r);
            }
            if let Some((n, svg)) = icons::icon(if muted { "volumeX" } else { "sound" }) {
                self.gfx.icon(n, svg, Rect::new(bar.x - 2.5 * r, cy - 0.75 * r, 1.5 * r, 1.5 * r), T2.alpha(hud), false, 1.5);
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
                Launch::Web { tv: true, .. } => format!("TV interface in {}", platform::chosen_browser(self.store.text("browser")).map_or("your browser", |b| b.name)),
                _ => format!("Opens full screen in {}", platform::chosen_browser(self.store.text("browser")).map_or("your browser", |b| b.name)),
            },
            "system" => system_desc(it.glyph.unwrap_or(""), &self.home_press()),
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
        let run = self.running.iter().find(|r| r.item == Some(i));
        let page = run.map(|r| self.run_page(r.win.hwnd)).unwrap_or_default();
        let meta = if running && !page.is_empty() {
            format!("Running  ·  {page}")
        } else if running {
            // How long this session has lasted: the couch loses track of time.
            match run.and_then(|r| r.since).map(|t| ((epoch_ms() - t) / 60_000.0).floor().max(0.0) as u32) {
                Some(0) => "Running  ·  Started just now".into(),
                Some(m) => format!("Running for {}", duration_text(m)),
                None => format!("Running  ·  {source}"),
            }
        } else {
            let mut parts = vec![source];
            if it.last_played > 0.0 {
                parts.push(format!("{} {}", if it.category == "games" { "Played" } else { "Opened" }, ago(it.last_played)));
            }
            if it.playtime > 0 {
                parts.push(playtime_text(it.playtime));
            }
            parts.join("  ·  ")
        };
        Hero {
            hero: it.art.hero.clone(),
            capsule: it.art.capsule.clone(),
            cover: None,
            icon: it.art.icon.clone(),
            logo: it.art.logo.clone(),
            name: it.name.clone(),
            look: web_look(&it.id),
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
        if let Some(key) = id.strip_prefix("mu:") {
            let m = &self.music;
            let brand = |name: String, meta: String, verb| Hero {
                hero: None,
                capsule: None,
                cover: None,
                icon: None,
                logo: None,
                name,
                look: Some(WebLook { bg: SPOTIFY_BG, fg: SPOTIFY, mark: Some("spotify") }),
                glyph: None,
                meta,
                verb,
                options: false,
            };
            return Some(match key {
                "setup" => brand(
                    if m.configured { "Reconnect Spotify" } else { "Connect Spotify" }.into(),
                    m.error.clone().unwrap_or_else(|| "Your playlists and albums, played from the couch".into()),
                    "Set up",
                ),
                "account" => brand("Spotify".into(), if m.ready { format!("Signed in as {}", m.user.clone().unwrap_or_default()) } else { "Reading your library".into() }, "Open"),
                uri => {
                    let p = self.pick(uri)?;
                    let meta = if m.starting.as_deref() == Some(uri) { "Starting on this PC".to_owned() } else { pick_meta(p, m.user.as_deref()) };
                    Hero {
                        hero: None,
                        capsule: None,
                        cover: p.cover.clone(),
                        icon: None,
                        logo: None,
                        name: p.name.clone(),
                        look: None,
                        glyph: Some(if p.kind == crate::spotify::Kind::Liked { "heart" } else { "music" }),
                        meta,
                        verb: "Play",
                        options: true,
                    }
                }
            });
        }
        if let Some(key) = id.strip_prefix("dc:") {
            let soc = self.soc_by_key(key)?;
            let verb = soc_verb(&soc);
            let f = self.soc_face(&soc);
            return Some(Hero {
                hero: None,
                capsule: None,
                cover: None,
                icon: f.image,
                logo: None,
                name: f.name,
                look: f.brand.then_some(WebLook { bg: DISCORD, fg: WHITE, mark: Some("discord") }),
                glyph: f.glyph,
                meta: f.sub,
                verb,
                options: false,
            });
        }
        if let Some(h) = id.strip_prefix("run:").and_then(|h| h.parse::<isize>().ok()) {
            let run = self.run_of(h)?;
            if let Some(i) = run.item.filter(|&i| i < self.items.len()) {
                return Some(self.item_hero(i));
            }
            return Some(Hero {
                hero: None,
                capsule: None,
                cover: None,
                icon: run.icon.clone(),
                logo: None,
                name: self.run_name(h),
                look: None,
                glyph: Some("app"),
                meta: match self.run_page(h) {
                    p if p.is_empty() => "Running".into(),
                    p => format!("Running  ·  {p}"),
                },
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
            } else if let Some(cover) = &hero.cover {
                // A playlist or album: its own colour washes the stage, the cover sits right of
                // centre (as a streaming service's colour and mark do).
                if let Some(tone) = self.gfx.tone(cover) {
                    self.gfx.fill(Rect::new(0.0, 0.0, w, h), tone.alpha(0.3 * opacity), 0.0);
                }
                let s = 26.0 * r;
                let cy = h / 2.0 + r;
                let at = Rect::new(w - 12.0 * r - s, cy - s / 2.0, s, s).scaled(zoom);
                self.gfx.image_sized(cover, at, (s * 1.03, s * 1.03), Fit::Cover, r, opacity);
            } else if let Some(look) = hero.look {
                // A streaming service: its colour fills the stage, its mark sits right of centre.
                self.gfx.fill(Rect::new(0.0, 0.0, w, h), look.bg.alpha(0.6 * opacity), 0.0);
                let cy = h / 2.0 + r;
                match look.mark.and_then(icons::brand) {
                    Some((n, path)) => {
                        let s = 16.0 * r * zoom;
                        self.gfx.icon(n, path, Rect::new(w - 12.0 * r - s, cy - s / 2.0, s, s), look.fg.alpha(opacity), true, 0.0);
                    }
                    None => {
                        let tw = 40.0 * r;
                        self.gfx.text(&hero.name, w - 12.0 * r - tw, cy - 4.5 * r, tw, 7.0 * r, 700, look.fg.alpha(opacity), Align::Right, 0, -0.03);
                    }
                }
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
                if !drawn && let Some((n, svg)) = hero.glyph.and_then(icons::icon) {
                    let s = 11.0 * r;
                    self.gfx.icon(n, svg, Rect::new(w - 12.0 * r - s, cy - s / 2.0, s, s), T2.alpha(opacity), false, 1.0);
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
            if count > 0 {
                let (tw_title, _) = self.gfx.measure(&title, 1.375 * r, 500);
                self.gfx.text(&count.to_string(), pad + tw_title + 0.75 * r, y0 + 0.2 * r, 6.0 * r, 1.125 * r, 400, T3, Align::Left, 0, 0.0);
            }
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
        if drawn || it.is_some_and(|it| self.web_face(it, rect, radius, 1.0)) {
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

    /// A streaming service's face (tile, launch screen): its colour with its mark or wordmark
    /// centred. False for anything that is not a streaming service.
    fn web_face(&mut self, it: &Item, rect: Rect, radius: f32, alpha: f32) -> bool {
        let Some(look) = web_look(&it.id) else { return false };
        let r = self.rem();
        self.gfx.fill(rect, look.bg.alpha(alpha), radius);
        self.gfx.stroke(rect.inset(0.5), LINE.alpha(0.08 * alpha), radius, 1.0);
        match look.mark.and_then(icons::brand) {
            Some((n, path)) => {
                let s = rect.w * 0.44;
                self.gfx.icon(n, path, Rect::new(rect.x + (rect.w - s) / 2.0, rect.y + (rect.h - s) / 2.0, s, s), look.fg.alpha(alpha), true, 0.0);
            }
            None => {
                let size = (rect.w * 0.17).max(r);
                let lh = self.gfx.text_height(&it.name, size, 700, rect.w - 2.0 * r, 2);
                self.gfx.text(&it.name, rect.x + r, rect.y + (rect.h - lh) / 2.0, rect.w - 2.0 * r, size, 700, look.fg.alpha(alpha), Align::Center, 2, -0.02);
            }
        }
        true
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
            Tile::Social(s) => self.social_tile(s, rect, base, focused),
            Tile::Music(m) => self.music_tile(m, rect, base, focused),
        }
    }

    /// A Spotify tile: the cover, square, with the name and what it is below (as music apps
    /// show them); Spotify's own colour and mark for the account and Connect.
    fn music_tile(&mut self, m: &Mu, rect: Rect, base: (f32, f32), focused: bool) {
        let r = self.rem();
        let radius = 0.75 * r;
        let w = rect.w - 2.0 * r;
        let sub_y = rect.bottom() - 1.25 * r - 0.95 * r * 1.3;
        let (name, sub, cover, glyph) = match m {
            Mu::Play(uri) => {
                let Some(p) = self.pick(uri) else { return };
                let sub = match p.kind {
                    crate::spotify::Kind::Album => p.by.clone(),
                    _ => p.songs.map(crate::spotify::songs_text).unwrap_or_default(),
                };
                (p.name.clone(), sub, p.cover.clone(), if p.kind == crate::spotify::Kind::Liked { "heart" } else { "music" })
            }
            Mu::Setup | Mu::Account => {
                self.gfx.fill(rect, SPOTIFY_BG, radius);
                self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
                if let Some((n, path)) = icons::brand("spotify") {
                    let s = rect.w * 0.36;
                    self.gfx.icon(n, path, Rect::new(rect.x + (rect.w - s) / 2.0, rect.y + rect.h * 0.2, s, s), SPOTIFY, true, 0.0);
                }
                let (name, sub) = match m {
                    Mu::Setup => (if self.music.configured { "Reconnect Spotify" } else { "Connect Spotify" }.to_owned(), self.music.error.clone().unwrap_or_else(|| "Play your music from here".into())),
                    _ => ("Spotify".to_owned(), if self.music.ready { self.music.user.clone().unwrap_or_default() } else { "Reading your library".into() }),
                };
                self.gfx.text(&sub, rect.x + r, sub_y, w, 0.95 * r, 400, WHITE.alpha(0.7), Align::Center, 0, 0.0);
                let nh = self.gfx.text_height(&name, 1.125 * r, 500, w, 2);
                self.gfx.text(&name, rect.x + r, sub_y - 0.35 * r - nh, w, 1.125 * r, 500, WHITE, Align::Center, 2, 0.0);
                return;
            }
        };
        self.gfx.fill(rect, CARD, radius);
        self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
        let art = Rect::new(rect.x + 0.75 * r, rect.y + 0.75 * r, rect.w - 1.5 * r, rect.w - 1.5 * r);
        self.gfx.fill(art, DEEP, 0.5 * r);
        let k = base.0 / rect.w.max(1.0);
        if cover.as_ref().and_then(|c| self.gfx.image_sized(c, art, (art.w * k, art.h * k), Fit::Cover, 0.5 * r, 1.0)).is_none()
            && let Some((n, svg)) = icons::icon(glyph)
        {
            let g = art.w * 0.34;
            self.gfx.icon(n, svg, Rect::new(art.x + (art.w - g) / 2.0, art.y + (art.h - g) / 2.0, g, g), T2, false, 1.5);
        }
        self.gfx.text(&sub, rect.x + r, sub_y, w, 0.95 * r, 400, T3, Align::Left, 0, 0.0);
        let nh = self.gfx.text_height(&name, 1.125 * r, 500, w, 2);
        self.gfx.text(&name, rect.x + r, sub_y - 0.35 * r - nh, w, 1.125 * r, 500, if focused { T1 } else { T2 }, Align::Left, 2, 0.0);
    }

    /// A Discord tile: Discord's colour and mark for your channel, the account and setup;
    /// otherwise a card with an avatar, server icon or line icon. Amber marks a state that is
    /// on (muted, deafened).
    fn social_tile(&mut self, s: &Soc, rect: Rect, base: (f32, f32), focused: bool) {
        let r = self.rem();
        let radius = 0.75 * r;
        let f = self.soc_face(s);
        let (name_c, sub_c) = if f.brand {
            self.gfx.fill(rect, DISCORD, radius);
            if let Some((n, path)) = icons::brand("discord") {
                let size = rect.w * 0.36;
                self.gfx.icon(n, path, Rect::new(rect.x + (rect.w - size) / 2.0, rect.y + rect.h * 0.2, size, size), WHITE, true, 0.0);
            }
            (WHITE, WHITE.alpha(0.78))
        } else {
            self.gfx.fill(rect, CARD, radius);
            self.gfx.stroke(rect.inset(0.5), if f.on { ACCENT.alpha(0.7) } else { LINE }, radius, 1.0);
            let s = 4.5 * r;
            let k = base.0 / rect.w.max(1.0);
            let img = Rect::new(rect.x + (rect.w - s) / 2.0, rect.y + 3.0 * r, s, s);
            let drawn = f.image.as_ref().is_some_and(|p| self.gfx.image_sized(p, img, (s * k, s * k), Fit::Cover, s / 2.0, 1.0).is_some());
            if !drawn && let Some((n, svg)) = f.glyph.and_then(icons::icon) {
                let g = 3.5 * r;
                self.gfx.icon(n, svg, Rect::new(rect.x + (rect.w - g) / 2.0, rect.y + 3.5 * r, g, g), if f.on { ACCENT } else { T2 }, false, 1.5);
            }
            (if focused { T1 } else { T2 }, if f.on { ACCENT } else { T3 })
        };
        let w = rect.w - 2.0 * r;
        let sub_y = rect.bottom() - 1.25 * r - 0.95 * r * 1.3;
        self.gfx.text(&f.sub, rect.x + r, sub_y, w, 0.95 * r, 400, sub_c, Align::Center, 0, 0.0);
        let nh = self.gfx.text_height(&f.name, 1.125 * r, 500, w, 2);
        self.gfx.text(&f.name, rect.x + r, sub_y - 0.35 * r - nh, w, 1.125 * r, 500, name_c, Align::Center, 2, 0.0);
    }

    /// The Discord tile shown with this id (`dc:` + `Soc::key`).
    fn soc_by_key(&self, key: &str) -> Option<Soc> {
        self.rows.iter().flat_map(|r| &r.tiles).find_map(|t| match t {
            Tile::Social(s) if s.key() == key => Some(s.clone()),
            _ => None,
        })
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
            Some(Tile::Social(s)) => (soc_verb(s), false),
            Some(Tile::Music(Mu::Play(_))) => ("Play", true),
            Some(Tile::Music(Mu::Setup)) => ("Set up", false),
            Some(Tile::Music(Mu::Account)) => ("Open", false),
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
        // Right: what works everywhere. Volume, and on Home the PS button's quick menu.
        let volume_x = self.volume_hint(cy, 1.0);
        if self.tab == "home" && self.input.ps_home {
            let q = "Quick menu";
            let qw = self.gfx.measure(q, 1.125 * r, 400).0;
            let kw = self.key_width(Key::Home);
            self.hint(Key::Home, q, volume_x - 2.25 * r - qw - 0.75 * r - kw, cy, T3);
        }
    }

    /// "L2 R2 Volume" at the right edge, centred on `cy`. Returns where it starts.
    fn volume_hint(&mut self, cy: f32, a: f32) -> f32 {
        let r = self.rem();
        let label = "Volume";
        let lw = self.gfx.measure(label, 1.125 * r, 400).0;
        let k2 = self.key_width(Key::R2);
        let k1 = self.key_width(Key::L2);
        let start = self.gfx.width - 4.0 * r - lw - 0.75 * r - k2 - 0.5 * r - k1;
        let mut rx = start;
        rx += self.keycap(Key::L2, rx, cy) + 0.5 * r;
        rx += self.keycap(Key::R2, rx, cy) + 0.75 * r;
        self.gfx.text(label, rx, cy - 0.73 * r, lw + 4.0, 1.125 * r, 400, T3.alpha(a), Align::Left, 0, 0.0);
        start
    }

    // ---------- Quick menu ----------

    /// The quick menu (the PS button on Home): a bedside clock with the date, when the PC will
    /// sleep and the controller's charge, then the controls people reach for from the couch.
    fn draw_quick(&mut self, focus: Ctl, at: f64, now: f64) {
        let r = self.rem();
        let (w, h) = (self.gfx.width, self.gfx.height);
        let p = if self.motion() > 0.0 { ease(((now - at) / 320.0).clamp(0.0, 1.0)) as f32 } else { 1.0 };
        self.gfx.fill(Rect::new(0.0, 0.0, w, h), SCRIM.alpha(p), 0.0);
        // The menu takes the shelf's place; the art stays above it.
        let ph = h * 0.42;
        let panel = Rect::new(0.0, h - ph + (1.0 - p) * 3.0 * r, w, ph);
        self.gfx.fill(panel, DEEP, 0.0);
        self.gfx.fill(Rect::new(0.0, panel.y, w, 1.0), LINE, 0.0);
        let x0 = 4.0 * r;
        // The time, large, with the date on its baseline.
        let ty = panel.y + 3.0 * r;
        let big = 3.5 * r;
        let small = 1.375 * r;
        let baseline = |size: f32| ty + big * 0.96 - size * 0.96;
        let clock = clock_text();
        let (cw, _) = self.gfx.measure(&clock, big, 500);
        self.gfx.text(&clock, x0, ty, cw + 8.0, big, 500, T1.alpha(p), Align::Left, 0, -0.02);
        self.gfx.text(&date_text(), x0 + cw + 1.5 * r, baseline(small), 40.0 * r, small, 400, T2.alpha(p), Align::Left, 0, 0.0);
        // Right, on the same line: the controller's charge, and when the PC sleeps.
        let mut rx = w - x0;
        let sy = baseline(small);
        let icy = sy + small * 0.62;
        let info = self.input.info();
        if let (true, Some(b)) = (info.connected, info.battery) {
            let text = format!("{b}%");
            let (tw, _) = self.gfx.measure(&text, small, 400);
            rx -= tw;
            let low = b <= 20 && !info.charging;
            self.gfx.text(&text, rx, sy, tw + 4.0, small, 400, (if low { ACCENT } else { T2 }).alpha(p), Align::Left, 0, 0.0);
            if info.charging
                && let Some((n, svg)) = icons::icon("zap")
            {
                rx -= 1.75 * r;
                self.gfx.icon(n, svg, Rect::new(rx, icy - 0.625 * r, 1.25 * r, 1.25 * r), T2.alpha(p), false, 1.5);
            }
            if let Some((n, path)) = icons::brand("playstation") {
                rx -= 2.0 * r;
                self.gfx.icon(n, path, Rect::new(rx, icy - 0.7 * r, 1.4 * r, 1.4 * r), T3.alpha(p), true, 0.0);
            }
            rx -= 2.5 * r;
        }
        if let Some(ms) = self.sleep_left() {
            let text = format!("Sleeps at {}", clock_in((ms / 60_000.0).round() as u32));
            let (tw, _) = self.gfx.measure(&text, small, 400);
            rx -= tw;
            self.gfx.text(&text, rx, sy, tw + 4.0, small, 400, T2.alpha(p), Align::Left, 0, 0.0);
            if let Some((n, svg)) = icons::icon("sleep") {
                rx -= 2.0 * r;
                self.gfx.icon(n, svg, Rect::new(rx, icy - 0.7 * r, 1.4 * r, 1.4 * r), ACCENT.alpha(p), false, 1.5);
            }
        }
        // The controls: as wide as what they say, scrolled so the focused one shows.
        let ctls = self.quick_ctls();
        let faces: Vec<CtlFace> = ctls.iter().map(|&c| self.ctl_face(c)).collect();
        let gap = 1.25 * r;
        let widths: Vec<f32> = ctls
            .iter()
            .zip(&faces)
            .map(|(c, f)| {
                let lw = self.gfx.measure(f.label, 1.125 * r, 400).0.max(self.gfx.measure(&f.app, 1.125 * r, 400).0);
                let vw = self.gfx.measure(&f.value, 1.375 * r, 500).0;
                match c {
                    // The art beside the track; long titles end in an ellipsis.
                    Ctl::Music => (lw.max(vw) + 11.25 * r).clamp(22.0 * r, 28.0 * r),
                    Ctl::Output => (lw.max(vw) + 3.25 * r).clamp(15.0 * r, 22.0 * r),
                    _ => (lw.max(vw) + 3.25 * r).clamp(10.5 * r, 22.0 * r),
                }
            })
            .collect();
        let fi = ctls.iter().position(|c| *c == focus).unwrap_or(0);
        let total = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0);
        let avail = w - 2.0 * x0;
        let before: f32 = widths[..fi].iter().sum::<f32>() + gap * fi as f32;
        let off = if total <= avail { 0.0 } else { (before + widths[fi] - avail).max(0.0).min(total - avail) };
        let y = ty + 7.25 * r;
        let mut x = x0 - off;
        for (i, f) in faces.iter().enumerate() {
            let rect = Rect::new(x, y, widths[i], 10.0 * r);
            if ctls[i] == Ctl::Music {
                self.playing_tile(f, rect, i == fi, p);
            } else {
                self.ctl_tile(f, rect, i == fi, p);
            }
            x += widths[i] + gap;
        }
        let hy = panel.bottom() - 2.75 * r;
        let mut hx = self.hint(Key::Cross, faces[fi].verb, x0, hy, T2.alpha(p));
        if ctls[fi] == Ctl::Music {
            hx = self.hint(Key::L1, "Previous", hx, hy, T2.alpha(p));
            hx = self.hint(Key::R1, "Next", hx, hy, T2.alpha(p));
        }
        hx = self.hint(Key::Options, "Settings", hx, hy, T2.alpha(p));
        self.hint(Key::Circle, "Close", hx, hy, T2.alpha(p));
        self.volume_hint(hy, p);
    }

    /// What is playing: its art, whether it plays (the amber dot, as on Running cards), the
    /// artist and the track.
    fn playing_tile(&mut self, f: &CtlFace, rect: Rect, focused: bool, a: f32) {
        let r = self.rem();
        let radius = 0.75 * r;
        if focused {
            self.gfx.stroke(rect.inset(-4.0 - 1.5), T1.alpha(a), radius + 5.5, 3.0);
        }
        self.gfx.fill(rect, if focused { RAISED } else { CARD }, radius);
        if !focused {
            self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
        }
        let art = Rect::new(rect.x + 1.5 * r, rect.y + 1.5 * r, rect.h - 3.0 * r, rect.h - 3.0 * r);
        self.gfx.fill(art, DEEP, 0.4 * r);
        // The art fades in over the empty square; without art, a note stands in.
        if f.art.as_ref().and_then(|p| self.gfx.image(p, art, Fit::Cover, 0.4 * r, a)).is_none()
            && let Some((n, svg)) = icons::icon("music")
        {
            let s = 2.5 * r;
            self.gfx.icon(n, svg, Rect::new(art.x + (art.w - s) / 2.0, art.y + (art.h - s) / 2.0, s, s), T3.alpha(a), false, 1.5);
        }
        let tx = art.right() + 1.25 * r;
        let tw = rect.right() - 1.5 * r - tx;
        let playing = f.verb == "Pause";
        let mut lx = tx;
        if playing {
            self.gfx.fill(Rect::new(tx, rect.y + 1.5 * r + 0.45 * r, 0.5 * r, 0.5 * r), ACCENT.alpha(a), 0.25 * r);
            lx += 0.9 * r;
        }
        self.gfx.text(f.label, lx, rect.y + 1.5 * r, tw, 1.125 * r, 400, (if playing { T2 } else { T3 }).alpha(a), Align::Left, 0, 0.0);
        let vy = rect.bottom() - 1.4 * r - 1.375 * r * 1.3;
        self.gfx.text(&f.app, tx, vy - 1.125 * r * 1.3 - 0.2 * r, tw, 1.125 * r, 400, (if focused { T2 } else { T3 }).alpha(a), Align::Left, 0, 0.0);
        self.gfx.text(&f.value, tx, vy, tw, 1.375 * r, 500, T1.alpha(a), Align::Left, 0, 0.0);
    }

    /// One quick menu control: its icon, then what it is and its state. A state that is on
    /// (muted, a timer running) is amber; the volume shows its level as a bar.
    fn ctl_tile(&mut self, f: &CtlFace, rect: Rect, focused: bool, a: f32) {
        let r = self.rem();
        let radius = 0.75 * r;
        if focused {
            self.gfx.stroke(rect.inset(-4.0 - 1.5), T1.alpha(a), radius + 5.5, 3.0);
        }
        self.gfx.fill(rect, if focused { RAISED } else { CARD }, radius);
        if !focused {
            self.gfx.stroke(rect.inset(0.5), LINE, radius, 1.0);
        }
        let ic = if f.on { ACCENT } else if focused { T1 } else { T2 };
        let icon_rect = Rect::new(rect.x + 1.5 * r, rect.y + 1.5 * r, 1.75 * r, 1.75 * r);
        if let Some((n, svg)) = icons::icon(f.icon) {
            self.gfx.icon(n, svg, icon_rect, ic.alpha(a), false, 1.5);
        }
        if let Some(level) = f.level {
            let bar = Rect::new(icon_rect.right() + 1.25 * r, icon_rect.y + icon_rect.h / 2.0 - 0.1875 * r, rect.right() - icon_rect.right() - 2.75 * r, 0.375 * r);
            self.gfx.fill(bar, T1.alpha(0.1 * a), 0.1875 * r);
            self.gfx.fill(Rect::new(bar.x, bar.y, bar.w * level.clamp(0.0, 1.0), bar.h), ACCENT.alpha(a), 0.1875 * r);
        }
        let tw = rect.w - 3.0 * r;
        let vy = rect.bottom() - 1.4 * r - 1.375 * r * 1.3;
        if f.value.is_empty() {
            self.gfx.text(f.label, rect.x + 1.5 * r, vy, tw, 1.375 * r, 500, (if focused { T1 } else { T2 }).alpha(a), Align::Left, 0, 0.0);
        } else {
            self.gfx.text(f.label, rect.x + 1.5 * r, vy - 1.125 * r * 1.3 - 0.2 * r, tw, 1.125 * r, 400, (if focused { T2 } else { T3 }).alpha(a), Align::Left, 0, 0.0);
            self.gfx.text(&f.value, rect.x + 1.5 * r, vy, tw, 1.375 * r, 500, (if f.on { ACCENT } else { T1 }).alpha(a), Align::Left, 0, 0.0);
        }
    }

    // ---------- Layers ----------

    /// An option's height: one line, or two with a detail line (setup steps).
    fn opt_height(&mut self, o: &Opt, width: f32) -> f32 {
        let r = self.rem();
        match &o.detail {
            Some(d) => 3.1 * r + self.gfx.text_height(d, r, 400, width - 4.5 * r, 2) + 0.9 * r,
            None => 3.75 * r,
        }
    }

    fn draw_options(&mut self, opts: &[Opt], focus: usize, x: f32, mut y: f32, width: f32) {
        let r = self.rem();
        for (i, o) in opts.iter().enumerate() {
            let oh = self.opt_height(o, width);
            let rect = Rect::new(x, y, width, oh);
            let on = i == focus;
            if on {
                self.gfx.fill(rect, RAISED, 10.0);
                self.gfx.stroke(rect.inset(0.5), LINE, 10.0, 1.0);
            }
            let mut tx = x + r;
            if let Some((n, svg)) = o.icon.and_then(icons::icon) {
                self.gfx.icon(n, svg, Rect::new(tx, y + 1.125 * r, 1.5 * r, 1.5 * r), if on { T1 } else { T3 }, false, 1.5);
                tx += 2.5 * r;
            }
            let room = if o.value.is_some() { 13.0 * r } else if o.switch.is_some() { 5.5 * r } else { r };
            self.gfx.text(&o.label, tx, y + 0.98 * r, width - (tx - x) - room, 1.375 * r, 400, if on { T1 } else { T2 }, Align::Left, 0, 0.0);
            if let Some(v) = &o.value {
                self.gfx.text(v, x + width - 13.0 * r, y + 1.06 * r, 12.0 * r, 1.25 * r, 400, T3, Align::Right, 0, 0.0);
            }
            if let Some(sw) = o.switch {
                let s = Rect::new(x + width - r - 3.25 * r, y + 0.875 * r, 3.25 * r, 2.0 * r);
                self.gfx.fill(s, if sw { ACCENT } else { RAISED }, r);
                if !sw {
                    self.gfx.stroke(s.inset(0.5), LINE, r, 1.0);
                }
                let k = Rect::new(s.x + 0.25 * r + if sw { 1.25 * r } else { 0.0 }, s.y + 0.25 * r, 1.5 * r, 1.5 * r);
                self.gfx.fill(k, if sw { INK } else { T2 }, 0.75 * r);
            }
            if let Some(d) = &o.detail {
                self.gfx.text(d, tx, y + 3.1 * r, width - (tx - x) - r, r, 400, T3, Align::Left, 2, 0.0);
            }
            y += oh + 0.25 * r;
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
                    // Long lists scroll by whole options so the focused one stays above the hints
                    // and no option shows cut in half at either edge.
                    let start = 3.0 * r + th + 3.5 * r;
                    let avail = h - 6.0 * r - start;
                    let width = panel.w - 4.0 * r;
                    let heights: Vec<f32> = opts.iter().map(|o| self.opt_height(o, width) + 0.25 * r).collect();
                    let upto: f32 = heights.iter().take(focus + 1).sum();
                    let (mut off, mut first) = (0.0, 0);
                    while upto - off > avail + 0.5 && first < focus {
                        off += heights[first];
                        first += 1;
                    }
                    let mut shown = 0.0;
                    for hgt in &heights[first..] {
                        if shown + hgt > avail + 0.5 {
                            break;
                        }
                        shown += hgt;
                    }
                    self.gfx.clip(Rect::new(panel.x, start - 0.5 * r, panel.w, shown + 0.5 * r));
                    self.draw_options(&opts, focus, x, start - off, width);
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
                    let ph = 2.5 * r + tth + 0.75 * r + bh + 2.0 * r + opts.len() as f32 * 4.0 * r + 1.75 * r;
                    let p = Rect::new((w - pw) / 2.0, (h - ph) / 2.0, pw, ph);
                    self.gfx.fill(p, CARD, 20.0);
                    self.gfx.stroke(p.inset(0.5), LINE, 20.0, 1.0);
                    self.gfx.text(&title, p.x + 2.5 * r, p.y + 2.5 * r, tw, 2.0 * r, 500, T1, Align::Left, 2, -0.02);
                    let by = p.y + 2.5 * r + tth + 0.75 * r;
                    self.gfx.text(&body, p.x + 2.5 * r, by, tw, 1.375 * r, 400, T2, Align::Left, 6, 0.0);
                    self.draw_options(&opts, focus, p.x + 1.5 * r, by + bh + 2.0 * r, pw - 3.0 * r);
                    // Where the hint bar is: what ✕ and ○ do here.
                    let hy = h - 2.5 * r;
                    let hx = self.hint(Key::Cross, "Select", 4.0 * r, hy, T2);
                    let back = if opts.iter().any(|o| matches!(o.act, Act::StayAwake)) { "Stay awake" } else { "Back" };
                    self.hint(Key::Circle, back, hx, hy, T2);
                }
                Layer::Launch { item, at } => self.draw_launch(item, at, now),
                Layer::Keyboard(kb) => self.draw_keyboard(&kb),
                Layer::Welcome { opts, focus, at } => self.draw_welcome(&opts, focus, at, now),
                Layer::Quick { focus, at } => self.draw_quick(focus, at, now),
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
        if !drawn && !self.web_face(&it, p.scaled(grow), 12.0, t) {
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
        let placeholder = match kb.purpose {
            KbPurpose::Search => "Search games and apps",
            KbPurpose::DcId => "Client ID",
            KbPurpose::DcSecret => "Client Secret",
            KbPurpose::SpId => "Spotify Client ID",
            KbPurpose::Rename(_) => "Name",
        };
        // A secret is not shown on the TV.
        let shown = if kb.purpose == KbPurpose::DcSecret { "\u{2022}".repeat(kb.text.chars().count()) } else { kb.text.clone() };
        let caret_x = if kb.text.is_empty() {
            self.gfx.text(placeholder, tx + 0.4 * r, ty, room, size, 400, T3, Align::Left, 0, 0.0);
            tx
        } else {
            self.gfx.text(&shown, tx, ty, room, size, 400, T1, Align::Left, 0, 0.0);
            tx + self.gfx.measure(&shown, size, 400).0.min(room)
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
        let body = format!("Your games, apps and music in one place, made for a controller. {} in any game to come back here, and again for the quick menu.", self.home_press());
        let bh = self.gfx.text_height(&body, 1.5 * r, 400, width, 4);
        self.gfx.text(&body, x, y + d2, width, 1.5 * r, 400, T2.alpha(p2), Align::Left, 4, 0.0);
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
        // Beside a sheet, not over its title.
        let right = if matches!(self.layers.last(), Some(Layer::Sheet { .. })) { self.gfx.width - 38.0 * r } else { self.gfx.width - 4.0 * r };
        let rect = Rect::new(right - w, 6.0 * r + (1.0 - p) * 8.0, w, 2.0 * r + 1.5 * r + bh);
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

/// A System tile's line under its name. `press`: how to come back ("Press the PS button").
fn system_desc(key: &str, press: &str) -> String {
    if key == "desktop" {
        return format!("Shows the {}desktop. {press} to come back", if cfg!(windows) { "Windows " } else { "" });
    }
    match key {
        "sleep" => "The PC sleeps until you wake it",
        "restart" => "Restarts the PC",
        "shutdown" => "Turns the PC off",
        "quit" => "Closes Sofa completely",
        "bigpicture" => "Opens Steam in Big Picture mode",
        "settings" if cfg!(windows) => "Start with Windows, buttons, browser and sounds",
        "settings" => "Start when you sign in, buttons, browser and sounds",
        "rescan" => "Looks for newly installed games and apps",
        "display" => "Resolution, HDR and scaling",
        "sound" => "TV, headphones or the controller",
        "bluetooth" => "Pair a controller or headphones",
        _ => "",
    }
    .into()
}

/// Steam's total play time: "45 min in total", "1 hour in total", "73 hours in total".
fn playtime_text(min: u32) -> String {
    match (min, (min + 30) / 60) {
        (0..=59, _) => format!("{min} min in total"),
        (_, 1) => "1 hour in total".into(),
        (_, h) => format!("{h} hours in total"),
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

/// Today's date in the person's own long format ("Friday, November 27, 2026").
#[cfg(windows)]
fn date_text() -> String {
    use windows::Win32::Globalization::{DATE_LONGDATE, GetDateFormatEx};
    let mut buf = [0u16; 128];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetDateFormatEx(windows::core::PCWSTR::null(), DATE_LONGDATE, None, windows::core::PCWSTR::null(), Some(&mut buf), windows::core::PCWSTR::null()) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// The time `minutes` from now, in the person's own format ("11:42 PM").
#[cfg(windows)]
fn clock_in(minutes: u32) -> String {
    use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain call.
    let mut t = unsafe { GetLocalTime() };
    let total = (u32::from(t.wHour) * 60 + u32::from(t.wMinute) + minutes) % (24 * 60);
    (t.wHour, t.wMinute, t.wSecond) = ((total / 60) as u16, (total % 60) as u16, 0);
    let mut buf = [0u16; 64];
    // SAFETY: the time and the buffer are locals passed with their sizes.
    let n = unsafe { GetTimeFormatEx(windows::core::PCWSTR::null(), TIME_NOSECONDS, Some(&t), windows::core::PCWSTR::null(), Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

/// The time in the person's own format, without seconds.
#[cfg(windows)]
fn clock_text() -> String {
    use windows::Win32::Globalization::{GetTimeFormatEx, TIME_NOSECONDS};
    let mut buf = [0u16; 64];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetTimeFormatEx(windows::core::PCWSTR::null(), TIME_NOSECONDS, None, windows::core::PCWSTR::null(), Some(&mut buf)) };
    String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize])
}

#[cfg(target_os = "linux")]
use crate::linux::clock::{clock_in, clock_text, date_text};
