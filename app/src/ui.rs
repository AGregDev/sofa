//! Sofa's state and behaviour: what each button does on each screen. Drawing is in `view.rs`.
//! Layout and motion follow Sofa 1.x (`renderer/app.js`, `style.css`): rem = screen height /
//! 67.5, one easing curve, warm graphite with one amber accent.

use crate::audio;
use crate::gfx::{Color, Gfx};
use crate::input::{self, Action, Input, Lights, PadKind};
use crate::library::{self, Item, Launch};
use crate::media::{self, NowPlaying};
use crate::shell::{self, Shell, Window};
use crate::spotify::{self, Music, Pick};
use crate::sound::{self, Sound};
use crate::social::{self, Req};
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
/// Discord's own colour, for its tiles and the controller light on them.
pub(crate) const DISCORD: Color = Color::hex(0x5865f2);

/// The setting that starts Sofa with the computer.
const START_WITH: &str = if cfg!(windows) { "Start Sofa with Windows" } else { "Start Sofa when you sign in" };

/// Sleep timer lengths, in minutes: the quick menu steps through them, then off.
const TIMER_STEPS: [u32; 4] = [30, 60, 90, 120];

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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: only nice(2), which is async-signal-safe, runs between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::nice(10);
                Ok(())
            });
        }
    }
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
    /// When its program started (epoch ms), for "Running · 1 h 12 min".
    pub since: Option<f64>,
}

/// A control in the quick menu (the PS button on Home), by what it changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Ctl {
    /// What is playing: ✕ plays or pauses, L1 and R1 skip.
    Music,
    Output,
    Volume,
    Mic,
    Timer,
    Light,
    /// Memory, video memory and Sofa's frame rate; the overlay.
    Perf,
    Power,
}

impl Ctl {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Ctl::Music => "music",
            Ctl::Output => "output",
            Ctl::Volume => "volume",
            Ctl::Mic => "mic",
            Ctl::Timer => "timer",
            Ctl::Light => "light",
            Ctl::Perf => "perf",
            Ctl::Power => "power",
        }
    }
}

/// How a quick menu control looks: its icon, what it is, its state, and what ✕ does.
pub(crate) struct CtlFace {
    pub icon: &'static str,
    pub label: &'static str,
    /// The state ("Living room TV", "40%", "Muted"); empty for controls that only open.
    pub value: String,
    /// A state that is on (muted, a timer running): amber.
    pub on: bool,
    /// The volume level, drawn as a bar.
    pub level: Option<f32>,
    pub verb: &'static str,
    /// What is playing: its art, and the app playing it.
    pub art: Option<PathBuf>,
    pub app: String,
}

/// Windows' sound devices as last read. A QA run starts with made-up ones and changes only
/// those: it must not touch the real sound settings.
#[derive(Clone, Default)]
pub(crate) struct AudioState {
    pub outputs: Vec<audio::Output>,
    pub current: Option<String>,
    pub volume: Option<f32>,
    pub muted: bool,
    pub mic: Option<bool>,
}

impl AudioState {
    fn qa() -> AudioState {
        let out = |id: &str, name: &str, kind| audio::Output { id: id.into(), name: name.into(), detail: String::new(), kind };
        AudioState {
            outputs: vec![out("qa-tv", "Living room TV", audio::Kind::Tv), out("qa-headphones", "Headphones", audio::Kind::Headphones)],
            current: Some("qa-tv".into()),
            volume: Some(0.4),
            muted: false,
            mic: Some(false),
        }
    }

    pub(crate) fn output(&self) -> Option<&audio::Output> {
        self.outputs.iter().find(|o| Some(&o.id) == self.current.as_ref())
    }
}

pub(crate) fn kind_icon(k: audio::Kind) -> &'static str {
    match k {
        audio::Kind::Tv => "tv",
        audio::Kind::Headphones => "headphones",
        audio::Kind::Speakers => "speaker",
        audio::Kind::Controller => "bigpicture",
    }
}

/// "45 min", "1 h 30 min".
pub(crate) fn duration_text(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

#[derive(Clone, PartialEq)]
pub(crate) enum Tile {
    Item(usize),
    Nav { cat: String, count: usize },
    /// A running app, by its window.
    Running(isize),
    /// Discord voice (Home's row and the Social tab).
    Social(Soc),
    /// Spotify (Home's row and the Music tab).
    Music(Mu),
}

/// A Spotify tile: something to play (by its URI), or the account and the way to connect.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Mu {
    Play(String),
    Account,
    Setup,
}

impl Mu {
    pub(crate) fn key(&self) -> String {
        match self {
            Mu::Play(uri) => uri.clone(),
            Mu::Account => "account".into(),
            Mu::Setup => "setup".into(),
        }
    }
}

/// Spotify's colours, for its tiles and the controller light on them.
pub(crate) const SPOTIFY: Color = Color::hex(0x1ed760);
pub(crate) const SPOTIFY_BG: Color = Color::hex(0x121212);

/// A Discord tile, by what it does (channel, server and member ids are Discord's).
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Soc {
    /// The voice channel you are in.
    Here,
    Mute,
    Deaf,
    Leave,
    /// A recent voice channel: selecting joins it.
    Join(String),
    Server(String),
    Member(String),
    Account,
    Setup,
    /// Discord is not running: selecting opens it.
    Open,
}

impl Soc {
    pub(crate) fn key(&self) -> String {
        match self {
            Soc::Here => "here".into(),
            Soc::Mute => "mute".into(),
            Soc::Deaf => "deaf".into(),
            Soc::Leave => "leave".into(),
            Soc::Join(id) => format!("vc:{id}"),
            Soc::Server(id) => format!("g:{id}"),
            Soc::Member(id) => format!("m:{id}"),
            Soc::Account => "account".into(),
            Soc::Setup => "setup".into(),
            Soc::Open => "open".into(),
        }
    }
}

/// The Connect Discord steps in progress (nothing is saved until Connect).
#[derive(Default)]
pub(crate) struct DcSetup {
    pub client_id: String,
    pub secret: Option<String>,
    pub msg: String,
    /// Connect was selected: the next answer from Discord closes or explains the sheet.
    pub waiting: bool,
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
    /// The PS button (Guide on Xbox, Home on Nintendo).
    Home,
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
    DcPage,
    DcId,
    DcSecret,
    DcConnect,
    DcRedo,
    DcForget,
    DcForgetNow,
    DcJoin(String),
    DcLeave,
    /// Make this sound output the default.
    Output(String),
    /// A System tile's action (`sys:<key>`), from the Power sheet.
    System(&'static str),
    /// Controller light: follows the game, amber, off.
    Light,
    /// The sleep timer's warning: cancel it, or sleep now.
    StayAwake,
    SleepNow,
    /// Spotify: the setup steps, the account, and playing.
    SpPage,
    SpId,
    SpConnect,
    SpRedo,
    SpRefresh,
    SpForget,
    SpForgetNow,
    SpPlay(String, bool),
    SpOpen(String),
    /// A line that only shows something (the Performance panel's numbers).
    Info,
}

#[derive(Clone)]
pub(crate) struct Opt {
    pub label: String,
    pub icon: Option<&'static str>,
    pub value: Option<String>,
    pub switch: Option<bool>,
    /// A second, smaller line under the label (setup steps).
    pub detail: Option<String>,
    pub act: Act,
}

impl Opt {
    fn new(label: impl Into<String>, act: Act) -> Opt {
        Opt { label: label.into(), icon: None, value: None, switch: None, detail: None, act }
    }
    fn detail(mut self, d: impl Into<String>) -> Opt {
        self.detail = Some(d.into());
        self
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
    DcSetup,
    DcAccount,
    /// Who is in your voice channel.
    DcVoice,
    /// A server's voice channels (`App::dc_server`).
    DcServer,
    /// Sound outputs, from the quick menu.
    Sound,
    /// Sleep, restart, shut down, from the quick menu.
    Power,
    /// Memory, video memory and the frame rate, from the quick menu.
    Perf,
    /// Connect Spotify, its account, and a playlist's or album's options.
    SpSetup,
    SpAccount,
    Music,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum KbPurpose {
    Search,
    Rename(usize),
    DcId,
    DcSecret,
    SpId,
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
    /// The quick menu: the PS button while Home is showing. Focus is kept by control, since
    /// the controls shown can change while it is open (a controller disconnects).
    Quick { focus: Ctl, at: f64 },
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
    /// The streaming service Sofa last opened (its window is `shell.media`).
    media_item: Option<String>,
    pub(crate) social: social::Social,
    /// Discord as last read (the `discord` thread updates it, WM_SOCIAL).
    pub(crate) voice: social::Voice,
    social_at: f64,
    pub(crate) dc: DcSetup,
    pub(crate) dc_server: Option<String>,
    dc_authorizing: bool,
    pub(crate) audio: AudioState,
    /// When L2 or R2 last changed the volume: the top bar shows the level for a moment.
    pub(crate) vol_at: f64,
    /// When the sleep timer ends (epoch ms, so time asleep counts).
    pub(crate) sleep_at: Option<f64>,
    /// The one-minute warning is up; it came over this window (resumed on Stay awake).
    sleep_warned: bool,
    sleep_from: Option<isize>,
    /// A Running window chosen with a press, waiting for it to be released (window, when it
    /// was chosen, since when nothing is down), and the last one that came forward.
    resume_wait: Option<(isize, f64, Option<f64>)>,
    resumed: Option<isize>,
    /// Seconds left as last drawn (the top bar's minutes, the warning's countdown).
    sleep_shown: i64,
    /// The colour Sofa gives the DualSense's lightbar while it is in front.
    light_rgb: (u8, u8, u8),
    /// What is playing (the `media` thread updates it, WM_MEDIA).
    pub(crate) media: media::Media,
    pub(crate) playing: Option<NowPlaying>,
    /// Spotify (the `spotify` thread updates it, WM_MUSIC), and the Client ID being set up.
    pub(crate) spotify: spotify::Spotify,
    pub(crate) music: Music,
    pub(crate) sp_id: String,
    /// Spotify changed while a menu was open: Home and Music rebuild when it closes.
    music_stale: bool,
    /// What the Performance panel and the overlay show, and when it was read.
    pub(crate) perf: Option<crate::perf::Usage>,
    perf_at: f64,
    /// The numbers last shown, so the screen is drawn again only when one changes.
    perf_shown: String,
    /// The frames drawn in the last second, for the frame rate.
    pub(crate) frames: crate::perf::Frames,
    /// An Xbox (XInput) pad was connected this run: Game Bar's use of its button is offered.
    pub(crate) xbox: bool,
}

const TABS: [(&str, &str); 7] = [("home", "Home"), ("games", "Games"), ("media", "Media"), ("music", "Music"), ("apps", "Apps"), ("social", "Social"), ("system", "System")];

/// A QA run's made-up Spotify library (no covers: nothing is downloaded in QA).
fn qa_music() -> Music {
    let pick = |kind, uri: &str, name: &str, songs: u32, by: &str| Pick { kind, uri: uri.into(), name: name.into(), songs: Some(songs), by: by.into(), cover: None };
    Music {
        configured: true,
        ready: true,
        user: Some("Sofa QA".into()),
        library: vec![
            pick(spotify::Kind::Liked, "spotify:user:qa:collection", "Liked Songs", 512, ""),
            pick(spotify::Kind::Playlist, "spotify:playlist:qa1", "Late Night Drive", 42, "Sofa QA"),
            pick(spotify::Kind::Playlist, "spotify:playlist:qa2", "Couch Co-op", 18, "Sofa QA"),
            pick(spotify::Kind::Playlist, "spotify:playlist:qa3", "Rainy Sunday", 30, "A friend"),
            pick(spotify::Kind::Album, "spotify:album:qa4", "Random Access Memories", 13, "Daft Punk"),
            pick(spotify::Kind::Album, "spotify:album:qa5", "Discovery", 14, "Daft Punk"),
        ],
        ..Music::default()
    }
}

pub(crate) fn cat_label(c: &str) -> &'static str {
    match c {
        "games" => "Games",
        "media" => "Media",
        "music" => "Music",
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
        // Discord voice: never in a QA run (it would talk to the real Discord).
        let discord = store.map("social").get("discord").cloned().unwrap_or(Value::Null);
        let social = social::Social::start(data.clone(), cache.clone(), shell.sofa, discord, !qa);
        let voice = social.snapshot();
        // What is playing: never read from (or controlled in) the real world in a QA run, which
        // plays a made-up track instead.
        let media = media::Media::start(cache.clone(), shell.sofa, !qa);
        if qa {
            media.set(Some(NowPlaying { title: "Night Drive".into(), artist: "The Late Show".into(), app: "Spotify".into(), playing: true, art: None }));
        }
        let playing = media.snapshot();
        // Spotify: never in a QA run, which shows a made-up library instead.
        let sp_cfg = store.map("music").get("spotify").cloned().unwrap_or(Value::Null);
        let spotify = spotify::Spotify::start(data.clone(), cache.clone(), shell.sofa, sp_cfg, !qa);
        if qa {
            spotify.set(qa_music());
        }
        let music = spotify.snapshot();
        // An Xbox pad connected at start: Game Bar's use of its button is offered.
        let xbox = !qa && input.xbox_now();
        let mut app = App {
            gfx,
            input,
            shell,
            store,
            data,
            cache,
            items: Vec::new(),
            running: Vec::new(),
            media_item: None,
            social,
            voice,
            social_at: -1e9,
            dc: DcSetup::default(),
            dc_server: None,
            dc_authorizing: false,
            audio: if qa { AudioState::qa() } else { AudioState::default() },
            vol_at: -1e9,
            sleep_at: None,
            sleep_warned: false,
            sleep_from: None,
            resume_wait: None,
            resumed: None,
            sleep_shown: -1,
            light_rgb: (0, 0, 0),
            media,
            playing,
            spotify,
            music,
            sp_id: String::new(),
            music_stale: false,
            perf: None,
            perf_at: -1e9,
            perf_shown: String::new(),
            frames: Default::default(),
            xbox,
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
        self.social_refresh(now, 0.0);
        if !self.qa {
            // The mute LED shows the microphone from the start.
            self.audio.mic = audio::mic_muted();
        }
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
            r.item = library::identify(&items, &r.win.exe).or_else(|| library::identify_name(&items, &r.win.app));
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
        // Music is for people with Spotify (installed, or signed in).
        if !self.music.configured && !self.items.iter().any(|i| i.source == "start" && i.name.eq_ignore_ascii_case("spotify")) {
            t.retain(|(k, _)| *k != "music");
        }
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
        self.grid = !matches!(self.tab.as_str(), "home" | "social" | "music");
        if self.tab == "social" {
            rows = self.social_rows();
        } else if self.tab == "music" {
            rows = self.music_rows();
        } else if self.tab == "home" {
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
            let voice = self.voice_tiles();
            let n = voice.len();
            push("voice", "Discord voice", voice, n);
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
                if cat == "media" {
                    // Spotify after Media: Liked Songs and the playlists, then the Music tab.
                    let picks: Vec<&Pick> = self.music.library.iter().filter(|p| p.kind != spotify::Kind::Album).collect();
                    let n = self.music.library.len();
                    let mut tiles: Vec<Tile> = picks.iter().take(12).map(|p| Tile::Music(Mu::Play(p.uri.clone()))).collect();
                    if n > tiles.len() {
                        tiles.push(Tile::Nav { cat: "music".into(), count: n });
                    }
                    push("music", "Music", tiles, n);
                }
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
            Tile::Social(s) => format!("dc:{}", s.key()),
            Tile::Music(m) => format!("mu:{}", m.key()),
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
        if !run.win.app.is_empty() {
            return run.win.app.clone();
        }
        if !run.win.title.trim().is_empty() {
            return run.win.title.trim().to_owned();
        }
        std::path::Path::new(&run.win.exe).file_stem().map_or("App".into(), |s| s.to_string_lossy().to_string())
    }

    /// What a running app shows (a browser's page, an editor's document), for the hero's meta
    /// line. Empty for games and when the title is only the app's name.
    pub(crate) fn run_page(&self, h: isize) -> String {
        let Some(run) = self.run_of(h) else { return String::new() };
        if run.item.and_then(|i| self.items.get(i)).is_some_and(|it| it.category == "games" || it.source == "web") {
            return String::new();
        }
        let app = if run.win.app.is_empty() { self.run_name(h) } else { run.win.app.clone() };
        shell::page_title(&shell::title(h), &app)
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
            return self.home_action(now);
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
            return self.volume_step(a == Action::VolUp, now);
        }
        if matches!(self.layers.last(), Some(Layer::Quick { .. })) {
            return self.quick_action(a, now);
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
                } else if self.focus != (0, 0) && !self.rows.is_empty() {
                    // Deep in Home, Back returns to the first tile.
                    let before = self.focus;
                    self.focus = (0, 0);
                    sound::play(Sound::Back);
                    self.focus_changed(before, now);
                } else {
                    sound::play(Sound::Edge);
                }
            }
            Action::Home | Action::VolUp | Action::VolDown => {}
        }
    }

    /// The PS button while Sofa is in front: anywhere else it goes Home; on Home it opens the
    /// quick menu, and closes it again.
    fn home_action(&mut self, now: f64) {
        if matches!(self.layers.last(), Some(Layer::Quick { .. })) {
            self.layers.pop();
            sound::play(Sound::Back);
            return;
        }
        if self.tab == "home" && self.layers.is_empty() {
            // Home is current under the menu: apps that closed lose their cards.
            self.prune_running();
            return self.quick_menu(now);
        }
        if self.go_home(now) {
            sound::play(Sound::Back);
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
        if tab == "social" || tab == "home" {
            self.social_refresh(now, 30_000.0);
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
                r.item = library::identify(&items, &r.win.exe).or_else(|| library::identify_name(&items, &r.win.app));
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
        // The streaming window Sofa opened is that service (YouTube), not "Google Chrome".
        let media = (self.shell.media == Some(w.hwnd)).then(|| self.media_item.as_ref().and_then(|id| self.items.iter().position(|it| &it.id == id))).flatten();
        let item = media.or_else(|| library::identify(&self.items, &w.exe)).or_else(|| library::identify_name(&self.items, &w.app));
        if strict && item.is_none() {
            return false;
        }
        // The same window keeps the time it was first seen running.
        let since = self.running.iter().find(|r| r.win.hwnd == w.hwnd).and_then(|r| r.since).or_else(|| shell::started(w.hwnd));
        self.running.retain(|r| r.win.hwnd != w.hwnd && (item.is_none() || r.item != item));
        let icon = (item.is_none() && !w.exe.is_empty()).then(|| library::icon_cache(&self.cache, std::path::Path::new(&w.exe)));
        #[cfg(target_os = "linux")]
        let icon = icon.map(|png| crate::linux::wm::running_icon(&w, png, self.qa));
        if let Some(png) = &icon
            && !png.exists()
            && !self.qa
        {
            spawn_scan(&self.cache, &[(w.exe.clone(), png.clone())], false, self.shell.sofa, WM_ICON);
        }
        self.running.insert(0, Run { win: w, item, icon, since });
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
            Some(Tile::Running(h)) => self.resume(h, now),
            Some(Tile::Social(s)) => self.social_action(s, now),
            Some(Tile::Music(m)) => self.music_action(m),
            None => {}
        }
    }

    /// Brings a Running window forward once the button (or key) that chose it is up: a game
    /// given the focus with ✕ still down reads it as a fresh press (the character jumps), and so
    /// does Steam Input switching from its desktop layout to the game's.
    fn resume(&mut self, h: isize, now: f64) {
        if self.run_of(h).is_none() {
            return;
        }
        sound::play(Sound::Select);
        self.resume_wait = Some((h, now, None));
    }

    fn resume_tick(&mut self, now: f64) {
        let Some((h, at, up)) = self.resume_wait else { return };
        let up = if self.input.anything_down(now, self.qa) { None } else { Some(up.unwrap_or(now)) };
        if !input::let_go(at, up, now) {
            self.resume_wait = Some((h, at, up));
            return;
        }
        self.resume_wait = None;
        self.resumed = Some(h);
        crate::log(&format!("resume {h:#x} after {:.0} ms", now - at));
        if !self.qa && self.run_of(h).is_some() {
            self.shell.hand_off();
            platform::focus_id(h);
        }
        crate::log(&self.state_line());
    }

    /// A window waits for its press to end (the loop keeps waking to check).
    pub fn resume_waiting(&self) -> bool {
        self.resume_wait.is_some()
    }

    fn open_item(&mut self, i: usize, now: f64) {
        let it = self.items[i].clone();
        if let Launch::System { action, uri } = &it.launch {
            return self.system(&it, action.clone(), uri.clone(), now);
        }
        // Like a console: picking something that is already running resumes it.
        if let Some(h) = self.running.iter().find(|r| r.item == Some(i)).map(|r| r.win.hwnd) {
            return self.resume(h, now);
        }
        // Open without a card (started before Sofa, from Steam or the desktop): switch to its
        // window instead of starting it again. Some apps answer a second start badly (Spotify asks
        // to force-close itself when its first copy is slow to reply), and Steam answers a game
        // that runs already by switching to it while ✕ is still down.
        if !self.qa
            && let Some(h) = self.window_of(&it)
        {
            if self.add_running(shell::window(h), false) && !self.layers_hold_items() {
                self.rebuild(true);
            }
            return self.resume(h, now);
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

    /// The open window of a library item that has no Running card: an app's own program, or a
    /// program in a game's folder.
    fn window_of(&self, it: &Item) -> Option<isize> {
        // A desktop entry: the window of the program it starts.
        #[cfg(target_os = "linux")]
        if let Launch::AppsFolder(id) = &it.launch {
            return crate::linux::desktop::window_for(std::path::Path::new(id));
        }
        if let Launch::AppsFolder(id) = &it.launch {
            let name = std::path::Path::new(id).file_name().map(|n| n.to_string_lossy().to_lowercase()).filter(|n| n.ends_with(".exe"))?;
            return shell::find_window(&name);
        }
        let dir = format!("{}{}", it.install_path.as_ref()?, std::path::MAIN_SEPARATOR);
        shell::top_windows().into_iter().find(|&h| h != self.shell.sofa && shell::exe_of(shell::pid(h)).to_lowercase().starts_with(&dir))
    }

    fn start(&mut self, i: usize, now: f64) {
        let it = self.items[i].clone();
        let ok = self.qa
            || match &it.launch {
                Launch::Web { url, tv } => {
                    let (ok, browser) = platform::open_web(&self.data, self.store.text("browser"), url.as_deref(), *tv);
                    match browser {
                        // The launch screen stays on top until the browser is ready.
                        Some(exe) => {
                            self.shell.cover(&exe, now);
                            self.media_item = Some(it.id.clone());
                        }
                        None => self.shell.step_down(now),
                    }
                    ok
                }
                launch => {
                    self.shell.step_down(now);
                    match launch {
                        Launch::Url(u) => platform::open(u),
                        Launch::AppsFolder(id) => platform::apps_folder(id),
                        Launch::Exec { exe, args } => platform::run(exe, args),
                        Launch::Web { .. } | Launch::System { .. } => false,
                    }
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
            self.toast(&format!("Couldn't open {}", it.name), if cfg!(windows) { "Windows didn't start it." } else { "It didn't start. Try it from the desktop." });
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
        shell::minimize(self.shell.sofa);
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
            Some(Tile::Music(Mu::Play(uri))) => {
                let Some(p) = self.pick(&uri).cloned() else { return };
                let opts = vec![
                    Opt::new("Play", Act::SpPlay(uri.clone(), false)).icon("play"),
                    Opt::new("Shuffle play", Act::SpPlay(uri.clone(), true)).icon("rescan"),
                    Opt::new("Open in Spotify", Act::SpOpen(uri)).icon("app"),
                ];
                sound::play(Sound::Open);
                self.layers.push(Layer::Sheet { title: p.name.clone(), sub: pick_meta(&p, self.music.user.as_deref()), opts, focus: 0, kind: SheetKind::Music });
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

    /// The controller's Home button by name: the PS button, or the Xbox button on an Xbox pad.
    pub(crate) fn guide_name(&self) -> &'static str {
        if self.input.kind == PadKind::Xbox { "Xbox button" } else { "PS button" }
    }

    /// The Home button as set: the guide button, or the two buttons pressed together.
    pub(crate) fn home_name(&self) -> &'static str {
        match (self.store.text("homeButton") == "combo", self.input.kind == PadKind::Xbox) {
            (true, true) => "View + Menu",
            (true, false) => "Create + Options",
            (false, true) => "Xbox button",
            (false, false) => "PS button",
        }
    }

    /// "Press the PS button", "Press View + Menu".
    pub(crate) fn home_press(&self) -> String {
        if self.store.text("homeButton") == "combo" { format!("Press {}", self.home_name()) } else { format!("Press the {}", self.home_name()) }
    }

    /// The DualSense's lightbar setting (`controllerLight`, as in 1.x).
    pub(crate) fn light_name(&self) -> &'static str {
        match self.store.text("controllerLight") {
            "amber" => "Amber",
            "off" => "Off",
            _ => "Game's colour",
        }
    }

    fn settings_opts(&self) -> Vec<Opt> {
        let browser = platform::chosen_browser(self.store.text("browser")).map_or("None found", |b| b.name);
        let autostart = if self.qa { self.store.flag("launchAtLogin") } else { install::autostart() };
        let mut opts = vec![
            Opt::new(START_WITH, Act::Toggle("launchAtLogin")).switch(autostart),
            Opt::new("Keep Sofa in front", Act::Toggle("keepInFront")).switch(self.store.flag("keepInFront")),
            Opt::new("Home button", Act::HomeButton).value(self.home_name()),
            Opt::new("Button layout", Act::Layout).value(self.layout_name()),
            Opt::new("Controller light", Act::Light).value(self.light_name()),
            Opt::new(format!("Keep Steam off the {}", self.guide_name()), Act::Toggle("steamGuide")).switch(self.store.flag("steamGuide")),
            Opt::new("Browser", Act::Browser).value(browser),
            Opt::new("Interface sounds", Act::Toggle("sounds")).switch(self.store.flag("sounds")),
            Opt::new("Reduce motion", Act::Toggle("motion")).switch(self.store.text("motion") == "reduced"),
            Opt::new("Show hidden apps", Act::Toggle("showHidden")).switch(self.store.flag("showHidden")),
            Opt::new("Rescan library", Act::Rescan).icon("rescan"),
            Opt::new("Go to desktop", Act::Desktop).icon("desktop").value(format!("{} returns", self.home_name())),
            Opt::new("Quit Sofa", Act::Quit).icon("quit"),
        ];
        // An Xbox pad: Game Bar also wants its button (Windows' setting, shown as it is now).
        if self.xbox && !self.qa {
            opts.insert(6, Opt::new("Keep Game Bar off the Xbox button", Act::Toggle("gameBarOff")).switch(!install::game_bar_button()));
        }
        opts
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
        let mut opts = vec![
            Opt::new(START_WITH, Act::Toggle("launchAtLogin")).switch(self.store.flag("launchAtLogin")),
            Opt::new(format!("Keep Steam off the {}", self.guide_name()), Act::Toggle("steamGuide")).switch(self.store.flag("steamGuide")),
        ];
        if self.xbox && !self.qa {
            opts.push(Opt::new("Keep Game Bar off the Xbox button", Act::Toggle("gameBarOff")).switch(self.store.flag("gameBarOff")));
        }
        opts.push(Opt::new("Button layout", Act::Layout).value(self.layout_name()));
        opts.push(Opt::new("Continue", Act::WelcomeDone).icon("check"));
        opts
    }

    fn welcome(&mut self, now: f64) {
        let opts = self.welcome_opts();
        let focus = opts.len() - 1;
        self.layers.push(Layer::Welcome { opts, focus, at: now });
    }

    /// The first-run screen follows its settings, focus kept on the same choice.
    fn refresh_welcome(&mut self) {
        let fresh = self.welcome_opts();
        if let Some(Layer::Welcome { opts, focus, .. }) = self.layers.iter_mut().find(|l| matches!(l, Layer::Welcome { .. })) {
            let act = opts.get(*focus).map(|o| std::mem::discriminant(&o.act));
            *opts = fresh;
            if let Some(i) = act.and_then(|d| opts.iter().position(|o| std::mem::discriminant(&o.act) == d)) {
                *focus = i;
            }
            *focus = (*focus).min(opts.len() - 1);
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
        match kb.purpose {
            KbPurpose::SpId => {
                self.sp_id = kb.text.trim().to_owned();
                self.refresh_sp_setup(3);
            }
            KbPurpose::DcId => {
                self.dc.client_id = kb.text.trim().to_owned();
                self.focus_setup(3);
            }
            KbPurpose::DcSecret => {
                if !kb.text.trim().is_empty() {
                    self.dc.secret = Some(kb.text.trim().to_owned());
                }
                self.focus_setup(4);
            }
            _ => {}
        }
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
        if a == Action::Back && self.layers.last().is_some_and(is_sleep_warning) {
            // Backing out of the warning keeps the PC awake rather than letting it sleep.
            sound::play(Sound::Back);
            return self.run(Act::StayAwake, now);
        }
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
            "gameBarOff" => {
                // After setup it changes Windows' setting at once, from what it is now.
                let off = if self.qa || !self.store.top_flag("setupDone") { !self.store.flag(key) } else { install::game_bar_button() };
                self.store.set(key, Value::Bool(off));
                if !self.qa && self.store.top_flag("setupDone") {
                    install::set_game_bar_button(!off);
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
                self.resume(h, now);
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
                if key == "perfOverlay" {
                    self.read_perf(now, true);
                }
            }
            Act::Info => sound::play(Sound::Edge),
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
            Act::Light => self.cycle_light(),
            sp @ (Act::SpPage | Act::SpId | Act::SpConnect | Act::SpRedo | Act::SpRefresh | Act::SpForget | Act::SpForgetNow | Act::SpPlay(..) | Act::SpOpen(_)) => self.sp_run(sp, now),
            Act::Output(id) => {
                let ok = self.qa || audio::set_output(&id);
                self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::Sound, .. }));
                if ok {
                    self.audio.current = Some(id);
                    self.audio_refresh();
                } else {
                    self.toast("Couldn't switch the sound", if cfg!(windows) { "Windows didn't accept that output. Try it in Windows' sound settings." } else { "The sound server didn't accept that output. Try it in the sound settings." });
                }
            }
            Act::System(key) => {
                let Some(it) = self.items.iter().find(|i| i.id == format!("sys:{key}")).cloned() else { return };
                if let Launch::System { action, uri } = &it.launch {
                    if key == "sleep" {
                        // Wake up to Home, not to an open menu.
                        self.layers.retain(|l| matches!(l, Layer::Welcome { .. }));
                    }
                    self.system(&it, action.clone(), uri.clone(), now);
                }
            }
            Act::StayAwake => {
                self.layers.retain(|l| !is_sleep_warning(l));
                self.sleep_at = None;
                self.sleep_warned = false;
                self.toast("Sleep timer off", "The PC stays on.");
                if let Some(h) = self.sleep_from.take() {
                    self.resume(h, now);
                }
            }
            Act::SleepNow => {
                self.sleep_at = Some(epoch_ms());
                self.sleep_tick(now);
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
                    if self.xbox && self.store.flag("gameBarOff") {
                        install::set_game_bar_button(false);
                    }
                }
                let body = format!("{} in any game to come back here, and on Home for the quick menu.", self.home_press());
                self.toast("Sofa is ready", &body);
            }
            Act::Close => {
                self.layers.pop();
            }
            dc @ (Act::DcPage | Act::DcId | Act::DcSecret | Act::DcConnect | Act::DcRedo | Act::DcForget | Act::DcForgetNow | Act::DcJoin(_) | Act::DcLeave) => self.dc_run(dc),
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
            1 => self.toast(&format!("Steam no longer takes the {}", self.guide_name()), "Steam restarted in the background."),
            2 => self.toast(&format!("Steam no longer takes the {}", self.guide_name()), "Sofa turned off \"Guide button focuses Steam\"."),
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
            title: format!("Steam is also using the {}", self.guide_name()),
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
            if !self.qa {
                // The microphone may have been muted elsewhere: the LED and the quick menu follow.
                self.audio.mic = audio::mic_muted();
            }
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
            // Something else took the screen first: nothing comes forward over it, and the keys
            // that were down come up in that window.
            self.resume_wait = None;
            self.input.keys_released();
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
        // An Xbox pad (after the first poll XInput is asked, not assumed): Settings offers to
        // keep Game Bar off its button.
        if x && !self.qa && !self.xbox {
            self.xbox = self.input.xbox_now();
        }
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
            && !self.shell.covering()
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
        if self.music_stale && !self.layers_hold_items() {
            self.music_stale = false;
            if self.tab == "home" || self.tab == "music" {
                self.rebuild(true);
            }
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
        // The Performance panel, its card and the overlay show numbers read once a second.
        let perf_up = self.store.flag("perfOverlay") || self.layers.iter().any(|l| matches!(l, Layer::Quick { .. } | Layer::Sheet { kind: SheetKind::Perf, .. }));
        if perf_up {
            self.read_perf(now, false);
        }
        self.watch_pads(now);
        self.resume_tick(now);
        self.sleep_tick(now);
        self.lights_tick(now);
        let m = self.motion() > 0.0;
        let hero_moving = m && self.hero.as_ref().is_some_and(|(_, t)| now - t < 1250.0);
        let welcome = self.layers.iter().any(|l| matches!(l, Layer::Welcome { at, .. } if now - at < 900.0));
        let quick = self.layers.iter().any(|l| matches!(l, Layer::Quick { at, .. } if now - at < 420.0));
        self.dirty
            || now - self.boot_at < 1000.0
            || welcome
            || quick
            || now - self.vol_at < 1800.0
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
            Some(Layer::Quick { focus, .. }) => format!("quick:{}", focus.key()),
        };
        format!(
            "state|tab={}|focus={},{}|tile={}|layer={}|rows={}|items={}|running={}|toast={}|sleep={}|volume={}|output={}|mic={}|music={}|resume={}|rowkeys={}",
            self.tab,
            self.focus.0,
            self.focus.1,
            self.focused_tile_id().unwrap_or_default(),
            layer,
            self.rows.len(),
            self.items.len(),
            self.running.len(),
            self.toast.as_ref().map_or("", |t| t.0.as_str()),
            self.sleep_left().map_or("off".to_owned(), |ms| format!("{:.0}", (ms / 60_000.0).ceil())),
            self.audio.volume.map_or("-".to_owned(), |v| format!("{:.0}", v * 100.0)),
            self.audio.current.as_deref().unwrap_or("-"),
            if self.mic_muted() { "muted" } else { "on" },
            self.playing.as_ref().map_or("-", |p| if p.playing { "playing" } else { "paused" }),
            match (self.resume_wait, self.resumed) {
                (Some((h, ..)), _) => format!("wait:{h}"),
                (None, Some(h)) => format!("done:{h}"),
                (None, None) => "-".to_owned(),
            },
            self.rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>().join(",")
        )
    }

    pub fn save(&mut self) {
        self.store.flush();
    }
}

#[cfg(windows)]
pub(crate) fn clock_minute() -> u16 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: plain call.
    let t = unsafe { GetLocalTime() };
    t.wHour * 60 + t.wMinute
}

#[cfg(target_os = "linux")]
pub(crate) fn clock_minute() -> u16 {
    crate::linux::clock::minute_of_day()
}

impl App {
    /// The browser Sofa covered is in front and drawn (`h`), or never showed (0).
    pub fn media_ready(&mut self, h: isize) {
        if h == 0 {
            self.media_item = None;
        }
        self.dirty = true;
    }

    /// Whether the controller is the remote of the TV window Sofa opened (YouTube's TV
    /// interface, in front). Sites built for a mouse (Netflix, Crunchyroll) keep whatever
    /// the person uses there, such as Steam's stick-as-mouse.
    pub fn remote_active(&self) -> bool {
        let tv = self.media_item.as_ref().and_then(|id| self.items.iter().find(|i| &i.id == id)).is_some_and(|i| matches!(i.launch, Launch::Web { tv: true, .. }));
        !self.qa && !self.focused && tv && self.shell.media_in_front()
    }

    /// A controller action while the streaming window is in front: the remote's key
    /// (✕ select, ○ back, D-pad and stick move, □ play/pause, L2/R2 volume).
    pub fn remote_action(&mut self, a: Action) {
        #[cfg(windows)]
        {
            use windows::Win32::UI::Input::KeyboardAndMouse::*;
            let vk = match a {
                Action::Up => VK_UP,
                Action::Down => VK_DOWN,
                Action::Left => VK_LEFT,
                Action::Right => VK_RIGHT,
                Action::Accept => VK_RETURN,
                Action::Back => VK_ESCAPE,
                Action::Search => VK_MEDIA_PLAY_PAUSE,
                Action::VolUp | Action::VolDown => return volume(a == Action::VolUp),
                _ => return,
            };
            send_key(vk.0);
        }
        #[cfg(target_os = "linux")]
        {
            // X11 keysyms: arrows, Return, Escape, XF86AudioPlay.
            let sym = match a {
                Action::Up => 0xff52,
                Action::Down => 0xff54,
                Action::Left => 0xff51,
                Action::Right => 0xff53,
                Action::Accept => 0xff0d,
                Action::Back => 0xff1b,
                Action::Search => 0x1008_ff14,
                Action::VolUp | Action::VolDown => return volume(a == Action::VolUp),
                _ => return,
            };
            crate::linux::x11::send_key(sym);
        }
    }
}

/// How a Discord tile looks: its name and second line, a line icon or Discord's mark, an
/// avatar or server icon when Discord has one, and whether its state is on (muted).
pub(crate) struct SocFace {
    pub name: String,
    pub sub: String,
    pub glyph: Option<&'static str>,
    pub brand: bool,
    pub on: bool,
    pub image: Option<PathBuf>,
}

/// A Discord application's Client ID: a long number.
#[allow(non_snake_case)]
fn DISCORD_ID(s: &str) -> bool {
    (17..=21).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// A Client Secret: 24 to 64 letters, digits, `_` and `-`.
#[allow(non_snake_case)]
fn DISCORD_SECRET(s: &str) -> bool {
    (24..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Discord voice (a port of Sofa 1.x's Social screens, `renderer/app.js`).
impl App {
    /// Asks the `discord` thread for fresh state when the last ask is older than `max_age` ms.
    pub(crate) fn social_refresh(&mut self, now: f64, max_age: f64) {
        if self.voice.configured && now - self.social_at >= max_age {
            self.social_at = now;
            self.social.send(Req::Refresh(false));
        }
    }

    /// WM_SOCIAL: Discord's state changed.
    pub fn social_changed(&mut self, now: f64) {
        if let Some(cfg) = self.social.take_save() {
            self.store.map_mut("social").insert("discord".into(), cfg);
            self.store.flush();
        }
        let v = self.social.snapshot();
        if v == self.voice {
            return;
        }
        let was = std::mem::replace(&mut self.voice, v);
        // Discord's "Authorize" prompt shows in Discord's window: Sofa steps aside for it, and
        // comes back once it was answered.
        if self.voice.authorizing && !self.dc_authorizing {
            self.dc_authorizing = true;
            self.shell.hand_off();
            if let Some(h) = shell::find_window("discord.exe") {
                platform::focus_id(h);
            }
        } else if !self.voice.authorizing && self.dc_authorizing {
            self.dc_authorizing = false;
            platform::focus_id(self.shell.sofa);
        }
        if self.dc.waiting && !self.voice.busy && !self.voice.authorizing {
            self.dc.waiting = false;
            if self.voice.ready {
                self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::DcSetup, .. }));
                let who = self.voice.user.clone().unwrap_or_else(|| "your account".into());
                self.toast("Discord connected", &format!("Signed in as {who}"));
            } else {
                self.dc.msg = self.voice.error.clone().unwrap_or_else(|| "Discord didn't answer. Is it open and signed in?".into());
            }
        }
        self.refresh_dc_sheets();
        let rows_changed = was.current != self.voice.current
            || was.mute != self.voice.mute
            || was.deaf != self.voice.deaf
            || was.recent != self.voice.recent
            || was.guilds != self.voice.guilds
            || was.ready != self.voice.ready
            || was.configured != self.voice.configured
            || was.error != self.voice.error;
        if rows_changed && (self.tab == "home" || self.tab == "social") && !self.layers_hold_items() {
            self.rebuild(true);
            if let Some(id) = self.focused_tile_id() {
                self.show_hero(id, now);
            }
        }
        self.dirty = true;
    }

    /// Home's row: the channel you are in with its controls, then channels you joined recently.
    fn voice_tiles(&self) -> Vec<Tile> {
        let v = &self.voice;
        if !v.configured || !v.ready {
            return Vec::new();
        }
        let mut out = Vec::new();
        if v.current.is_some() {
            out.extend([Soc::Here, Soc::Mute, Soc::Deaf, Soc::Leave].map(Tile::Social));
        }
        let here = v.current.as_ref().map(|c| c.id.as_str());
        out.extend(v.recent.iter().filter(|c| Some(c.id.as_str()) != here).map(|c| Tile::Social(Soc::Join(c.id.clone()))));
        out
    }

    /// The Social tab: your channel (who is in it, mute, deafen, leave), your servers, or
    /// the way to connect.
    fn social_rows(&self) -> Vec<Row> {
        let v = &self.voice;
        let mut rows = Vec::new();
        if let Some(c) = &v.current {
            let mut tiles: Vec<Tile> = c.members.iter().map(|m| Tile::Social(Soc::Member(m.id.clone()))).collect();
            tiles.extend([Soc::Mute, Soc::Deaf, Soc::Leave].map(Tile::Social));
            rows.push(Row { key: "voice".into(), title: c.name.clone(), count: c.members.len(), tiles });
        }
        if v.ready && !v.guilds.is_empty() {
            let mut tiles: Vec<Tile> = v.guilds.iter().map(|g| Tile::Social(Soc::Server(g.id.clone()))).collect();
            tiles.push(Tile::Social(Soc::Account));
            rows.push(Row { key: "servers".into(), title: "Discord servers".into(), count: v.guilds.len(), tiles });
        } else if v.configured && v.not_running {
            rows.push(Row { key: "dsetup".into(), title: "Discord".into(), count: 0, tiles: vec![Tile::Social(Soc::Open)] });
        } else {
            rows.push(Row { key: "dsetup".into(), title: "Discord".into(), count: 0, tiles: vec![Tile::Social(Soc::Setup)] });
        }
        rows
    }

    pub(crate) fn soc_face(&self, s: &Soc) -> SocFace {
        let v = &self.voice;
        let face = |name: String, sub: String, glyph: Option<&'static str>, brand: bool, on: bool, image: Option<PathBuf>| SocFace { name, sub, glyph, brand, on, image };
        let in_voice = |n: usize| if n == 1 { "1 in voice".to_owned() } else { format!("{n} in voice") };
        match s {
            Soc::Here => {
                let c = v.current.clone().unwrap_or_default();
                face(c.name, in_voice(c.members.len()), None, true, true, None)
            }
            Soc::Mute => face(if v.mute { "Unmute" } else { "Mute" }.into(), if v.mute { "Muted".into() } else { "Microphone on".into() }, Some(if v.mute { "micOff" } else { "mic" }), false, v.mute, None),
            Soc::Deaf => face(if v.deaf { "Undeafen" } else { "Deafen" }.into(), if v.deaf { "Deafened".into() } else { "Hearing voice".into() }, Some("headphones"), false, v.deaf, None),
            Soc::Leave => face("Leave voice".into(), v.current.as_ref().map(|c| c.name.clone()).unwrap_or_default(), Some("close"), false, false, None),
            Soc::Join(id) => {
                let c = v.recent.iter().find(|c| &c.id == id).cloned().unwrap_or_default();
                let sub = if c.members.is_empty() { if c.guild.is_empty() { "Empty".into() } else { c.guild.clone() } } else { in_voice(c.members.len()) };
                let image = c.members.iter().find_map(|m| m.avatar.clone());
                face(c.name, sub, Some("sound"), false, false, image)
            }
            Soc::Server(id) => {
                let g = v.guilds.iter().find(|g| &g.id == id).cloned().unwrap_or_default();
                face(g.name, "Voice channels".into(), Some("users"), false, false, g.icon)
            }
            Soc::Member(id) => {
                let m = v.current.as_ref().and_then(|c| c.members.iter().find(|m| &m.id == id)).cloned().unwrap_or_default();
                face(m.name, if m.muted { "Muted".into() } else { "In voice".into() }, Some("users"), false, false, m.avatar)
            }
            Soc::Account => face("Discord account".into(), v.user.clone().unwrap_or_else(|| "Connected".into()), None, true, false, None),
            Soc::Setup => {
                let err = v.configured.then(|| v.error.clone()).flatten();
                face(if err.is_some() { "Reconnect Discord" } else { "Connect Discord" }.into(), err.unwrap_or_else(|| "Join voice from the couch".into()), None, true, false, None)
            }
            Soc::Open => face("Open Discord".into(), "Discord needs to be running".into(), None, true, false, None),
        }
    }

    fn social_action(&mut self, s: Soc, now: f64) {
        sound::play(Sound::Select);
        match s {
            // The tile flips at once; Discord's answer confirms it.
            Soc::Mute => {
                self.voice.mute = !self.voice.mute;
                self.social.send(Req::SetVoice { mute: Some(self.voice.mute), deaf: None });
            }
            Soc::Deaf => {
                self.voice.deaf = !self.voice.deaf;
                self.social.send(Req::SetVoice { mute: None, deaf: Some(self.voice.deaf) });
            }
            Soc::Leave => self.social.send(Req::Join(None)),
            Soc::Join(id) => self.social.send(Req::Join(Some(id))),
            Soc::Here | Soc::Member(_) => self.voice_sheet(),
            Soc::Server(id) => {
                let name = self.voice.guilds.iter().find(|g| g.id == id).map(|g| g.name.clone()).unwrap_or_default();
                self.dc_server = Some(id.clone());
                self.social.send(Req::Server(id));
                let loading = vec![Opt::new("Loading voice channels", Act::Close).icon("sound")];
                self.layers.push(Layer::Sheet { title: name, sub: "Voice channels".into(), opts: loading, focus: 0, kind: SheetKind::DcServer });
            }
            Soc::Account => self.discord_account(),
            Soc::Setup => self.discord_setup(),
            Soc::Open => match self.items.iter().position(|i| i.source == "start" && i.name.eq_ignore_ascii_case("discord")) {
                Some(i) => self.open_item(i, now),
                None => self.layers.push(Layer::Dialog {
                    title: "Discord isn't installed".into(),
                    body: "Install Discord on this PC, then open it once and sign in.".into(),
                    opts: vec![Opt::new("OK", Act::Close).icon("check")],
                    focus: 0,
                }),
            },
        }
        self.dirty = true;
        if self.tab == "home" || self.tab == "social" {
            self.rebuild(true);
        }
    }

    fn setup_opts(&self) -> Vec<Opt> {
        let id = &self.dc.client_id;
        let has_secret = self.dc.secret.is_some() || self.voice.configured;
        let step = |done: bool, icon: &'static str| if done { "check" } else { icon };
        let short = if id.len() > 6 { format!("{}…", &id[..6]) } else { id.clone() };
        let connect = if self.dc.waiting {
            "Waiting for Discord. Approve Sofa there.".to_owned()
        } else if !self.dc.msg.is_empty() {
            self.dc.msg.clone()
        } else {
            "Discord asks you to Authorize Sofa once.".to_owned()
        };
        vec![
            Opt::new("Make a Discord application", Act::DcPage).icon(step(!id.is_empty(), "globe")).detail("Opens discord.com/developers. Choose New Application and name it Sofa."),
            Opt::new("Add the redirect", Act::DcPage).icon(step(!id.is_empty(), "link")).detail(format!("In OAuth2, add the redirect {} and save.", social::REDIRECT)),
            Opt::new("Paste the Client ID", Act::DcId).icon(step(!id.is_empty(), "rename")).detail("Copy it from OAuth2, then select this.").value(short),
            Opt::new("Paste the Client Secret", Act::DcSecret).icon(step(has_secret, "rename")).detail("Reset Secret, copy it, then select this.").value(if has_secret { "Saved" } else { "" }),
            Opt::new("Connect", Act::DcConnect).icon("play").detail(connect),
        ]
    }

    /// Connect Discord: one-time setup, step by step (1.x's walkthrough).
    fn discord_setup(&mut self) {
        sound::play(Sound::Open);
        if self.dc.client_id.is_empty() {
            self.dc.client_id = self.store.map("social").get("discord").and_then(|d| d.get("clientId")).and_then(Value::as_str).unwrap_or("").to_owned();
        }
        self.dc.msg.clear();
        let opts = self.setup_opts();
        let focus = if self.dc.client_id.is_empty() { 0 } else if self.dc.secret.is_none() && !self.voice.configured { 3 } else { 4 };
        self.layers.push(Layer::Sheet {
            title: "Connect Discord".into(),
            sub: "One-time setup, about two minutes: Discord only lets apps you made control voice.".into(),
            opts,
            focus,
            kind: SheetKind::DcSetup,
        });
    }

    fn discord_account(&mut self) {
        sound::play(Sound::Open);
        let who = self.voice.user.clone().unwrap_or_else(|| "your account".into());
        self.layers.push(Layer::Sheet {
            title: "Discord".into(),
            sub: format!("Signed in as {who}. Sofa stays signed in until you disconnect."),
            opts: vec![Opt::new("Set up again", Act::DcRedo).icon("restart"), Opt::new("Disconnect", Act::DcForget).icon("close")],
            focus: 0,
            kind: SheetKind::DcAccount,
        });
    }

    fn voice_opts(&self) -> Vec<Opt> {
        let mut opts: Vec<Opt> = self
            .voice
            .current
            .iter()
            .flat_map(|c| &c.members)
            .map(|m| {
                let o = Opt::new(m.name.clone(), Act::Close).icon("users");
                if m.muted { o.value("Muted") } else { o }
            })
            .collect();
        opts.push(Opt::new("Leave voice", Act::DcLeave).icon("close"));
        opts
    }

    fn voice_sheet(&mut self) {
        let Some(c) = self.voice.current.clone() else { return };
        sound::play(Sound::Open);
        let n = c.members.len();
        self.layers.push(Layer::Sheet { title: c.name, sub: format!("{n} in voice"), opts: self.voice_opts(), focus: 0, kind: SheetKind::DcVoice });
    }

    /// Open Discord sheets follow what Discord says.
    fn refresh_dc_sheets(&mut self) {
        let setup = self.setup_opts();
        let voice = self.voice_opts();
        let server = match (&self.dc_server, &self.voice.server) {
            (Some(want), Some((got, list))) if want == got => Some(if list.is_empty() {
                vec![Opt::new("No voice channels you can join", Act::Close).icon("sound")]
            } else {
                let here = self.voice.current.as_ref().map(|c| c.id.clone());
                list.iter()
                    .map(|c| {
                        let n = c.members.len();
                        let value = if Some(&c.id) == here.as_ref() { "You're here".to_owned() } else if n > 0 { format!("{n} in voice") } else { String::new() };
                        Opt::new(c.name.clone(), Act::DcJoin(c.id.clone())).icon("sound").value(value)
                    })
                    .collect()
            }),
            _ => None,
        };
        let in_voice = self.voice.current.is_some();
        for l in &mut self.layers {
            if let Layer::Sheet { opts, focus, kind, .. } = l {
                let fresh = match kind {
                    SheetKind::DcSetup => Some(setup.clone()),
                    SheetKind::DcVoice if in_voice => Some(voice.clone()),
                    SheetKind::DcServer => server.clone(),
                    _ => None,
                };
                if let Some(f) = fresh {
                    *focus = (*focus).min(f.len().saturating_sub(1));
                    *opts = f;
                }
            }
        }
        if !in_voice {
            self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::DcVoice, .. }));
        }
    }

    /// The Discord options of the sheets above.
    fn dc_run(&mut self, act: Act) {
        match act {
            Act::DcPage => {
                if !self.qa {
                    self.shell.step_down(0.0);
                    platform::open(social::DEVELOPERS);
                }
            }
            Act::DcId => {
                // Copied on the PC: selecting the step pastes it.
                let clip = platform::clipboard_text().trim().to_owned();
                if DISCORD_ID(&clip) {
                    self.dc.client_id = clip;
                    self.dc.msg.clear();
                    self.focus_setup(3);
                } else {
                    let id = self.dc.client_id.clone();
                    self.keyboard(KbPurpose::DcId, id);
                }
            }
            Act::DcSecret => {
                let clip = platform::clipboard_text().trim().to_owned();
                if DISCORD_SECRET(&clip) && !DISCORD_ID(&clip) {
                    self.dc.secret = Some(clip);
                    self.dc.msg.clear();
                    self.focus_setup(4);
                } else {
                    self.keyboard(KbPurpose::DcSecret, String::new());
                }
            }
            Act::DcConnect => {
                if !DISCORD_ID(&self.dc.client_id) {
                    self.dc.msg = "The Client ID is a long number from the OAuth2 page.".into();
                    return self.focus_setup(2);
                }
                if self.dc.secret.is_none() && !self.voice.configured {
                    self.dc.msg = "Paste the Client Secret first.".into();
                    return self.focus_setup(3);
                }
                self.dc.msg.clear();
                self.dc.waiting = true;
                self.social.send(Req::Configure { client_id: self.dc.client_id.clone(), secret: self.dc.secret.take() });
                self.refresh_dc_sheets();
            }
            Act::DcRedo => {
                self.layers.pop();
                self.discord_setup();
            }
            Act::DcForget => self.layers.push(Layer::Dialog {
                title: "Disconnect Discord?".into(),
                body: "Sofa forgets the application and its sign-in.".into(),
                opts: vec![Opt::new("Disconnect", Act::DcForgetNow).icon("close"), Opt::new("Cancel", Act::Close).icon("restart")],
                focus: 1,
            }),
            Act::DcForgetNow => {
                self.layers.retain(|l| !matches!(l, Layer::Dialog { .. } | Layer::Sheet { kind: SheetKind::DcAccount, .. }));
                self.dc = DcSetup::default();
                self.social.send(Req::Forget);
            }
            Act::DcJoin(id) => {
                self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::DcServer, .. }));
                self.social.send(Req::Join(Some(id)));
            }
            Act::DcLeave => {
                self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::DcVoice, .. }));
                self.social.send(Req::Join(None));
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn focus_setup(&mut self, step: usize) {
        self.refresh_dc_sheets();
        if let Some(Layer::Sheet { focus, kind: SheetKind::DcSetup, .. }) = self.layers.last_mut() {
            *focus = step;
        }
    }
}

/// The quick menu, sound, the microphone, the controller's lights and the sleep timer.
impl App {
    /// The quick menu's controls, left to right. Each shows only when it can work.
    pub(crate) fn quick_ctls(&self) -> Vec<Ctl> {
        let mut v = Vec::new();
        if self.playing.is_some() {
            v.push(Ctl::Music);
        }
        if self.audio.output().is_some() {
            v.push(Ctl::Output);
        }
        if self.audio.volume.is_some() {
            v.push(Ctl::Volume);
        }
        if self.audio.mic.is_some() || self.in_voice() {
            v.push(Ctl::Mic);
        }
        v.push(Ctl::Timer);
        // The light needs a DualSense (a QA run shows it whatever is plugged in).
        if self.qa || self.input.info().connected {
            v.push(Ctl::Light);
        }
        v.push(Ctl::Perf);
        v.push(Ctl::Power);
        v
    }

    pub(crate) fn ctl_face(&self, c: Ctl) -> CtlFace {
        let face = |icon, label, value: String, on, verb| CtlFace { icon, label, value, on, level: None, verb, art: None, app: String::new() };
        match c {
            Ctl::Music => {
                let np = self.playing.clone().unwrap_or_default();
                let label = if np.playing { "Now playing" } else { "Paused" };
                CtlFace { icon: "music", label, value: np.title, on: false, level: None, verb: if np.playing { "Pause" } else { "Play" }, art: np.art, app: if np.artist.is_empty() { np.app } else { np.artist } }
            }
            Ctl::Output => match self.audio.output() {
                Some(o) => face(kind_icon(o.kind), "Sound output", o.name.clone(), false, "Choose"),
                None => face("sound", "Sound output", "None".into(), false, "Choose"),
            },
            Ctl::Volume => {
                let (v, m) = (self.audio.volume.unwrap_or(0.0), self.audio.muted);
                let value = if m { "Muted".to_owned() } else { format!("{}%", (v * 100.0).round()) };
                CtlFace { level: Some(if m { 0.0 } else { v }), ..face(if m { "volumeX" } else { "sound" }, "Volume", value, m, if m { "Unmute" } else { "Mute" }) }
            }
            Ctl::Mic => {
                let m = self.mic_muted();
                let label = if self.in_voice() { "Discord microphone" } else { "Microphone" };
                face(if m { "micOff" } else { "mic" }, label, if m { "Muted" } else { "On" }.into(), m, if m { "Unmute" } else { "Mute" })
            }
            Ctl::Timer => match self.sleep_left() {
                Some(ms) => face("timer", "Sleep timer", duration_text((ms / 60_000.0).ceil() as u32), true, "Change"),
                None => face("timer", "Sleep timer", "Off".into(), false, "Start"),
            },
            Ctl::Light => face("lightbulb", "Controller light", self.light_name().into(), false, "Change"),
            Ctl::Perf => face("gauge", "Performance", self.perf_card(), self.store.flag("perfOverlay"), "Open"),
            Ctl::Power => face("shutdown", "Power", String::new(), false, "Open"),
        }
    }

    /// Sofa's frame rate as the panel and the overlay say it ("60 fps, 3.1 ms to draw"). Sofa
    /// draws only when something changes, so with fewer than two frames in the last second it is
    /// idle (and the time a frame takes is left out, or the overlay would draw itself again each
    /// second to show the time of the frame before).
    pub(crate) fn fps_text(&self, now: f64) -> String {
        let fps = self.frames.fps(now);
        match (fps, self.frames.ms(now)) {
            (2.., Some(ms)) => format!("{fps} fps, {ms:.1} ms to draw"),
            _ => "Idle".into(),
        }
    }

    /// Reads memory again (at most once a second), and video memory with it while the Performance
    /// panel or the overlay is up; the open panel follows, and the screen is drawn again only when
    /// a number on it changed.
    fn read_perf(&mut self, now: f64, force: bool) {
        if !force && now - self.perf_at < 1000.0 {
            return;
        }
        let video = self.store.flag("perfOverlay") || self.layers.iter().any(|l| matches!(l, Layer::Sheet { kind: SheetKind::Perf, .. }));
        let usage = crate::perf::read(video);
        // Video memory is still being opened (on its own thread, for a tenth of a second or so):
        // look again soon instead of in a second.
        self.perf_at = if video && usage.gpus.is_none() { now - 800.0 } else { now };
        self.perf = Some(usage);
        let opts = self.perf_opts(now);
        // What is on screen: the quick menu's card, and the panel's or the overlay's numbers.
        let mut shown = self.perf_card();
        if video {
            for value in opts.iter().filter_map(|o| o.value.as_deref()) {
                shown.push('|');
                shown.push_str(value);
            }
        }
        if !force && shown == self.perf_shown {
            return;
        }
        self.perf_shown = shown;
        for l in &mut self.layers {
            if let Layer::Sheet { opts: o, kind: SheetKind::Perf, .. } = l {
                *o = opts.clone();
            }
        }
        self.dirty = true;
    }

    /// The quick menu card's value: "38% memory".
    fn perf_card(&self) -> String {
        self.perf.as_ref().filter(|u| u.ram_total > 0).map_or_else(String::new, |u| format!("{}% memory", crate::perf::percent(u.ram_used, u.ram_total)))
    }

    fn perf_opts(&self, now: f64) -> Vec<Opt> {
        let mut opts = vec![Opt::new("Show on screen", Act::Toggle("perfOverlay")).icon("gauge").switch(self.store.flag("perfOverlay"))];
        if let Some(u) = &self.perf {
            if u.ram_total > 0 {
                opts.push(Opt::new("Memory", Act::Info).value(format!("{}, {}%", crate::perf::of(u.ram_used, u.ram_total), crate::perf::percent(u.ram_used, u.ram_total))));
            }
            match &u.gpus {
                None => opts.push(Opt::new("Video memory", Act::Info).value("Reading")),
                Some(gpus) if gpus.is_empty() => opts.push(Opt::new("Video memory", Act::Info).value("Not reported by this graphics card")),
                Some(gpus) => {
                    for g in gpus {
                        let value = match g.used {
                            Some(used) => format!("{}, {}%", crate::perf::of(used, g.total), crate::perf::percent(used, g.total)),
                            None => format!("{} in all", crate::perf::gb(g.total)),
                        };
                        opts.push(Opt::new("Video memory", Act::Info).detail(g.name.clone()).value(value));
                    }
                }
            }
            opts.push(Opt::new("Sofa", Act::Info).value(crate::perf::mb(u.sofa)));
        }
        opts.push(Opt::new("Frame rate", Act::Info).value(self.fps_text(now)));
        opts
    }

    fn perf_sheet(&mut self, now: f64) {
        sound::play(Sound::Open);
        self.layers.push(Layer::Sheet { title: "Performance".into(), sub: "Memory and video memory in use now, and how fast Sofa draws.".into(), opts: Vec::new(), focus: 0, kind: SheetKind::Perf });
        // Read with the panel open, so video memory is read too; this fills the panel.
        self.read_perf(now, true);
    }

    fn quick_menu(&mut self, now: f64) {
        self.audio_refresh();
        self.read_perf(now, true);
        let focus = self.quick_ctls()[0];
        sound::play(Sound::Open);
        self.layers.push(Layer::Quick { focus, at: now });
    }

    fn quick_action(&mut self, a: Action, now: f64) {
        let ctls = self.quick_ctls();
        let Some(Layer::Quick { focus, .. }) = self.layers.last_mut() else { return };
        let i = ctls.iter().position(|c| c == focus).unwrap_or(0);
        match a {
            Action::Left | Action::Right => {
                let j = if a == Action::Left { i.checked_sub(1) } else { Some(i + 1).filter(|&j| j < ctls.len()) };
                match j {
                    Some(j) => {
                        *focus = ctls[j];
                        sound::play(Sound::Move);
                    }
                    None => sound::play(Sound::Edge),
                }
            }
            Action::Back => {
                self.layers.pop();
                sound::play(Sound::Back);
            }
            Action::Accept => {
                let c = ctls[i];
                *focus = c;
                sound::play(Sound::Select);
                self.quick_run(c, now);
            }
            Action::TabPrev | Action::TabNext if ctls[i] == Ctl::Music => {
                // L1 and R1 skip back and forward.
                sound::play(Sound::Move);
                self.media.send(if a == Action::TabPrev { media::Cmd::Previous } else { media::Cmd::Next });
            }
            Action::Options => self.settings(),
            _ => sound::play(Sound::Edge),
        }
    }

    /// WM_MEDIA: what is playing changed.
    pub fn media_changed(&mut self) {
        let now = self.media.snapshot();
        if now != self.playing {
            self.playing = now;
            self.dirty = true;
        }
    }

    fn quick_run(&mut self, c: Ctl, now: f64) {
        match c {
            Ctl::Music => {
                self.media.send(media::Cmd::Toggle);
                // Shown at once; the player's own answer follows (WM_MEDIA).
                if let Some(p) = &mut self.playing {
                    p.playing = !p.playing;
                }
                if self.qa {
                    self.media.set(self.playing.clone());
                }
            }
            Ctl::Output => self.output_sheet(),
            Ctl::Volume => {
                let m = !self.audio.muted;
                if self.qa || audio::set_muted(m) {
                    self.audio.muted = m;
                }
            }
            Ctl::Mic => {
                self.toggle_mic();
                self.lights_tick(now);
            }
            Ctl::Timer => self.cycle_timer(),
            Ctl::Light => self.cycle_light(),
            Ctl::Perf => self.perf_sheet(now),
            Ctl::Power => self.power_sheet(),
        }
        self.dirty = true;
    }

    fn output_sheet(&mut self) {
        self.audio_refresh();
        let current = self.audio.current.clone();
        let mut opts: Vec<Opt> = self
            .audio
            .outputs
            .iter()
            .map(|o| {
                let opt = Opt::new(o.name.clone(), Act::Output(o.id.clone())).icon(kind_icon(o.kind));
                if Some(&o.id) == current.as_ref() { opt.value("In use") } else { opt }
            })
            .collect();
        let focus = self.audio.outputs.iter().position(|o| Some(&o.id) == current.as_ref()).unwrap_or(0);
        opts.push(Opt::new(if cfg!(windows) { "Windows sound settings" } else { "Sound settings" }, Act::System("sound")).icon("settings"));
        sound::play(Sound::Open);
        self.layers.push(Layer::Sheet { title: "Sound output".into(), sub: "Games, videos and Sofa play here.".into(), opts, focus, kind: SheetKind::Sound });
    }

    fn power_sheet(&mut self) {
        let opts = vec![
            Opt::new("Sleep", Act::System("sleep")).icon("sleep"),
            Opt::new("Restart", Act::System("restart")).icon("restart"),
            Opt::new("Shut down", Act::System("shutdown")).icon("shutdown"),
            Opt::new("Go to desktop", Act::System("desktop")).icon("desktop").value(format!("{} returns", self.home_name())),
            Opt::new("Quit Sofa", Act::System("quit")).icon("quit"),
        ];
        sound::play(Sound::Open);
        self.layers.push(Layer::Sheet { title: "Power".into(), sub: "Games stay where they are while the PC sleeps.".into(), opts, focus: 0, kind: SheetKind::Power });
    }

    /// Reads the sound devices again (a QA run keeps its made-up ones).
    fn audio_refresh(&mut self) {
        if self.qa {
            return;
        }
        let (outputs, current) = audio::outputs();
        self.audio.outputs = outputs;
        self.audio.current = current;
        match audio::volume() {
            Some((v, m)) => {
                self.audio.volume = Some(v);
                self.audio.muted = m;
            }
            None => self.audio.volume = None,
        }
        self.audio.mic = audio::mic_muted();
    }

    /// L2 and R2: the volume in 4 % steps, shown in the top bar or on the quick menu.
    fn volume_step(&mut self, up: bool, now: f64) {
        if !self.qa {
            match audio::volume() {
                Some((v, m)) => {
                    self.audio.volume = Some(v);
                    self.audio.muted = m;
                }
                // No volume to read: Windows' own volume keys.
                None => return volume(up),
            }
        }
        let Some(v) = self.audio.volume else { return };
        let next = (((v * 25.0).round() + if up { 1.0 } else { -1.0 }) / 25.0).clamp(0.0, 1.0);
        if self.qa || audio::set_volume(next) {
            self.audio.volume = Some(next);
            self.audio.muted = false;
        }
        self.vol_at = now;
        sound::play(if (next - v).abs() < 0.001 { Sound::Edge } else { Sound::Move });
    }

    /// In a Discord voice channel: Sofa's mute then mutes you there, so the others see it.
    fn in_voice(&self) -> bool {
        self.voice.ready && self.voice.current.is_some()
    }

    /// Whether the microphone is muted: in Discord while in voice, else Windows' default one.
    pub(crate) fn mic_muted(&self) -> bool {
        if self.in_voice() { self.voice.mute } else { self.audio.mic.unwrap_or(false) }
    }

    /// Mutes or unmutes the microphone; the new state, or None without one.
    fn toggle_mic(&mut self) -> Option<bool> {
        if self.in_voice() {
            self.voice.mute = !self.voice.mute;
            self.social.send(Req::SetVoice { mute: Some(self.voice.mute), deaf: None });
            if (self.tab == "home" || self.tab == "social") && !self.layers_hold_items() {
                self.rebuild(true);
            }
            return Some(self.voice.mute);
        }
        if !self.qa {
            self.audio.mic = audio::mic_muted();
        }
        let m = !self.audio.mic?;
        if self.qa || audio::set_mic_muted(m) {
            self.audio.mic = Some(m);
        }
        self.audio.mic
    }

    /// The DualSense's mute button, from any app, like on a PS5. Its LED shows the result.
    pub fn mute_button(&mut self, now: f64) {
        let discord = self.in_voice();
        let Some(m) = self.toggle_mic() else { return };
        if self.focused {
            let title = match (discord, m) {
                (true, true) => "Muted in Discord",
                (true, false) => "Unmuted in Discord",
                (false, true) => "Microphone muted",
                (false, false) => "Microphone on",
            };
            self.toast(title, "");
        }
        self.lights_tick(now);
        self.dirty = true;
    }

    fn cycle_light(&mut self) {
        let next = match self.store.text("controllerLight") {
            "amber" => "off",
            "off" => "follow",
            _ => "amber",
        };
        self.store.set("controllerLight", Value::from(next));
        self.store.flush();
        self.refresh_settings();
        self.light_rgb = self.light_color();
    }

    /// The lightbar's colour: the focused game's (dimmed, so the light stays calm), amber or off.
    fn light_color(&self) -> (u8, u8, u8) {
        let dim = |c: Color| {
            let (r, g, b) = c.rgb8();
            let d = |v: u8| (f32::from(v) * 0.7) as u8;
            (d(r), d(g), d(b))
        };
        match self.store.text("controllerLight") {
            "off" => (0, 0, 0),
            "amber" => dim(ACCENT),
            _ => dim(self.focused_tone().unwrap_or(ACCENT)),
        }
    }

    /// The focused tile's colour: its service's or Discord's, else its art's most vivid one.
    pub(crate) fn focused_tone(&self) -> Option<Color> {
        let (id, _) = self.hero.as_ref()?;
        if id.starts_with("dc:") {
            return Some(DISCORD);
        }
        if let Some(key) = id.strip_prefix("mu:") {
            return Some(self.pick(key).and_then(|p| p.cover.as_ref()).and_then(|c| self.gfx.tone(c)).unwrap_or(SPOTIFY));
        }
        let i = match id.strip_prefix("run:").and_then(|h| h.parse::<isize>().ok()) {
            Some(h) => self.run_of(h)?.item?,
            None => self.items.iter().position(|it| &it.id == id)?,
        };
        let it = self.items.get(i)?;
        it.color.as_deref().and_then(Color::parse).or_else(|| [&it.art.capsule, &it.art.hero, &it.art.icon].into_iter().flatten().find_map(|p| self.gfx.tone(p)))
    }

    /// Tells the DualSense thread what to show. The lightbar takes the focused tile's colour
    /// once focus has rested 0.7 s, so moving along a row sends nothing.
    fn lights_tick(&mut self, now: f64) {
        if self.qa {
            return;
        }
        let front = self.focused;
        if front && self.hero_pending.is_none() && self.hero.as_ref().is_none_or(|(_, t)| now - t >= 700.0) {
            self.light_rgb = self.light_color();
        }
        self.input.set_lights(Lights { armed: true, front, rgb: self.light_rgb, mute: self.mic_muted() });
    }

    /// Time left on the sleep timer (ms).
    pub(crate) fn sleep_left(&self) -> Option<f64> {
        self.sleep_at.map(|at| (at - epoch_ms()).max(0.0))
    }

    /// The quick menu's Sleep timer: 30 min, 1 h, 1 h 30 min, 2 h, then off.
    fn cycle_timer(&mut self) {
        let left = self.sleep_left().map(|ms| (ms / 60_000.0).ceil() as u32);
        let next = match left {
            None => Some(TIMER_STEPS[0]),
            Some(m) => TIMER_STEPS.iter().copied().find(|&s| s > m),
        };
        self.sleep_at = next.map(|m| epoch_ms() + f64::from(m) * 60_000.0);
        self.sleep_warned = false;
        self.sleep_shown = -1;
    }

    /// The sleep timer: a minute before the end Sofa comes forward with a countdown; at the
    /// end the PC sleeps (never in a QA run).
    pub(crate) fn sleep_tick(&mut self, now: f64) {
        let Some(at) = self.sleep_at else { return };
        let left = at - epoch_ms();
        if left <= 0.0 {
            self.sleep_at = None;
            self.sleep_warned = false;
            self.sleep_from = None;
            self.layers.retain(|l| !is_sleep_warning(l));
            self.dirty = true;
            // Long past the end, the PC was already asleep or off: the timer's job is done.
            if left > -120_000.0 {
                crate::log("sleep timer ended");
                if !self.qa {
                    platform::sleep();
                }
            }
            return;
        }
        if left <= 60_000.0 && !self.sleep_warned {
            self.sleep_warned = true;
            self.sleep_warning(now);
        }
        let secs = (left / 1000.0).ceil() as i64;
        let shown = if self.sleep_warned { secs } else { (secs + 59) / 60 };
        if shown != self.sleep_shown {
            self.sleep_shown = shown;
            if self.sleep_warned {
                let title = if secs == 1 { "Sleeping in 1 second".to_owned() } else { format!("Sleeping in {secs} seconds") };
                for l in &mut self.layers {
                    if is_sleep_warning(l)
                        && let Layer::Dialog { title: t, .. } = l
                    {
                        *t = title.clone();
                    }
                }
            }
            self.dirty = true;
        }
    }

    fn sleep_warning(&mut self, now: f64) {
        self.sleep_from = None;
        if !self.qa && !self.focused {
            // Over a game or a film: Sofa comes forward, and Stay awake goes back to it.
            let came = self.shell.home_pressed(now);
            self.sleep_from = came.as_ref().map(|w| w.hwnd);
            platform::focus_id(self.shell.sofa);
            self.ps_home(came, now);
        } else {
            sound::play(Sound::Notice);
        }
        self.layers.push(Layer::Dialog {
            title: "Sleeping in 60 seconds".into(),
            body: "Your sleep timer is ending. Games and videos stay where they are while the PC sleeps.".into(),
            opts: vec![Opt::new("Stay awake", Act::StayAwake).icon("play"), Opt::new("Sleep now", Act::SleepNow).icon("sleep")],
            focus: 0,
        });
        self.dirty = true;
    }

    /// QA only: the sleep timer ends in under a minute, so its warning shows at once.
    pub fn qa_sleep_soon(&mut self) {
        self.sleep_at = Some(epoch_ms() + 59_000.0);
        self.sleep_warned = false;
        self.dirty = true;
    }
}

/// "Playlist · 42 songs · by Greg", "Album · Daft Punk · 13 songs", "Your Liked Songs · 512 songs".
pub(crate) fn pick_meta(p: &Pick, user: Option<&str>) -> String {
    let songs = p.songs.map(spotify::songs_text);
    let mut parts: Vec<String> = Vec::new();
    match p.kind {
        spotify::Kind::Liked => {
            parts.push("Your Liked Songs".into());
            parts.extend(songs);
        }
        spotify::Kind::Playlist => {
            parts.push("Playlist".into());
            parts.extend(songs);
            if !p.by.is_empty() && Some(p.by.as_str()) != user {
                parts.push(format!("by {}", p.by));
            }
        }
        spotify::Kind::Album => {
            parts.push("Album".into());
            if !p.by.is_empty() {
                parts.push(p.by.clone());
            }
            parts.extend(songs);
        }
    }
    parts.join("  ·  ")
}

/// Spotify: the Music tab, Home's row, its setup and account sheets, and playing.
impl App {
    pub(crate) fn pick(&self, uri: &str) -> Option<&Pick> {
        self.music.library.iter().find(|p| p.uri == uri)
    }

    /// The Music tab: Liked Songs and the playlists, then the albums; or the way to connect.
    fn music_rows(&self) -> Vec<Row> {
        let m = &self.music;
        if !m.ready {
            // Signed in and reading: the account tile says so; otherwise, Connect.
            let tile = if m.configured && m.error.is_none() { Mu::Account } else { Mu::Setup };
            return vec![Row { key: "spotify".into(), title: "Spotify".into(), count: 0, tiles: vec![Tile::Music(tile)] }];
        }
        let tiles_of = |albums: bool| -> Vec<Tile> { m.library.iter().filter(|p| (p.kind == spotify::Kind::Album) == albums).map(|p| Tile::Music(Mu::Play(p.uri.clone()))).collect() };
        let mut lists = tiles_of(false);
        let n = lists.len();
        lists.push(Tile::Music(Mu::Account));
        let mut rows = vec![Row { key: "playlists".into(), title: "Playlists".into(), count: n, tiles: lists }];
        let albums = tiles_of(true);
        if !albums.is_empty() {
            rows.push(Row { key: "albums".into(), title: "Albums".into(), count: albums.len(), tiles: albums });
        }
        rows
    }

    fn music_action(&mut self, m: Mu) {
        sound::play(Sound::Select);
        match m {
            Mu::Play(uri) => self.play_music(&uri, false),
            Mu::Setup => self.spotify_setup(),
            Mu::Account => self.spotify_account(),
        }
        self.dirty = true;
    }

    /// Plays something on this PC's Spotify app (a QA run only says it would).
    fn play_music(&mut self, uri: &str, shuffle: bool) {
        let Some(p) = self.pick(uri).cloned() else { return };
        if self.qa {
            self.toast(&format!("Playing {}", p.name), "Spotify, on this PC");
            return;
        }
        self.music.starting = Some(uri.to_owned());
        self.spotify.send(spotify::Req::Play { uri: uri.to_owned(), name: p.name, shuffle });
    }

    /// WM_MUSIC: Spotify's state changed (or it asks Sofa to open the sign-in, show a notice,
    /// or come back to the front).
    pub fn music_changed(&mut self, now: f64) {
        let spotify::Taken { open, notice, come_back, save } = self.spotify.take();
        if let Some(cfg) = save {
            self.store.map_mut("music").insert("spotify".into(), cfg);
            self.store.flush();
        }
        if let Some(url) = open
            && !self.qa
        {
            // The sign-in is a web page: Sofa steps aside for the browser.
            self.shell.step_down(now);
            platform::open(&url);
        }
        if let Some((title, body)) = notice {
            self.toast(&title, &body);
        }
        if come_back && !self.qa && !self.focused {
            platform::focus_id(self.shell.sofa);
        }
        let m = self.spotify.snapshot();
        if m == self.music {
            return;
        }
        let rows = m.library != self.music.library || m.ready != self.music.ready || m.configured != self.music.configured || m.error != self.music.error;
        self.music = m;
        if self.music.ready {
            self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::SpSetup, .. }));
        }
        // Disconnected without Spotify installed: the Music tab goes.
        if self.tab == "music" && !self.tabs().iter().any(|(k, _)| *k == "music") {
            self.set_tab("home", now);
        }
        self.refresh_sp_setup(usize::MAX);
        if rows && (self.tab == "home" || self.tab == "music") {
            if self.layers_hold_items() {
                self.music_stale = true;
            } else {
                self.rebuild(true);
                if let Some(id) = self.focused_tile_id() {
                    self.show_hero(id, now);
                }
            }
        }
        self.dirty = true;
    }

    fn sp_setup_opts(&self) -> Vec<Opt> {
        let id = &self.sp_id;
        let ok = spotify::client_id_ok(id);
        let step = |icon: &'static str| if ok { "check" } else { icon };
        let short = if id.len() > 6 { format!("{}…", &id[..6]) } else { id.clone() };
        let connect = if self.music.signing_in {
            "Waiting for your browser. Approve Sofa there.".to_owned()
        } else if let Some(e) = &self.music.error {
            e.clone()
        } else {
            "Your browser opens once. Approve Sofa there, then come back.".to_owned()
        };
        vec![
            Opt::new("Make a Spotify app", Act::SpPage).icon(step("globe")).detail("Opens developer.spotify.com. Create an app named Sofa and tick Web API. Spotify asks for Premium on this account."),
            Opt::new("Add the redirect", Act::SpPage).icon(step("link")).detail(format!("In the app's settings, add the redirect URI {} and save.", spotify::REDIRECT)),
            Opt::new("Paste the Client ID", Act::SpId).icon(step("rename")).detail("Copy it from the app's settings, then select this.").value(short),
            Opt::new("Sign in", Act::SpConnect).icon("play").detail(connect),
        ]
    }

    /// Connect Spotify: one-time setup, step by step (like Connect Discord).
    fn spotify_setup(&mut self) {
        sound::play(Sound::Open);
        if self.sp_id.is_empty() {
            self.sp_id = self.store.map("music").get("spotify").and_then(|s| s.get("clientId")).and_then(Value::as_str).unwrap_or("").to_owned();
        }
        let focus = if spotify::client_id_ok(&self.sp_id) { 3 } else if self.sp_id.is_empty() { 0 } else { 2 };
        self.layers.push(Layer::Sheet {
            title: "Connect Spotify".into(),
            sub: "One-time setup, about two minutes. Spotify only lets apps you made control playback.".into(),
            opts: self.sp_setup_opts(),
            focus,
            kind: SheetKind::SpSetup,
        });
    }

    /// The setup sheet follows what happened; `step` moves its focus (usize::MAX keeps it).
    fn refresh_sp_setup(&mut self, step: usize) {
        let opts = self.sp_setup_opts();
        for l in &mut self.layers {
            if let Layer::Sheet { opts: o, focus, kind: SheetKind::SpSetup, .. } = l {
                *o = opts.clone();
                if step != usize::MAX {
                    *focus = step.min(o.len() - 1);
                }
            }
        }
    }

    fn spotify_account(&mut self) {
        sound::play(Sound::Open);
        let who = self.music.user.clone().unwrap_or_else(|| "your account".into());
        let sub = match &self.music.error {
            Some(e) => e.clone(),
            None => format!("Signed in as {who}. Sofa plays through the Spotify app on this PC."),
        };
        self.layers.push(Layer::Sheet {
            title: "Spotify".into(),
            sub,
            opts: vec![Opt::new("Refresh library", Act::SpRefresh).icon("rescan"), Opt::new("Sign in again", Act::SpRedo).icon("restart"), Opt::new("Disconnect", Act::SpForget).icon("close")],
            focus: 0,
            kind: SheetKind::SpAccount,
        });
    }

    fn sp_run(&mut self, act: Act, now: f64) {
        match act {
            Act::SpPage => {
                if !self.qa {
                    self.shell.step_down(now);
                    platform::open(spotify::DASHBOARD);
                }
            }
            Act::SpId => {
                // Copied on the PC: selecting the step pastes it.
                let clip = platform::clipboard_text().trim().to_owned();
                if spotify::client_id_ok(&clip) {
                    self.sp_id = clip;
                    self.music.error = None;
                    self.refresh_sp_setup(3);
                } else {
                    let id = self.sp_id.clone();
                    self.keyboard(KbPurpose::SpId, id);
                }
            }
            Act::SpConnect => {
                if !spotify::client_id_ok(&self.sp_id) {
                    self.music.error = Some("The Client ID is 32 letters and digits, from your Spotify app's settings.".into());
                    return self.refresh_sp_setup(2);
                }
                self.music.error = None;
                self.music.signing_in = true;
                self.spotify.send(spotify::Req::Connect(self.sp_id.clone()));
                self.refresh_sp_setup(3);
            }
            Act::SpRedo => {
                self.layers.pop();
                self.spotify_setup();
            }
            Act::SpRefresh => {
                self.layers.pop();
                self.spotify.send(spotify::Req::Refresh);
            }
            Act::SpForget => self.layers.push(Layer::Dialog {
                title: "Disconnect Spotify?".into(),
                body: "Sofa forgets the app and its sign-in. Your Spotify account and its music stay as they are.".into(),
                opts: vec![Opt::new("Disconnect", Act::SpForgetNow).icon("close"), Opt::new("Cancel", Act::Close).icon("restart")],
                focus: 1,
            }),
            Act::SpForgetNow => {
                self.layers.retain(|l| !matches!(l, Layer::Dialog { .. } | Layer::Sheet { kind: SheetKind::SpAccount, .. }));
                self.sp_id.clear();
                self.spotify.send(spotify::Req::Forget);
                if self.qa {
                    self.spotify.set(Music::default());
                    self.music_changed(now);
                }
            }
            Act::SpPlay(uri, shuffle) => {
                self.layers.retain(|l| !matches!(l, Layer::Sheet { kind: SheetKind::Music, .. }));
                self.play_music(&uri, shuffle);
            }
            Act::SpOpen(uri) => {
                self.layers.pop();
                if !self.qa {
                    self.shell.step_down(now);
                    platform::open(&uri);
                }
            }
            _ => {}
        }
        self.dirty = true;
    }
}

/// The sleep timer's one-minute warning.
fn is_sleep_warning(l: &Layer) -> bool {
    matches!(l, Layer::Dialog { opts, .. } if opts.iter().any(|o| matches!(o.act, Act::StayAwake)))
}

/// A key press and release, as a remote (or keyboard) sends it, to the window in front.
#[cfg(windows)]
fn send_key(vk: u16) {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    let extended = matches!(VIRTUAL_KEY(vk), VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_MEDIA_PLAY_PAUSE);
    let flags = if extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) };
    let key = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { flags | KEYEVENTF_KEYUP } else { flags }, time: 0, dwExtraInfo: platform::SOFA_MARK } },
    };
    // SAFETY: the inputs live on the stack for the call.
    unsafe {
        SendInput(&[key(false), key(true)], std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn volume(up: bool) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_VOLUME_DOWN, VK_VOLUME_UP, keybd_event};
    let k = if up { VK_VOLUME_UP } else { VK_VOLUME_DOWN };
    // SAFETY: synthesizing the media volume key.
    unsafe {
        keybd_event(k.0 as u8, 0, Default::default(), platform::SOFA_MARK);
        keybd_event(k.0 as u8, 0, KEYEVENTF_KEYUP, platform::SOFA_MARK);
    }
}

/// The media volume keys' step, on the default output.
#[cfg(target_os = "linux")]
fn volume(up: bool) {
    if let Some((v, _)) = audio::volume() {
        audio::set_volume(v + if up { 0.05 } else { -0.05 });
    }
}

fn quit() {
    crate::quit();
}

fn post(hwnd: isize, msg: u32, w: usize) {
    crate::post(hwnd, msg, w);
}
