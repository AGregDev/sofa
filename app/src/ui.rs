//! Sofa's state and behaviour: what each button does on each screen. Drawing is in `view.rs`.
//! Layout and motion follow Sofa 1.x (`renderer/app.js`, `style.css`): rem = screen height /
//! 67.5, one easing curve, warm graphite with one amber accent.

use crate::gfx::{Color, Gfx};
use crate::input::{Action, Input};
use crate::library::{self, Item, Launch};
use crate::shell::{self, Shell, Window};
use crate::sound::{self, Sound};
use crate::store::Store;
use crate::{install, platform};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) const DEEP: Color = Color::hex(0x12110f);
pub(crate) const BG: Color = Color::hex(0x161513);
pub(crate) const CARD: Color = Color::hex(0x1c1a18);
pub(crate) const RAISED: Color = Color::hex(0x262421);
pub(crate) const T1: Color = Color::hex(0xefe9de);
pub(crate) const T2: Color = Color::hex(0xaaa398);
pub(crate) const T3: Color = Color::hex(0x8a847a);
pub(crate) const ACCENT: Color = Color::hex(0xf0a24b);
pub(crate) const INK: Color = Color::hex(0x1f1408);
pub(crate) const LINE: Color = Color(0.937, 0.914, 0.871, 0.08);

/// Messages background threads post to the window.
pub const WM_LIBRARY: u32 = 0x8000 + 5; // the Start menu scan or icon extraction finished
pub const WM_STEAM: u32 = 0x8000 + 6; // the Steam fix finished (wparam: result code)
pub const WM_ICON: u32 = 0x8000 + 8; // an icon for a Running card was saved

static SCANNING: AtomicBool = AtomicBool::new(false);
static JOBS: AtomicUsize = AtomicUsize::new(0);

/// Runs `sofa.exe --scan` as a short-lived child process at low priority: the Start menu
/// listing and program icons (`startmenu.rs`). Windows' icon code loads many libraries; in a
/// child they go away when it ends, instead of staying in Sofa's memory. Posts `done` after.
fn spawn_scan(cache: &std::path::Path, jobs: &[(String, PathBuf)], startmenu: bool, hwnd: isize, done: u32) -> bool {
    use std::os::windows::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return false };
    let file = cache.join(format!("jobs-{}-{}.json", std::process::id(), JOBS.fetch_add(1, Ordering::Relaxed)));
    let list: Vec<Value> = jobs.iter().map(|(e, p)| Value::Array(vec![Value::from(e.as_str()), Value::from(p.to_string_lossy().as_ref())])).collect();
    if std::fs::create_dir_all(cache).is_err() || std::fs::write(&file, Value::Array(list).to_string()).is_err() {
        return false;
    }
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--scan").arg(cache).arg("--jobs").arg(&file);
    if !startmenu {
        cmd.arg("--no-startmenu");
    }
    // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS: never in the way of a game.
    cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    let Ok(mut child) = cmd.spawn() else {
        let _ = std::fs::remove_file(&file);
        return false;
    };
    let _ = std::thread::Builder::new().name("scan".into()).spawn(move || {
        let _ = child.wait();
        if startmenu {
            SCANNING.store(false, Ordering::Release);
        }
        post(hwnd, done, 0);
    });
    true
}

/// `cubic-bezier(0.22, 1, 0.36, 1)`, the curve every transition in Sofa uses.
pub(crate) fn ease(p: f64) -> f64 {
    let p = p.clamp(0.0, 1.0);
    let bez = |t: f64, a: f64, b: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bez(mid, 0.22, 0.36) < p { lo = mid } else { hi = mid }
    }
    bez((lo + hi) / 2.0, 1.0, 1.0)
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Tween {
    pub from: f32,
    pub to: f32,
    pub start: f64,
    pub dur: f64,
}

impl Tween {
    pub fn at(v: f32) -> Tween {
        Tween { from: v, to: v, start: 0.0, dur: 0.0 }
    }
    pub fn value(&self, now: f64) -> f32 {
        if self.dur <= 0.0 || now >= self.start + self.dur {
            return self.to;
        }
        self.from + (self.to - self.from) * ease((now - self.start) / self.dur) as f32
    }
    pub fn go(&mut self, to: f32, now: f64, dur: f64) {
        if (self.to - to).abs() < 0.01 {
            return;
        }
        *self = Tween { from: self.value(now), to, start: now, dur };
    }
    pub fn busy(&self, now: f64) -> bool {
        self.dur > 0.0 && now < self.start + self.dur
    }
}

/// Something that was in front when the PS button was pressed: the Running card.
pub(crate) struct Run {
    pub win: Window,
    pub item: Option<usize>,
    pub icon: Option<PathBuf>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum Tile {
    Item(usize),
    Nav { cat: String, count: usize },
    /// A running app, by its window.
    Running(isize),
}

pub(crate) struct Row {
    pub key: String,
    pub title: String,
    pub count: usize,
    pub tiles: Vec<Tile>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Key {
    Cross,
    Circle,
    Square,
    Triangle,
    Options,
    L1,
    R1,
    L2,
    R2,
}

#[derive(Clone)]
pub(crate) enum Act {
    Favorite(usize),
    Move(usize, &'static str),
    Hide(usize, bool),
    Rename(usize),
    Resume(isize),
    CloseRun(isize),
    KillRun(isize),
    SwitchClose(isize, usize),
    SwitchBoth(usize),
    Toggle(&'static str),
    HomeButton,
    Layout,
    Browser,
    Rescan,
    Desktop,
    Quit,
    Power(&'static str),
    SteamFixNow,
    WelcomeDone,
    Close,
}

#[derive(Clone)]
pub(crate) struct Opt {
    pub label: String,
    pub icon: Option<&'static str>,
    pub value: Option<String>,
    pub switch: Option<bool>,
    pub act: Act,
}

impl Opt {
    fn new(label: impl Into<String>, act: Act) -> Opt {
        Opt { label: label.into(), icon: None, value: None, switch: None, act }
    }
    fn icon(mut self, i: &'static str) -> Opt {
        self.icon = Some(i);
        self
    }
    fn value(mut self, v: impl Into<String>) -> Opt {
        self.value = Some(v.into());
        self
    }
    fn switch(mut self, on: bool) -> Opt {
        self.switch = Some(on);
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SheetKind {
    Settings,
    Item,
    Run,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum KbPurpose {
    Search,
    Rename(usize),
}

/// The on-screen keyboard: 4 rows of 10 characters, then the bottom row.
pub(crate) const KB_ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm-.&"];
/// Bottom row: (key, first column, width in columns).
pub(crate) const KB_BOTTOM: [(&str, usize, usize); 6] = [("shift", 0, 1), ("space", 1, 4), ("erase", 5, 1), ("Paste", 6, 1), ("Clear", 7, 1), ("Done", 8, 2)];
/// Search results beside the keyboard: 2 columns, 4 rows.
pub(crate) const KB_RESULT_COLS: usize = 2;
const KB_RESULTS: usize = 8;

#[derive(Clone)]
pub(crate) struct Keyboard {
    pub text: String,
    pub purpose: KbPurpose,
    /// (row, column) on the keys; row 4 is the bottom row. `result` focus takes over when set.
    pub key: (usize, usize),
    pub result: Option<usize>,
    pub hits: Vec<usize>,
    pub shift: bool,
    /// Last input came from a physical keyboard (Enter then submits instead of pressing the
    /// focused on-screen key).
    pub typed: bool,
}

#[derive(Clone)]
pub(crate) enum Layer {
    Sheet { title: String, sub: String, opts: Vec<Opt>, focus: usize, kind: SheetKind },
    Dialog { title: String, body: String, opts: Vec<Opt>, focus: usize },
    Launch { item: usize, at: f64 },
    Keyboard(Keyboard),
    Welcome { opts: Vec<Opt>, focus: usize, at: f64 },
}

pub struct App {
    pub gfx: Gfx,
    pub input: Input,
    pub shell: Shell,
    pub(crate) store: Store,
    pub(crate) data: PathBuf,
    pub(crate) cache: PathBuf,
    pub(crate) items: Vec<Item>,
    pub(crate) running: Vec<Run>,
    pub(crate) tab: String,
    pub(crate) rows: Vec<Row>,
    pub(crate) grid: bool,
    pub(crate) focus: (usize, usize),
    tab_focus: HashMap<String, (usize, usize)>,
    pub(crate) scroll_x: Vec<Tween>,
    pub(crate) scroll_y: Tween,
    pub(crate) tab_line: (Tween, Tween),
    pub(crate) scale: HashMap<String, Tween>,
    pub(crate) hero: Option<(String, f64)>,
    pub(crate) hero_prev: Option<(String, f64)>,
    hero_pending: Option<(String, f64)>,
    pub(crate) layers: Vec<Layer>,
    pub(crate) toast: Option<(String, String, f64)>,
    pub focused: bool,
    pub dirty: bool,
    pub qa: bool,
    pub boot_at: f64,
    pub last_home: f64,
    pub draw_errors: u32,
    minute: u16,
    pub(crate) version: &'static str,
    pub(crate) icon_png: PathBuf,
    pub(crate) welcome_jpg: PathBuf,
    steam_tip: bool,
    unfocused_at: Option<f64>,
    trimmed: bool,
    prune_at: Option<f64>,
    pads: (bool, bool, bool),
    pending_rescan: bool,
    scanned_at: f64,
}

const TABS: [(&str, &str); 5] = [("home", "Home"), ("games", "Games"), ("media", "Media"), ("apps", "Apps"), ("system", "System")];

pub(crate) fn cat_label(c: &str) -> &'static str {
    match c {
        "games" => "Games",
        "media" => "Media",
        "hidden" => "Hidden",
        "system" => "System",
        _ => "Apps",
    }
}

pub(crate) fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(gfx: Gfx, input: Input, shell: Shell, data: PathBuf, icon_png: PathBuf, welcome_jpg: PathBuf, qa: bool, now: f64) -> App {
        let store = Store::open(&data);
        let cache = data.join("cache");
        let mut app = App {
            gfx,
            input,
            shell,
            store,
            data,
            cache,
            items: Vec::new(),
            running: Vec::new(),
            tab: "home".into(),
            rows: Vec::new(),
            grid: false,
            focus: (0, 0),
            tab_focus: HashMap::new(),
            scroll_x: Vec::new(),
            scroll_y: Tween::at(0.0),
            tab_line: (Tween::at(0.0), Tween::at(0.0)),
            scale: HashMap::new(),
            hero: None,
            hero_prev: None,
            hero_pending: None,
            layers: Vec::new(),
            toast: None,
            focused: true,
            dirty: true,
            qa,
            boot_at: now,
            last_home: -1e9,
            draw_errors: 0,
            minute: 99,
            version: env!("CARGO_PKG_VERSION"),
            icon_png,
            welcome_jpg,
            steam_tip: false,
            unfocused_at: None,
            trimmed: false,
            prune_at: None,
            pads: (false, false, false),
            pending_rescan: false,
            scanned_at: now,
        };
        app.apply_settings();
        app.rescan();
        if !app.store.top_flag("setupDone") {
            app.welcome(now);
        }
        app
    }

    /// Background work at start: the Start menu scan, and Steam's PS-button setting (changed
    /// silently when Steam is closed).
    pub fn startup(&mut self, now: f64) {
        self.scan_in_background(now);
        if self.store.top_flag("setupDone") && self.store.flag("steamGuide") {
            self.steam_fix(false);
        }
    }

    pub(crate) fn rem(&self) -> f32 {
        self.gfx.height / 67.5
    }

    pub(crate) fn motion(&self) -> f64 {
        if self.store.text("motion") == "reduced" { 0.0 } else { 1.0 }
    }

    fn apply_settings(&mut self) {
        let layout = self.store.text("controllerLayout").to_owned();
        self.input.set_layout(if layout.is_empty() { "auto" } else { &layout });
        self.input.ps_home = self.store.text("homeButton") != "combo";
        self.shell.guard = !self.qa && self.store.flag("keepInFront");
        self.shell.set_top(self.focused);
        sound::ENABLED.store(self.store.flag("sounds") && !self.qa, Ordering::Relaxed);
    }

    /// Reads the library again from Steam and the caches, keeping focus on the same tile.
    pub fn rescan(&mut self) {
        let browser = platform::chosen_browser(self.store.text("browser")).map(|b| b.exe);
        let (_, items) = library::build(&self.cache, &self.store.map("overrides"), &self.store.map("recent"), browser);
        for r in &mut self.running {
            r.item = library::identify(&items, &r.win.exe);
        }
        self.items = items;
        self.pending_rescan = false;
        self.rebuild(true);
    }

    /// Layers hold item positions, so a new library waits until they close.
    fn layers_hold_items(&self) -> bool {
        self.layers.iter().any(|l| !matches!(l, Layer::Welcome { .. }))
    }

    /// The background scan finished: new Start menu apps or icons.
    pub fn library_changed(&mut self) {
        self.gfx.retry_failed();
        if self.layers_hold_items() {
            self.pending_rescan = true;
        } else {
            self.rescan();
        }
    }

    /// Lists the Start menu and extracts missing program icons in the background, then
    /// posts WM_LIBRARY. One scan at a time.
    pub fn scan_in_background(&mut self, now: f64) {
        if SCANNING.swap(true, Ordering::AcqRel) {
            return;
        }
        self.scanned_at = now;
        let icons = library::missing_icons(&self.items, &self.cache);
        if !spawn_scan(&self.cache, &icons, true, self.shell.sofa, WM_LIBRARY) {
            SCANNING.store(false, Ordering::Release);
        }
    }

    pub(crate) fn tabs(&self) -> Vec<(&'static str, &'static str)> {
        let mut t = TABS.to_vec();
        if self.store.flag("showHidden") {
            t.push(("hidden", "Hidden"));
        }
        t
    }

    fn sorted(&self, cat: &str) -> Vec<usize> {
        let mut v: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].category == cat).collect();
        if cat != "system" {
            v.sort_by(|&a, &b| {
                let (x, y) = (&self.items[a], &self.items[b]);
                y.last_played.partial_cmp(&x.last_played).unwrap_or(std::cmp::Ordering::Equal).then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase()))
            });
        }
        v
    }

    /// Rebuilds the current tab's rows, keeping focus on the same tile where it still exists.
    pub(crate) fn rebuild(&mut self, keep: bool) {
        let keep_id = keep.then(|| self.focused_tile_id()).flatten();
        let mut rows = Vec::new();
        self.grid = self.tab != "home";
        if self.tab == "home" {
            let visible: Vec<usize> = (0..self.items.len()).filter(|&i| !matches!(self.items[i].category.as_str(), "hidden" | "system")).collect();
            let mut recent: Vec<usize> = visible.iter().copied().filter(|&i| self.items[i].last_played > 0.0).collect();
            recent.sort_by(|&a, &b| self.items[b].last_played.partial_cmp(&self.items[a].last_played).unwrap_or(std::cmp::Ordering::Equal));
            recent.truncate(10);
            let favs: Vec<usize> = visible.iter().copied().filter(|&i| self.items[i].favorite).collect();
            let mut push = |key: &str, title: &str, list: Vec<Tile>, count: usize| {
                if !list.is_empty() {
                    rows.push(Row { key: key.into(), title: title.into(), count, tiles: list });
                }
            };
            push("running", "Running", self.running.iter().map(|r| Tile::Running(r.win.hwnd)).collect(), self.running.len());
            push("recent", "Jump back in", recent.iter().map(|&i| Tile::Item(i)).collect(), recent.len());
            push("fav", "Favorites", favs.iter().map(|&i| Tile::Item(i)).collect(), favs.len());
            for cat in ["games", "media", "apps"] {
                let all = self.sorted(cat);
                let n = all.len();
                let mut tiles: Vec<Tile> = all.iter().take(14).map(|&i| Tile::Item(i)).collect();
                if n > 14 {
                    tiles.push(Tile::Nav { cat: cat.into(), count: n });
                }
                push(cat, cat_label(cat), tiles, n);
            }
        } else {
            let list = self.sorted(&self.tab);
            rows.push(Row { key: self.tab.clone(), title: cat_label(&self.tab).into(), count: list.len(), tiles: list.into_iter().map(Tile::Item).collect() });
            rows.retain(|r| !r.tiles.is_empty());
        }
        self.rows = rows;
        self.scroll_x = self.rows.iter().map(|_| Tween::at(0.0)).collect();
        let found = keep_id.and_then(|id| self.rows.iter().enumerate().find_map(|(r, row)| row.tiles.iter().position(|t| self.tile_id(t) == id).map(|c| (r, c))));
        self.focus = found.or_else(|| self.tab_focus.get(&self.tab).copied()).unwrap_or((0, 0));
        self.clamp_focus();
        self.scroll_y = Tween::at(0.0);
        self.reveal(0.0, false);
        match self.focused_tile_id() {
            Some(id) if self.hero.is_none() => self.show_hero(id, 0.0),
            Some(id) => self.hero_pending = Some((id, 0.0)),
            None => {
                if self.hero.is_some() {
                    self.hero_prev = self.hero.take();
                }
            }
        }
        self.dirty = true;
    }

    fn clamp_focus(&mut self) {
        if self.rows.is_empty() {
            self.focus = (0, 0);
            return;
        }
        self.focus.0 = self.focus.0.min(self.rows.len() - 1);
        self.focus.1 = self.focus.1.min(self.rows[self.focus.0].tiles.len().saturating_sub(1));
    }

    pub(crate) fn tile_id(&self, t: &Tile) -> String {
        match t {
            Tile::Item(i) => self.items[*i].id.clone(),
            Tile::Nav { cat, .. } => format!("nav:{cat}"),
            Tile::Running(h) => format!("run:{h}"),
        }
    }

    pub(crate) fn focused_tile(&self) -> Option<&Tile> {
        self.rows.get(self.focus.0)?.tiles.get(self.focus.1)
    }

    fn focused_tile_id(&self) -> Option<String> {
        self.focused_tile().map(|t| self.tile_id(t))
    }

    pub(crate) fn run_of(&self, h: isize) -> Option<&Run> {
        self.running.iter().find(|r| r.win.hwnd == h)
    }

    /// The display name of a running app: its library name, else its window title.
    pub(crate) fn run_name(&self, h: isize) -> String {
        let Some(run) = self.run_of(h) else { return "App".into() };
        if let Some(it) = run.item.and_then(|i| self.items.get(i)) {
            return it.name.clone();
        }
        if !run.win.title.trim().is_empty() {
            return run.win.title.trim().to_owned();
        }
        std::path::Path::new(&run.win.exe).file_stem().map_or("App".into(), |s| s.to_string_lossy().to_string())
    }

    /// Whether a library item is running (it has a Running card).
    pub(crate) fn item_running(&self, i: usize) -> bool {
        self.running.iter().any(|r| r.item == Some(i))
    }

    // ---------- Geometry ----------

    pub(crate) fn tile_size(&self) -> (f32, f32, f32) {
        let r = self.rem();
        (10.0 * r, 15.0 * r, 1.5 * r)
    }

    pub(crate) fn grid_cols(&self) -> usize {
        let r = self.rem();
        let (tw, _, gap) = self.tile_size();
        (((self.gfx.width - 8.0 * r + gap) / (tw + gap)).floor() as usize).max(1)
    }

    /// A Home row's top within the shelf, before vertical scrolling.
    pub(crate) fn row_top(&self, row: usize) -> f32 {
        let r = self.rem();
        row as f32 * (1.925 * r + 1.0 * r + 15.0 * r + 1.5 * r + 1.0 * r)
    }

    /// Keeps the focused tile on screen: its row scrolls sideways (Home), the shelf vertically.
    fn reveal(&mut self, now: f64, animate: bool) {
        let r = self.rem();
        let (tw, th, gap) = self.tile_size();
        let dur = if animate { 420.0 * self.motion() } else { 0.0 };
        let (row, col) = self.focus;
        if self.grid {
            let line = col / self.grid_cols();
            let y = if line == 0 { 0.0 } else { 1.925 * r + 1.0 * r + line as f32 * (th + gap) - 1.0 * r };
            self.scroll_y.go(y, now, dur);
        } else {
            if let Some(t) = self.scroll_x.get_mut(row) {
                let view = self.gfx.width - 8.0 * r;
                let left = col as f32 * (tw + gap);
                let mut x = t.to;
                if left < x {
                    x = left;
                } else if left + tw > x + view {
                    x = left + tw - view;
                }
                t.go(x.max(0.0), now, dur);
            }
            self.scroll_y.go(self.row_top(row), now, dur);
        }
    }

    // ---------- Input ----------

    /// Whether typed characters go to the on-screen keyboard.
    pub fn text_mode(&self) -> bool {
        matches!(self.layers.last(), Some(Layer::Keyboard(_)))
    }

    pub fn action(&mut self, a: Action, now: f64) {
        self.dirty = true;
        if a == Action::Home {
            if self.go_home(now) {
                sound::play(Sound::Back);
            }
            return;
        }
        if let Some(Layer::Launch { .. }) = self.layers.last() {
            if a == Action::Back {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            return;
        }
        if self.text_mode() {
            return self.kb_action(a, now);
        }
        if matches!(a, Action::VolUp | Action::VolDown) {
            return volume(a == Action::VolUp);
        }
        if !self.layers.is_empty() {
            return self.layer_action(a, now);
        }
        match a {
            Action::Up | Action::Down | Action::Left | Action::Right => self.move_focus(a, now),
            Action::Accept => self.open_focused(now),
            Action::Menu => self.item_menu(),
            Action::Options => self.settings(),
            Action::TabPrev => self.switch_tab(-1, now),
            Action::TabNext => self.switch_tab(1, now),
            Action::Search => self.keyboard(KbPurpose::Search, String::new()),
            Action::Back => {
                if self.tab != "home" {
                    sound::play(Sound::Back);
                    self.set_tab("home", now);
                } else {
                    sound::play(Sound::Edge);
                }
            }
            Action::Home | Action::VolUp | Action::VolDown => {}
        }
    }

    fn move_focus(&mut self, a: Action, now: f64) {
        if self.rows.is_empty() {
            return;
        }
        let (row, col) = self.focus;
        let before = self.focus;
        if self.grid {
            let n = self.rows[0].tiles.len();
            let cols = self.grid_cols();
            let next = match a {
                Action::Left if col % cols > 0 => col - 1,
                Action::Right if col % cols < cols - 1 && col + 1 < n => col + 1,
                Action::Up if col >= cols => col - cols,
                Action::Down if col + cols < n => col + cols,
                Action::Down if col / cols < (n - 1) / cols => n - 1,
                _ => col,
            };
            self.focus = (0, next);
        } else {
            match a {
                Action::Left if col > 0 => self.focus.1 -= 1,
                Action::Right if col + 1 < self.rows[row].tiles.len() => self.focus.1 += 1,
                Action::Up | Action::Down => {
                    let target = if a == Action::Up { row.checked_sub(1) } else { (row + 1 < self.rows.len()).then_some(row + 1) };
                    if let Some(t) = target {
                        // The tile closest to the same place on screen.
                        let (tw, _, gap) = self.tile_size();
                        let x = col as f32 * (tw + gap) - self.scroll_x[row].to;
                        let best = ((x + self.scroll_x[t].to) / (tw + gap)).round().max(0.0) as usize;
                        self.focus = (t, best.min(self.rows[t].tiles.len() - 1));
                    }
                }
                _ => {}
            }
        }
        if self.focus != before {
            sound::play(Sound::Move);
            self.focus_changed(before, now);
        } else {
            sound::play(Sound::Edge);
        }
    }

    fn focus_changed(&mut self, before: (usize, usize), now: f64) {
        let dur = 200.0 * self.motion();
        if let Some(t) = self.rows.get(before.0).and_then(|r| r.tiles.get(before.1)) {
            let key = format!("{}:{}", self.rows[before.0].key, self.tile_id(t));
            self.scale.entry(key).or_insert(Tween::at(1.06)).go(1.0, now, dur);
        }
        if let Some(id) = self.focused_tile_id() {
            let key = format!("{}:{id}", self.rows[self.focus.0].key);
            self.scale.entry(key).or_insert(Tween::at(1.0)).go(1.06, now, dur);
            self.hero_pending = Some((id, now));
        }
        self.tab_focus.insert(self.tab.clone(), self.focus);
        self.reveal(now, true);
    }

    fn switch_tab(&mut self, d: i32, now: f64) {
        let tabs = self.tabs();
        let i = tabs.iter().position(|(k, _)| *k == self.tab).unwrap_or(0) as i32;
        let next = tabs[((i + d).rem_euclid(tabs.len() as i32)) as usize].0;
        sound::play(Sound::Move);
        self.set_tab(next, now);
    }

    fn set_tab(&mut self, tab: &str, now: f64) {
        if self.tab == tab {
            return;
        }
        self.tab_focus.insert(self.tab.clone(), self.focus);
        self.tab = tab.to_owned();
        self.scale.clear();
        self.rebuild(false);
        if let Some(id) = self.focused_tile_id() {
            self.show_hero(id, now);
        }
    }

    /// Home: the first row (the Running card when something runs), every menu closed.
    /// Returns whether anything changed.
    pub fn go_home(&mut self, now: f64) -> bool {
        let changed = self.layers_hold_items() || self.tab != "home" || self.focus != (0, 0);
        self.layers.retain(|l| matches!(l, Layer::Welcome { .. }));
        self.running.retain(|r| shell::alive(r.win.hwnd));
        if self.pending_rescan {
            self.pending_rescan = false;
            let browser = platform::chosen_browser(self.store.text("browser")).map(|b| b.exe);
            let (_, items) = library::build(&self.cache, &self.store.map("overrides"), &self.store.map("recent"), browser);
            for r in &mut self.running {
                r.item = library::identify(&items, &r.win.exe);
            }
            self.items = items;
        }
        if self.tab != "home" {
            self.tab_focus.insert(self.tab.clone(), self.focus);
            self.tab = "home".into();
        }
        self.tab_focus.remove("home");
        self.scale.clear();
        self.rebuild(false);
        if let Some(id) = self.focused_tile_id() {
            self.show_hero(id, now);
        }
        self.dirty = true;
        changed
    }

    /// The PS button from another app: Sofa is in front again, with that app as the first card.
    pub fn ps_home(&mut self, came: Option<Window>, now: f64) {
        if let Some(w) = came {
            self.add_running(w, false);
        }
        sound::play(Sound::Open);
        self.go_home(now);
    }

    /// QA only: a Running card for `w` that stands in for the game on the first Home tile.
    pub fn qa_running(&mut self, w: Window, now: f64) {
        let item = self.rows.iter().flat_map(|r| &r.tiles).find_map(|t| if let Tile::Item(i) = t { Some(*i) } else { None });
        if self.add_running(w, false)
            && let Some(r) = self.running.first_mut()
        {
            r.item = item;
            r.icon = None;
        }
        self.go_home(now);
    }

    /// Adds a Running card for a window (first). `strict` takes only library apps.
    fn add_running(&mut self, w: Window, strict: bool) -> bool {
        if w.hwnd == 0 || w.hwnd == self.shell.sofa {
            return false;
        }
        let item = library::identify(&self.items, &w.exe);
        if strict && item.is_none() {
            return false;
        }
        self.running.retain(|r| r.win.hwnd != w.hwnd && (item.is_none() || r.item != item));
        let icon = (item.is_none() && !w.exe.is_empty()).then(|| library::icon_cache(&self.cache, std::path::Path::new(&w.exe)));
        if let Some(png) = &icon
            && !png.exists()
            && !self.qa
        {
            spawn_scan(&self.cache, &[(w.exe.clone(), png.clone())], false, self.shell.sofa, WM_ICON);
        }
        self.running.insert(0, Run { win: w, item, icon });
        self.running.truncate(6);
        true
    }

    fn prune_running(&mut self) {
        let before = self.running.len();
        self.running.retain(|r| shell::alive(r.win.hwnd));
        if self.running.len() != before && !self.layers_hold_items() {
            self.rebuild(true);
        }
    }

    pub fn running_ended(&mut self, hwnd: isize) {
        let before = self.running.len();
        self.running.retain(|r| r.win.hwnd != hwnd);
        if before != self.running.len() && !self.layers_hold_items() {
            self.rebuild(true);
        }
    }

    fn open_focused(&mut self, now: f64) {
        match self.focused_tile().cloned() {
            Some(Tile::Nav { cat, .. }) => {
                sound::play(Sound::Select);
                self.set_tab(&cat, now);
            }
            Some(Tile::Item(i)) => self.open_item(i, now),
            Some(Tile::Running(h)) => self.resume(h),
            None => {}
        }
    }

    fn resume(&mut self, h: isize) {
        if self.run_of(h).is_none() {
            return;
        }
        sound::play(Sound::Select);
        if !self.qa {
            self.shell.hand_off();
            platform::focus(windows::Win32::Foundation::HWND(h as *mut _));
        }
    }

    fn open_item(&mut self, i: usize, now: f64) {
        let it = self.items[i].clone();
        if let Launch::System { action, uri } = &it.launch {
            return self.system(&it, action.clone(), uri.clone(), now);
        }
        // Like a console: picking something that is already running resumes it.
        if let Some(h) = self.running.iter().find(|r| r.item == Some(i)).map(|r| r.win.hwnd) {
            return self.resume(h);
        }
        // One game at a time, unless the person wants both.
        let other = self.running.iter().find(|r| r.item.is_some_and(|j| self.items[j].category == "games")).map(|r| r.win.hwnd);
        if it.category == "games"
            && let Some(h) = other
        {
            sound::play(Sound::Open);
            let current = self.run_name(h);
            self.layers.push(Layer::Dialog {
                title: format!("{current} is still running"),
                body: format!("Close it first, or keep it open and start {} as well.", it.name),
                opts: vec![
                    Opt::new(format!("Close it and start {}", it.name), Act::SwitchClose(h, i)).icon("close"),
                    Opt::new(format!("Start {} as well", it.name), Act::SwitchBoth(i)).icon("play"),
                    Opt::new("Cancel", Act::Close).icon("restart"),
                ],
                focus: 0,
            });
            return;
        }
        self.start(i, now);
    }

    fn start(&mut self, i: usize, now: f64) {
        let it = self.items[i].clone();
        let ok = self.qa || {
            self.shell.step_down(now);
            match &it.launch {
                Launch::Url(u) => platform::open(u),
                Launch::AppsFolder(id) => platform::apps_folder(id),
                Launch::Exec { exe, args } => platform::run(exe, args),
                Launch::Web { url, tv } => platform::open_web(&self.data, self.store.text("browser"), url.as_deref(), *tv),
                Launch::System { .. } => false,
            }
        };
        if ok {
            sound::play(Sound::Launch);
            self.store.map_mut("recent").insert(it.id.clone(), Value::from(epoch_ms()));
            self.store.flush();
            self.items[i].last_played = epoch_ms();
            self.layers.push(Layer::Launch { item: i, at: now });
        } else {
            self.shell.set_top(self.focused);
            self.toast(&format!("Couldn't open {}", it.name), "Windows didn't start it.");
        }
    }

    fn system(&mut self, it: &Item, action: String, uri: Option<String>, now: f64) {
        sound::play(Sound::Select);
        if it.confirm {
            let what: &'static str = if action == "restart" { "restart" } else { "shutdown" };
            self.layers.push(Layer::Dialog {
                title: format!("{}?", it.name),
                body: "Anything unsaved in open apps will be lost.".into(),
                opts: vec![Opt::new(it.name.clone(), Act::Power(what)).icon("check"), Opt::new("Cancel", Act::Close).icon("close")],
                focus: 0,
            });
            return;
        }
        match action.as_str() {
            "sleep" if !self.qa => platform::sleep(),
            "desktop" => self.desktop(),
            "quit" => quit(),
            "bigpicture" if !self.qa => {
                self.shell.step_down(now);
                platform::open("steam://open/bigpicture");
            }
            "settings" => self.settings(),
            "rescan" => self.manual_rescan(now),
            "sound" | "display" | "bluetooth" if !self.qa => {
                self.shell.step_down(now);
                platform::open(uri.as_deref().unwrap_or("ms-settings:sound"));
            }
            _ => {}
        }
    }

    fn manual_rescan(&mut self, now: f64) {
        self.rescan();
        self.scan_in_background(now);
        let n = self.items.iter().filter(|i| i.source != "system").count();
        self.toast("Library rescanned", &format!("{n} games and apps"));
    }

    /// Desktop mode: Sofa steps aside; the PS button brings it back.
    fn desktop(&mut self) {
        self.layers.retain(|l| matches!(l, Layer::Welcome { .. }));
        if self.qa {
            return;
        }
        self.shell.desktop = true;
        self.shell.hand_off();
        // SAFETY: minimizing our own window.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(self.gfx.hwnd(), windows::Win32::UI::WindowsAndMessaging::SW_MINIMIZE);
        }
    }

    // ---------- Layers ----------

    fn item_menu(&mut self) {
        match self.focused_tile().cloned() {
            Some(Tile::Running(h)) => {
                let Some(run) = self.run_of(h) else { return };
                let name = self.run_name(h);
                let exe = run.win.exe.to_lowercase();
                let mut opts = vec![Opt::new("Resume", Act::Resume(h)).icon("play"), Opt::new(format!("Close {name}"), Act::CloseRun(h)).icon("close")];
                if !exe.ends_with("\\applicationframehost.exe") && !exe.ends_with("\\explorer.exe") {
                    opts.push(Opt::new("Force close", Act::KillRun(h)).icon("shutdown").value("If it stops responding"));
                }
                sound::play(Sound::Open);
                self.layers.push(Layer::Sheet { title: name, sub: "Running".into(), opts, focus: 0, kind: SheetKind::Run });
            }
            Some(Tile::Item(i)) => {
                let it = &self.items[i];
                if it.source == "system" {
                    sound::play(Sound::Edge);
                    return;
                }
                let mut opts = Vec::new();
                if let Some(h) = self.running.iter().find(|r| r.item == Some(i)).map(|r| r.win.hwnd) {
                    opts.push(Opt::new("Resume", Act::Resume(h)).icon("play"));
                    opts.push(Opt::new(format!("Close {}", it.name), Act::CloseRun(h)).icon("close"));
                }
                opts.push(Opt::new(if it.favorite { "Remove from favorites" } else { "Add to favorites" }, Act::Favorite(i)).icon("star"));
                for c in ["games", "media", "apps"] {
                    if c != it.category {
                        opts.push(Opt::new(format!("Move to {}", cat_label(c)), Act::Move(i, c)).icon("move"));
                    }
                }
                opts.push(Opt::new("Rename", Act::Rename(i)).icon("rename"));
                let hidden = it.category == "hidden";
                opts.push(Opt::new(if hidden { "Show again" } else { "Hide" }, Act::Hide(i, !hidden)).icon(if hidden { "show" } else { "hide" }));
                let (title, sub) = (it.name.clone(), cat_label(&it.category).to_owned());
                sound::play(Sound::Open);
                self.layers.push(Layer::Sheet { title, sub, opts, focus: 0, kind: SheetKind::Item });
            }
            _ => sound::play(Sound::Edge),
        }
    }

    fn layout_name(&self) -> &'static str {
        match self.input.layout() {
            "ps" => "PlayStation",
            "xbox" => "Xbox",
            "nintendo" => "Nintendo",
            _ => "Match controller",
        }
    }

    fn settings_opts(&self) -> Vec<Opt> {
        let browser = platform::chosen_browser(self.store.text("browser")).map_or("None found", |b| b.name);
        let autostart = if self.qa { self.store.flag("launchAtLogin") } else { install::autostart() };
        vec![
            Opt::new("Start Sofa with Windows", Act::Toggle("launchAtLogin")).switch(autostart),
            Opt::new("Keep Sofa in front", Act::Toggle("keepInFront")).switch(self.store.flag("keepInFront")),
            Opt::new("Home button", Act::HomeButton).value(if self.store.text("homeButton") == "combo" { "Create + Options" } else { "PS button" }),
            Opt::new("Button layout", Act::Layout).value(self.layout_name()),
            Opt::new("Keep Steam off the PS button", Act::Toggle("steamGuide")).switch(self.store.flag("steamGuide")),
            Opt::new("Browser", Act::Browser).value(browser),
            Opt::new("Interface sounds", Act::Toggle("sounds")).switch(self.store.flag("sounds")),
            Opt::new("Reduce motion", Act::Toggle("motion")).switch(self.store.text("motion") == "reduced"),
            Opt::new("Show hidden apps", Act::Toggle("showHidden")).switch(self.store.flag("showHidden")),
            Opt::new("Rescan library", Act::Rescan).icon("rescan"),
            Opt::new("Go to desktop", Act::Desktop).icon("desktop").value("PS button returns"),
            Opt::new("Quit Sofa", Act::Quit).icon("quit"),
        ]
    }

    fn settings(&mut self) {
        sound::play(Sound::Open);
        let opts = self.settings_opts();
        self.layers.push(Layer::Sheet { title: "Settings".into(), sub: format!("Sofa {}", self.version), opts, focus: 0, kind: SheetKind::Settings });
    }

    fn refresh_settings(&mut self) {
        let opts = self.settings_opts();
        for l in &mut self.layers {
            if let Layer::Sheet { opts: o, kind: SheetKind::Settings, .. } = l {
                *o = opts.clone();
            }
        }
    }

    fn welcome_opts(&self) -> Vec<Opt> {
        vec![
            Opt::new("Start Sofa with Windows", Act::Toggle("launchAtLogin")).switch(self.store.flag("launchAtLogin")),
            Opt::new("Keep Steam off the PS button", Act::Toggle("steamGuide")).switch(self.store.flag("steamGuide")),
            Opt::new("Button layout", Act::Layout).value(self.layout_name()),
            Opt::new("Continue", Act::WelcomeDone).icon("check"),
        ]
    }

    fn welcome(&mut self, now: f64) {
        let opts = self.welcome_opts();
        self.layers.push(Layer::Welcome { opts, focus: 3, at: now });
    }

    fn refresh_welcome(&mut self) {
        let fresh = self.welcome_opts();
        if let Some(Layer::Welcome { opts, .. }) = self.layers.iter_mut().find(|l| matches!(l, Layer::Welcome { .. })) {
            *opts = fresh;
        }
    }

    // ---------- On-screen keyboard ----------

    pub(crate) fn keyboard(&mut self, purpose: KbPurpose, text: String) {
        sound::play(Sound::Open);
        let mut kb = Keyboard { text, purpose, key: (2, 0), result: None, hits: Vec::new(), shift: false, typed: false };
        self.search(&mut kb);
        self.layers.push(Layer::Keyboard(kb));
    }

    fn search(&self, kb: &mut Keyboard) {
        kb.hits.clear();
        if kb.purpose != KbPurpose::Search {
            return;
        }
        let q = kb.text.trim().to_lowercase();
        if q.is_empty() {
            return;
        }
        let show_hidden = self.store.flag("showHidden");
        let mut hits: Vec<(usize, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.source != "system" && (show_hidden || it.category != "hidden"))
            .filter_map(|(i, it)| {
                let n = it.name.to_lowercase();
                // Name start first, then a word start, then anywhere.
                n.find(&q).map(|p| (if p == 0 { 0 } else if n[..p].ends_with([' ', ':', '-']) { 1 } else { 2 }, i))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| self.items[a.1].name.len().cmp(&self.items[b.1].name.len())));
        kb.hits = hits.into_iter().take(KB_RESULTS).map(|(_, i)| i).collect();
    }

    /// A typed character (physical keyboard) for the on-screen keyboard.
    pub fn type_char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.kb_edit(|t| t.push(c));
        self.kb_mark_typed();
    }

    /// The physical keyboard was used (Backspace, Ctrl+V, a character).
    pub fn kb_mark_typed(&mut self) {
        if let Some(Layer::Keyboard(kb)) = self.layers.last_mut() {
            kb.typed = true;
        }
    }

    pub fn erase(&mut self) {
        self.kb_edit(|t| {
            t.pop();
        });
    }

    pub fn paste(&mut self) {
        let clip = platform::clipboard_text().split_whitespace().collect::<Vec<_>>().join(" ");
        let clip: String = clip.chars().take(120).collect();
        if clip.is_empty() {
            sound::play(Sound::Edge);
        } else {
            self.kb_edit(|t| t.push_str(&clip));
        }
    }

    fn kb_edit(&mut self, f: impl FnOnce(&mut String)) {
        let Some(Layer::Keyboard(mut kb)) = self.layers.pop() else { return };
        f(&mut kb.text);
        kb.text = kb.text.chars().take(80).collect();
        self.search(&mut kb);
        if kb.result.is_some_and(|r| r >= kb.hits.len()) {
            kb.result = None;
        }
        self.layers.push(Layer::Keyboard(kb));
        self.dirty = true;
    }

    /// Finishes the keyboard: rename saves, search just closes.
    pub fn kb_done(&mut self) {
        let Some(Layer::Keyboard(kb)) = self.layers.pop() else { return };
        if let KbPurpose::Rename(i) = kb.purpose {
            let name = kb.text.trim().to_owned();
            if name.is_empty() {
                // An empty name brings the original back.
                self.set_override(i, "name", None);
                self.pending_rescan = true;
            } else if name != self.items[i].name {
                self.items[i].name = name.clone();
                self.set_override(i, "name", Some(Value::from(name)));
                self.rebuild(true);
            }
        }
        sound::play(Sound::Back);
    }

    /// Enter on a physical keyboard: after typing it opens the first result (or finishes);
    /// after moving on the on-screen keys it presses the focused one.
    pub fn kb_enter(&mut self, now: f64) {
        let Some(Layer::Keyboard(kb)) = self.layers.last() else { return };
        if !kb.typed || kb.result.is_some() {
            return self.kb_action(Action::Accept, now);
        }
        if let Some(&i) = kb.hits.first() {
            self.layers.pop();
            self.open_item(i, now);
        } else {
            self.kb_done();
        }
    }

    fn kb_action(&mut self, a: Action, now: f64) {
        let Some(Layer::Keyboard(kb)) = self.layers.last_mut() else { return };
        match a {
            Action::Back => {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            Action::Search => self.erase(),
            Action::Menu => self.kb_edit(|t| t.push(' ')),
            Action::Options | Action::VolUp => self.kb_done(),
            Action::VolDown => {
                kb.shift = !kb.shift;
                sound::play(Sound::Move);
            }
            Action::Accept => {
                if let Some(r) = kb.result {
                    let i = kb.hits[r];
                    self.layers.pop();
                    self.open_item(i, now);
                    return;
                }
                let (row, col) = kb.key;
                if row < 4 {
                    let c = KB_ROWS[row].as_bytes()[col] as char;
                    let c = if kb.shift { c.to_ascii_uppercase() } else { c };
                    sound::play(Sound::Move);
                    self.kb_edit(|t| t.push(c));
                } else {
                    let key = KB_BOTTOM.iter().find(|(_, c, w)| col >= *c && col < c + w).map_or("Done", |k| k.0);
                    match key {
                        "shift" => {
                            kb.shift = !kb.shift;
                            sound::play(Sound::Move);
                        }
                        "space" => self.kb_edit(|t| t.push(' ')),
                        "erase" => self.erase(),
                        "Paste" => self.paste(),
                        "Clear" => self.kb_edit(String::clear),
                        _ => self.kb_done(),
                    }
                }
            }
            Action::Up | Action::Down | Action::Left | Action::Right => {
                kb.typed = false;
                let moved = if let Some(r) = kb.result {
                    let n = kb.hits.len();
                    let cols = KB_RESULT_COLS;
                    match a {
                        Action::Left if r % cols == 0 => {
                            // Back to the keys, on the same row.
                            kb.result = None;
                            kb.key.0 = (r / cols).min(4);
                            kb.key.1 = if kb.key.0 == 4 { 8 } else { 9 };
                            true
                        }
                        Action::Left => {
                            kb.result = Some(r - 1);
                            true
                        }
                        Action::Right if r % cols < cols - 1 && r + 1 < n => {
                            kb.result = Some(r + 1);
                            true
                        }
                        Action::Up if r >= cols => {
                            kb.result = Some(r - cols);
                            true
                        }
                        Action::Down if r + cols < n => {
                            kb.result = Some(r + cols);
                            true
                        }
                        _ => false,
                    }
                } else {
                    let (row, col) = kb.key;
                    // On the bottom row a step moves across a whole key.
                    let span = KB_BOTTOM.iter().find(|(_, c, w)| row == 4 && col >= *c && col < c + w).map_or((col, 1), |k| (k.1, k.2));
                    match a {
                        Action::Left if row == 4 && span.0 > 0 => {
                            kb.key.1 = span.0 - 1;
                            true
                        }
                        Action::Left if row < 4 && col > 0 => {
                            kb.key.1 -= 1;
                            true
                        }
                        Action::Right if (row < 4 && col < 9) || (row == 4 && span.0 + span.1 < 10) => {
                            kb.key.1 = if row == 4 { span.0 + span.1 } else { col + 1 };
                            true
                        }
                        Action::Right if !kb.hits.is_empty() => {
                            // Into the results, on the same row where there is one.
                            let rows = kb.hits.len().div_ceil(KB_RESULT_COLS);
                            kb.result = Some(row.min(rows - 1) * KB_RESULT_COLS);
                            true
                        }
                        Action::Up if row > 0 => {
                            kb.key.0 -= 1;
                            true
                        }
                        Action::Down if row < 4 => {
                            kb.key.0 += 1;
                            true
                        }
                        _ => false,
                    }
                };
                sound::play(if moved { Sound::Move } else { Sound::Edge });
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn layer_action(&mut self, a: Action, now: f64) {
        let kind = match self.layers.last() {
            Some(Layer::Sheet { kind, .. }) => Some(*kind),
            _ => None,
        };
        let welcome = matches!(self.layers.last(), Some(Layer::Welcome { .. }));
        let Some(top) = self.layers.last_mut() else { return };
        let (opts, focus) = match top {
            Layer::Sheet { opts, focus, .. } | Layer::Dialog { opts, focus, .. } | Layer::Welcome { opts, focus, .. } => (opts.clone(), focus),
            _ => return,
        };
        match a {
            Action::Up if *focus > 0 => {
                *focus -= 1;
                sound::play(Sound::Move);
            }
            Action::Down if *focus + 1 < opts.len() => {
                *focus += 1;
                sound::play(Sound::Move);
            }
            Action::Up | Action::Down => sound::play(Sound::Edge),
            Action::Back if !welcome => {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            Action::Options if kind == Some(SheetKind::Settings) => {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            Action::Menu if matches!(kind, Some(SheetKind::Item | SheetKind::Run)) => {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            Action::Accept => {
                let act = opts[*focus].act.clone();
                sound::play(Sound::Select);
                self.run(act, now);
            }
            _ => {}
        }
    }

    fn set_override(&mut self, i: usize, key: &str, value: Option<Value>) {
        let id = self.items[i].id.clone();
        let ov = self.store.map_mut("overrides");
        let entry = ov.entry(id.clone()).or_insert_with(|| Value::Object(Default::default()));
        if let Value::Object(m) = entry {
            match value {
                Some(v) => {
                    m.insert(key.into(), v);
                }
                None => {
                    m.remove(key);
                }
            }
            if m.is_empty() {
                ov.remove(&id);
            }
        }
        self.store.flush();
    }

    fn toggle(&mut self, key: &'static str, now: f64) {
        match key {
            "launchAtLogin" => {
                let on = if self.qa || !self.store.top_flag("setupDone") { !self.store.flag(key) } else { !install::autostart() };
                if !self.qa && self.store.top_flag("setupDone") {
                    install::set_autostart(on);
                }
                self.store.set(key, Value::Bool(on));
            }
            "motion" => {
                let reduced = self.store.text("motion") != "reduced";
                self.store.set("motion", Value::from(if reduced { "reduced" } else { "full" }));
            }
            "steamGuide" => {
                let on = !self.store.flag(key);
                self.store.set(key, Value::Bool(on));
                if on && !self.qa && self.store.top_flag("setupDone") {
                    self.steam_fix(false);
                }
            }
            _ => {
                let on = !self.store.flag(key);
                self.store.set(key, Value::Bool(on));
                if key == "showHidden" && !on && self.tab == "hidden" {
                    self.set_tab("home", now);
                }
            }
        }
        self.store.flush();
        self.apply_settings();
    }

    fn run(&mut self, act: Act, now: f64) {
        match act {
            Act::Favorite(i) => {
                let fav = !self.items[i].favorite;
                self.items[i].favorite = fav;
                self.set_override(i, "favorite", fav.then_some(Value::Bool(true)));
                self.layers.pop();
                self.rebuild(true);
            }
            Act::Move(i, c) => {
                self.items[i].category = c.into();
                self.set_override(i, "category", Some(Value::from(c)));
                self.set_override(i, "hidden", None);
                self.layers.pop();
                self.rebuild(true);
            }
            Act::Hide(i, hide) => {
                self.layers.pop();
                if hide {
                    self.items[i].category = "hidden".into();
                    self.set_override(i, "hidden", Some(Value::Bool(true)));
                    self.rebuild(true);
                } else {
                    self.set_override(i, "hidden", None);
                    self.set_override(i, "category", None);
                    self.rescan();
                }
            }
            Act::Rename(i) => {
                self.layers.pop();
                let name = self.items[i].name.clone();
                self.keyboard(KbPurpose::Rename(i), name);
            }
            Act::Resume(h) => {
                self.layers.pop();
                self.resume(h);
            }
            Act::CloseRun(h) | Act::KillRun(h) => {
                self.layers.pop();
                if !self.qa {
                    if matches!(act, Act::KillRun(_)) {
                        shell::kill(h);
                    } else {
                        shell::close(h, self.shell.sofa);
                    }
                }
                // The card goes once the window is gone (a game may ask to save first).
                self.prune_at = Some(now + 1500.0);
            }
            Act::SwitchClose(h, i) => {
                self.layers.pop();
                if !self.qa {
                    shell::close(h, self.shell.sofa);
                }
                self.running.retain(|r| r.win.hwnd != h);
                self.rebuild(true);
                self.start(i, now);
            }
            Act::SwitchBoth(i) => {
                self.layers.pop();
                self.start(i, now);
            }
            Act::Toggle(key) => {
                self.toggle(key, now);
                self.refresh_settings();
                self.refresh_welcome();
            }
            Act::HomeButton => {
                let next = if self.store.text("homeButton") == "combo" { "ps" } else { "combo" };
                self.store.set("homeButton", Value::from(next));
                self.store.flush();
                self.apply_settings();
                self.refresh_settings();
            }
            Act::Layout => {
                let order = ["auto", "ps", "xbox", "nintendo"];
                let next = order[(order.iter().position(|l| *l == self.input.layout()).unwrap_or(0) + 1) % order.len()];
                self.store.set("controllerLayout", Value::from(next));
                self.store.flush();
                self.apply_settings();
                self.refresh_settings();
                self.refresh_welcome();
            }
            Act::Browser => {
                let all = platform::browsers();
                if all.len() > 1 {
                    let cur = platform::chosen_browser(self.store.text("browser")).map(|b| b.key);
                    let i = all.iter().position(|b| Some(b.key) == cur).map_or(0, |i| (i + 1) % all.len());
                    self.store.set("browser", Value::from(all[i].key));
                    self.store.flush();
                    self.pending_rescan = true;
                } else {
                    sound::play(Sound::Edge);
                }
                self.refresh_settings();
            }
            Act::Rescan => {
                self.layers.pop();
                self.manual_rescan(now);
            }
            Act::Desktop => self.desktop(),
            Act::Quit => quit(),
            Act::Power(what) => {
                self.layers.pop();
                if !self.qa {
                    platform::restart(what == "shutdown");
                }
            }
            Act::SteamFixNow => {
                self.layers.pop();
                self.steam_fix(true);
            }
            Act::WelcomeDone => {
                self.layers.pop();
                self.store.set_top("setupDone", Value::Bool(true));
                self.store.flush();
                if !self.qa {
                    install::set_autostart(self.store.flag("launchAtLogin"));
                    if self.store.flag("steamGuide") {
                        self.steam_fix(false);
                    }
                }
                self.toast("Sofa is ready", "Press the PS button in any game to come back here.");
            }
            Act::Close => {
                self.layers.pop();
            }
        }
    }

    // ---------- Steam and the PS button ----------

    /// Turns off Steam's "Guide button focuses Steam" on a background thread. With `restart`,
    /// a running Steam is closed and started again (the person agreed). The result arrives as
    /// WM_STEAM.
    pub fn steam_fix(&mut self, restart: bool) {
        if self.qa {
            return;
        }
        let Some(root) = library::steam_root() else { return };
        let hwnd = self.shell.sofa;
        if restart {
            self.toast("Fixing Steam", "Steam closes, changes one setting and starts again in the background.");
        }
        let _ = std::thread::Builder::new().name("steam".into()).spawn(move || {
            let code = match steam::fix(&root, restart, &platform::steam_running) {
                steam::Fix::Changed { restarted: true } => 1,
                steam::Fix::Changed { restarted: false } => 2,
                steam::Fix::AlreadyOff => 3,
                steam::Fix::Running => 4,
                steam::Fix::StillRunning => 5,
                steam::Fix::NoSteam | steam::Fix::NoAccount => 7,
                steam::Fix::Write(_) => 6,
            };
            post(hwnd, WM_STEAM, code);
        });
    }

    pub fn steam_result(&mut self, code: usize) {
        crate::log(&format!("steam fix result {code}"));
        match code {
            1 => self.toast("Steam no longer takes the PS button", "Steam restarted in the background."),
            2 => self.toast("Steam no longer takes the PS button", "Sofa turned off \"Guide button focuses Steam\"."),
            5 => self.toast("Steam didn't close", "Close Steam yourself, then try again from Settings."),
            6 => self.toast("Couldn't change Steam's setting", "You can turn it off in Steam, Settings, Controller."),
            _ => {}
        }
    }

    /// Steam answered the PS button too. Sofa pushed it back; offer the fix once per run.
    pub fn steam_grab(&mut self) {
        crate::log("steam took the screen; pushed back");
        if self.steam_tip || self.qa || !self.layers.is_empty() || !self.store.flag("steamGuide") {
            return;
        }
        self.steam_tip = true;
        let Some(root) = library::steam_root() else { return };
        if steam::status(&root, &|| true).off {
            return;
        }
        sound::play(Sound::Open);
        self.layers.push(Layer::Dialog {
            title: "Steam is also using the PS button".into(),
            body: "Sofa can turn off Steam's \"Guide button focuses Steam\". Steam closes and starts again in the background, so finish any Steam game first.".into(),
            opts: vec![Opt::new("Fix it now", Act::SteamFixNow).icon("check"), Opt::new("Not now", Act::Close).icon("close")],
            focus: 0,
        });
        self.dirty = true;
    }

    // ---------- Notices, focus and memory ----------

    pub fn toast(&mut self, title: &str, body: &str) {
        self.toast = Some((title.into(), body.into(), -1.0));
        sound::play(Sound::Notice);
        self.dirty = true;
    }

    pub fn on_focus(&mut self, focused: bool, now: f64) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        self.shell.on_focus(focused && !self.qa, now);
        if focused {
            self.unfocused_at = None;
            self.trimmed = false;
            // Back from an app without the PS button (it closed, or Alt+Tab): it gets a card
            // when it is a library app or a full-screen one.
            if let Some((w, full)) = self.shell.take_last_app()
                && self.add_running(w, !full)
                && !self.layers_hold_items()
            {
                self.rebuild(true);
            }
            if let Some(Layer::Launch { at, .. }) = self.layers.last()
                && now - at > 300.0
            {
                self.layers.pop();
            }
            self.prune_running();
            if now - self.scanned_at > 300_000.0 {
                self.scan_in_background(now);
            }
        } else {
            self.unfocused_at = Some(now);
        }
        self.dirty = true;
    }

    /// Gives memory back after Sofa has been behind a game for a while (it reloads its art when
    /// it comes back).
    pub fn trim_if_idle(&mut self, now: f64) {
        let Some(at) = self.unfocused_at else { return };
        if self.trimmed || now - at < 15_000.0 {
            return;
        }
        self.trim();
    }

    pub fn trim(&mut self) {
        self.trimmed = true;
        self.layers.retain(|l| !matches!(l, Layer::Launch { .. }));
        self.gfx.release();
        platform::trim_working_set();
        self.dirty = true;
        crate::log("trimmed");
    }

    /// Controllers connecting, disconnecting, and a low battery, as notices.
    fn watch_pads(&mut self, now: f64) {
        let info = self.input.info();
        let x = self.input.xinput_active();
        let low = info.connected && info.battery.is_some_and(|b| b <= 15) && !info.charging;
        let (was_ds, was_x, was_low) = self.pads;
        self.pads = (info.connected, x, low);
        if self.qa || now - self.boot_at < 4000.0 {
            return;
        }
        if info.connected && !was_ds {
            let b = info.battery.map_or(String::new(), |b| format!("Battery {b}%"));
            self.toast("Wireless Controller connected", &b);
        } else if !info.connected && was_ds {
            self.toast("Controller disconnected", "");
        } else if x && !was_x && !info.connected {
            self.toast("Controller connected", "");
        }
        if low && !was_low {
            self.toast("Controller battery low", &format!("{}% left. Connect it with a USB cable to charge.", info.battery.unwrap_or(0)));
        }
    }

    pub(crate) fn show_hero(&mut self, id: String, now: f64) {
        if self.hero.as_ref().is_some_and(|(h, _)| *h == id) {
            self.hero_pending = None;
            return;
        }
        self.hero_prev = self.hero.take();
        self.hero = Some((id, now));
        self.hero_pending = None;
    }

    /// Advances timers; true while anything still moves (keep drawing frames).
    pub fn animating(&mut self, now: f64) -> bool {
        if let Some((id, at)) = self.hero_pending.clone()
            && now - at >= 90.0
        {
            self.show_hero(id, now);
        }
        if let Some(Layer::Launch { at, .. }) = self.layers.last()
            && now - at > 7000.0
        {
            self.layers.pop();
            self.dirty = true;
        }
        if let Some(t) = self.prune_at
            && now > t
        {
            self.prune_at = None;
            self.prune_running();
        }
        if self.pending_rescan && !self.layers_hold_items() {
            self.rescan();
        }
        if let Some((_, _, at)) = &mut self.toast {
            if *at < 0.0 {
                *at = now;
            }
            if now - *at > 5300.0 {
                self.toast = None;
                self.dirty = true;
            }
        }
        let minute = clock_minute();
        if minute != self.minute {
            self.minute = minute;
            self.dirty = true;
        }
        self.watch_pads(now);
        let m = self.motion() > 0.0;
        let hero_moving = m && self.hero.as_ref().is_some_and(|(_, t)| now - t < 1250.0);
        let welcome = self.layers.iter().any(|l| matches!(l, Layer::Welcome { at, .. } if now - at < 900.0));
        self.dirty
            || now - self.boot_at < 1000.0
            || welcome
            || hero_moving
            || self.hero_pending.is_some()
            || self.scroll_y.busy(now)
            || self.scroll_x.iter().any(|t| t.busy(now))
            || self.scale.values().any(|t| t.busy(now))
            || self.tab_line.0.busy(now)
            || self.gfx.fading()
            || matches!(self.layers.last(), Some(Layer::Launch { .. }))
            || self.toast.as_ref().is_some_and(|(_, _, at)| *at < 0.0 || now - at < 300.0 || now - at > 4990.0)
    }

    /// One line describing what is on screen, for the QA run's log.
    pub fn state_line(&self) -> String {
        let layer = match self.layers.last() {
            None => "none".to_owned(),
            Some(Layer::Sheet { title, focus, .. }) => format!("sheet:{title}:{focus}"),
            Some(Layer::Dialog { title, focus, .. }) => format!("dialog:{title}:{focus}"),
            Some(Layer::Launch { item, .. }) => format!("launch:{}", self.items[*item].id),
            Some(Layer::Keyboard(kb)) => format!("keyboard:{:?}:{}:{}:{}", kb.purpose, kb.text, kb.hits.len(), kb.result.map_or(-1, |r| r as i64)),
            Some(Layer::Welcome { focus, .. }) => format!("welcome:{focus}"),
        };
        format!(
            "state|tab={}|focus={},{}|tile={}|layer={}|rows={}|items={}|running={}|toast={}",
            self.tab,
            self.focus.0,
            self.focus.1,
            self.focused_tile_id().unwrap_or_default(),
            layer,
            self.rows.len(),
            self.items.len(),
            self.running.len(),
            self.toast.as_ref().map_or("", |t| t.0.as_str())
        )
    }

    pub fn save(&mut self) {
        self.store.flush();
    }
}

pub(crate) fn clock_minute() -> u16 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain call.
    let t = unsafe { GetLocalTime() };
    t.wHour * 60 + t.wMinute
}

fn volume(up: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_VOLUME_DOWN, VK_VOLUME_UP, keybd_event};
    let k = if up { VK_VOLUME_UP } else { VK_VOLUME_DOWN };
    // SAFETY: synthesizing the media volume key.
    unsafe {
        keybd_event(k.0 as u8, 0, Default::default(), 0);
        keybd_event(k.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
}

fn quit() {
    // SAFETY: posts WM_QUIT to this thread's queue.
    unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
}

fn post(hwnd: isize, msg: u32, w: usize) {
    // SAFETY: posting to our window; a closed window drops it.
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(windows::Win32::Foundation::HWND(hwnd as *mut _)),
            msg,
            windows::Win32::Foundation::WPARAM(w),
            windows::Win32::Foundation::LPARAM(0),
        );
    }
}
