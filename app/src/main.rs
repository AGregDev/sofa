//! Sofa: one Windows executable, no web engine (docs/port/DECISIONS.md, ADR-001 and ADR-008).
//! The window and its message loop. Sofa draws only while something moves; otherwise it
//! sleeps until a message arrives (the DualSense thread posts one when a button changes;
//! XInput pads are polled at 60 Hz while Sofa is in front).
//!
//! The same file is the installer: named `Sofa-Setup-*.exe` it installs itself, and
//! `Sofa.exe --uninstall` removes it (see `install.rs`).
#![windows_subsystem = "windows"]

mod audio;
mod gfx;
mod icons;
mod input;
mod install;
mod library;
mod media;
mod platform;
mod shell;
mod social;
mod spotify;
mod sound;
mod startmenu;
mod store;
mod ui;
mod view;

use input::Action;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const FONT: &[u8] = include_bytes!("../assets/fonts/figtree.woff2");
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");
const WELCOME_JPG: &[u8] = include_bytes!("../../assets/welcome.jpg");

/// Another start of Sofa asks the running one to come forward.
const WM_SHOW: u32 = 0x8000 + 7;
/// QA only: give memory back now, as after 15 s behind a game.
const WM_QA_TRIM: u32 = 0x8000 + 9;
/// QA only: the PS button was pressed in the window `wparam` (a stand-in for a game).
const WM_QA_RUN: u32 = 0x8000 + 10;
/// QA only: the sleep timer ends in a minute (its warning shows at once).
const WM_QA_SLEEP: u32 = 0x8000 + 13;

thread_local! {
    static APP: RefCell<Option<ui::App>> = const { RefCell::new(None) };
    static START: Instant = Instant::now();
    /// Window messages that arrived while the app was busy (a Win32 call inside an action
    /// sends messages back to this window); handled right after.
    static DEFERRED: RefCell<Vec<(u32, usize, isize)>> = const { RefCell::new(Vec::new()) };
}

static LOG: OnceLock<PathBuf> = OnceLock::new();
static QA: OnceLock<bool> = OnceLock::new();
static FULLSCREEN: OnceLock<bool> = OnceLock::new();

/// A diagnostic log, only with `--log` (`%TEMP%\sofa-native.log`) or in a QA run.
pub fn log(line: &str) {
    use std::io::Write;
    let Some(path) = LOG.get() else { return };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 500_000) {
        let _ = std::fs::remove_file(path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{:>9.0}  {line}", now());
    }
}

fn qa() -> bool {
    QA.get().copied().unwrap_or(false)
}

fn now() -> f64 {
    START.with(|s| s.elapsed().as_secs_f64() * 1000.0)
}

fn with_app<R>(f: impl FnOnce(&mut ui::App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn log_state(app: &ui::App) {
    if LOG.get().is_some() {
        log(&app.state_line());
    }
}

/// The keys Sofa reads as actions (also the keys held back from other programs while Sofa is
/// in front, `platform::Hold::SofaKeys`).
pub(crate) fn key_action(vk: VIRTUAL_KEY) -> Option<Action> {
    use input::Action::*;
    Some(match vk {
        VK_UP => Up,
        VK_DOWN => Down,
        VK_LEFT => Left,
        VK_RIGHT => Right,
        VK_RETURN | VK_SPACE => Accept,
        VK_ESCAPE | VK_BACK => Back,
        VK_HOME => Home,
        VIRTUAL_KEY(0x4D) => Menu,    // M
        VIRTUAL_KEY(0x46) => Search,  // F
        VIRTUAL_KEY(0x4F) => Options, // O
        VIRTUAL_KEY(0x51) => TabPrev, // Q
        VIRTUAL_KEY(0x45) => TabNext, // E
        VK_OEM_4 => TabPrev,          // [
        VK_OEM_6 => TabNext,          // ]
        VK_OEM_MINUS => VolDown,      // -
        VK_OEM_PLUS => VolUp,         // =
        _ => return None,
    })
}

/// The Home button, from a controller (`ps`: the PS button itself, which can be turned off).
fn home_button(hwnd: HWND, ps: bool, t: f64) {
    with_app(|app| {
        if ps && !app.input.ps_home {
            return;
        }
        // One press can arrive twice (the HID thread and XInput through Steam).
        if t - app.last_home < 350.0 {
            return;
        }
        app.last_home = t;
        // SAFETY: plain queries.
        let front = unsafe { GetForegroundWindow() == hwnd && !IsIconic(hwnd).as_bool() };
        if front || app.qa {
            app.action(Action::Home, t);
        } else {
            let came = app.shell.home_pressed(t);
            platform::focus(hwnd);
            app.ps_home(came, t);
        }
        log_state(app);
    });
}

/// The messages `handle` acts on. Only these wait in DEFERRED while the app is busy; every
/// other message goes to DefWindowProc as usual. (Answering 0 to all of them broke restoring:
/// WM_QUERYOPEN answered 0 keeps a minimized window minimized, and WM_WINDOWPOSCHANGED never
/// reaching DefWindowProc loses WM_SIZE.)
fn ours(msg: u32) -> bool {
    matches!(
        msg,
        WM_KEYDOWN
            | WM_SYSKEYDOWN
            | WM_CHAR
            | WM_SIZE
            | WM_ACTIVATEAPP
            | WM_DISPLAYCHANGE
            | WM_HOTKEY
            | WM_SHOW
            | WM_QA_TRIM
            | WM_QA_RUN
            | WM_QA_SLEEP
            | gfx::WM_IMAGE
            | input::WM_PAD_HOME
            | input::WM_PAD_INPUT
            | input::WM_PAD_MUTE
            | ui::WM_LIBRARY
            | ui::WM_ICON
            | ui::WM_STEAM
            | social::WM_SOCIAL
            | media::WM_MEDIA
            | spotify::WM_MUSIC
    )
}

/// Handles the messages that need the app. When it is busy, the message waits in DEFERRED.
fn handle(hwnd: HWND, msg: u32, w: usize, l: isize) -> Option<LRESULT> {
    if !ours(msg) {
        return None;
    }
    if APP.with(|a| a.try_borrow_mut().is_err()) {
        DEFERRED.with(|d| d.borrow_mut().push((msg, w, l)));
        return Some(LRESULT(0));
    }
    let t = now();
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = VIRTUAL_KEY(w as u16);
            let repeat = l & (1 << 30) != 0;
            // SAFETY: plain query.
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
            if with_app(|app| app.text_mode()).unwrap_or(false) {
                // Typing: letters arrive as WM_CHAR; these keys edit or move.
                let done = with_app(|app| {
                    let handled = match vk {
                        VK_BACK => {
                            app.erase();
                            app.kb_mark_typed();
                            true
                        }
                        VIRTUAL_KEY(0x56) if ctrl => {
                            app.paste();
                            app.kb_mark_typed();
                            true
                        }
                        VK_RETURN => {
                            app.kb_enter(t);
                            true
                        }
                        VK_ESCAPE => {
                            app.action(Action::Back, t);
                            true
                        }
                        VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT => {
                            if let Some(a) = key_action(vk) {
                                app.action(a, t);
                            }
                            true
                        }
                        _ => false,
                    };
                    log_state(app);
                    handled
                });
                return done.unwrap_or(false).then_some(LRESULT(0));
            }
            let action = key_action(vk)?;
            let dir = matches!(action, Action::Up | Action::Down | Action::Left | Action::Right);
            if !repeat || dir {
                with_app(|app| {
                    if let Some(a) = app.input.key(action, t) {
                        app.action(a, t);
                    }
                    log_state(app);
                });
            }
            Some(LRESULT(0))
        }
        WM_CHAR => {
            let c = char::from_u32(w as u32)?;
            with_app(|app| {
                if app.text_mode() && !c.is_control() {
                    app.type_char(c);
                    log_state(app);
                }
            });
            Some(LRESULT(0))
        }
        WM_SIZE => {
            if w as u32 != SIZE_MINIMIZED {
                with_app(|app| {
                    let _ = app.gfx.resize();
                    app.dirty = true;
                });
            }
            Some(LRESULT(0))
        }
        WM_ACTIVATEAPP => {
            if !qa() {
                with_app(|app| app.on_focus(w != 0, t));
            }
            Some(LRESULT(0))
        }
        WM_DISPLAYCHANGE => {
            if FULLSCREEN.get().copied().unwrap_or(false) {
                // SAFETY: resizing our own window to the new screen size.
                unsafe {
                    let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
                    let _ = SetWindowPos(hwnd, None, 0, 0, sw, sh, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            Some(LRESULT(0))
        }
        gfx::WM_IMAGE => {
            with_app(|app| {
                if app.gfx.take_decoded() {
                    app.dirty = true;
                }
            });
            Some(LRESULT(0))
        }
        // A QA run takes input only from its own posted keys: someone using the controller
        // during a run must not change its results.
        input::WM_PAD_HOME => {
            if !qa() {
                home_button(hwnd, w == 0, t);
            }
            Some(LRESULT(0))
        }
        // The DualSense's buttons changed: the loop wakes up and polls.
        input::WM_PAD_INPUT => Some(LRESULT(0)),
        input::WM_PAD_MUTE => {
            if !qa() {
                with_app(|app| {
                    app.mute_button(t);
                    log_state(app);
                });
            }
            Some(LRESULT(0))
        }
        ui::WM_LIBRARY => {
            with_app(|app| {
                app.library_changed();
                log(&format!("library: {} items", app.state_line()));
            });
            Some(LRESULT(0))
        }
        ui::WM_ICON => {
            with_app(|app| {
                app.gfx.retry_failed();
                app.dirty = true;
            });
            Some(LRESULT(0))
        }
        ui::WM_STEAM => {
            with_app(|app| app.steam_result(w));
            Some(LRESULT(0))
        }
        social::WM_SOCIAL => {
            with_app(|app| {
                app.social_changed(t);
                log_state(app);
            });
            Some(LRESULT(0))
        }
        media::WM_MEDIA => {
            with_app(|app| app.media_changed());
            Some(LRESULT(0))
        }
        spotify::WM_MUSIC => {
            with_app(|app| {
                app.music_changed(t);
                log_state(app);
            });
            Some(LRESULT(0))
        }
        WM_SHOW => {
            with_app(|app| {
                if !app.qa {
                    platform::focus(hwnd);
                }
                app.go_home(t);
            });
            Some(LRESULT(0))
        }
        WM_QA_TRIM if qa() => {
            with_app(|app| app.trim());
            Some(LRESULT(0))
        }
        WM_QA_SLEEP if qa() => {
            with_app(|app| {
                app.qa_sleep_soon();
                app.animating(t);
                log_state(app);
            });
            Some(LRESULT(0))
        }
        WM_QA_RUN if qa() => {
            with_app(|app| {
                let win = shell::window(w as isize);
                if l == 1 {
                    app.qa_running(win, t);
                } else {
                    app.ps_home(Some(win), t);
                }
                log_state(app);
            });
            Some(LRESULT(0))
        }
        // Ctrl+Alt+H, from anywhere: the keyboard's Home button.
        WM_HOTKEY => {
            home_button(hwnd, false, t);
            Some(LRESULT(0))
        }
        _ => None,
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            // SAFETY: validates the window's dirty region.
            unsafe {
                BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
            }
            with_app(|app| app.dirty = true);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
            // SAFETY: hides the pointer over Sofa; it is a controller UI.
            unsafe { SetCursor(None) };
            return LRESULT(1);
        }
        // Alt alone would open the (absent) window menu and swallow the next key.
        WM_SYSCOMMAND if (wparam.0 & 0xfff0) as u32 == SC_KEYMENU => return LRESULT(0),
        WM_QUERYENDSESSION => return LRESULT(1),
        WM_ENDSESSION => {
            with_app(|app| app.save());
            return LRESULT(0);
        }
        WM_DESTROY => {
            // SAFETY: ends the message loop.
            unsafe { PostQuitMessage(0) };
            return LRESULT(0);
        }
        _ => {
            if let Some(r) = handle(hwnd, msg, wparam.0, lparam.0) {
                return r;
            }
        }
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn message_box(title: &str, text: &str, question: bool) -> bool {
    let flags = if question { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONERROR };
    // SAFETY: a modal message box with owned strings.
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), flags | MB_SETFOREGROUND) == IDYES }
}

/// `--size 1600x900`: a borderless window of that size (screenshots in the QA run).
fn size_arg(args: &[String]) -> Option<(i32, i32)> {
    let i = args.iter().position(|a| a == "--size")?;
    let (w, h) = args.get(i + 1)?.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let qa_run = has("--qa");
    let _ = QA.set(qa_run);

    // `--scan <cache> [--jobs <file>] [--no-startmenu]`: the background child that lists the
    // Start menu and extracts program icons, then exits (see `ui::spawn_scan`).
    if let Some(i) = args.iter().position(|a| a == "--scan") {
        let Some(cache) = args.get(i + 1).map(PathBuf::from) else { return Ok(()) };
        // SAFETY: COM for this process's only thread.
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        if !has("--no-startmenu") {
            startmenu::scan(&cache);
        }
        if let Some(file) = args.iter().position(|a| a == "--jobs").and_then(|j| args.get(j + 1)).map(PathBuf::from) {
            let jobs: Vec<(String, String)> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            for (exe, png) in jobs {
                let png = std::path::Path::new(&png);
                if !startmenu::save_exe_icon(&exe, png) && !exe.to_lowercase().ends_with(".exe") {
                    startmenu::save_icon(&exe, png);
                }
            }
            let _ = std::fs::remove_file(&file);
        }
        return Ok(());
    }

    if has("--uninstall") {
        // `--quiet` skips the question (scripts, and the QA install check).
        if has("--quiet") || message_box("Uninstall Sofa", "Remove Sofa from this PC? Your settings stay, so installing it again picks up where you left off.", true) {
            install::uninstall();
        }
        return Ok(());
    }
    if install::is_setup() && !qa_run {
        let autostart = store::Store::open(&store::data_dir()).flag("launchAtLogin");
        match install::install(env!("CARGO_PKG_VERSION"), autostart) {
            Ok(exe) => {
                let _ = std::process::Command::new(&exe).spawn();
            }
            Err(e) => {
                message_box("Sofa", &format!("Sofa couldn't be installed. {e}"), false);
            }
        }
        return Ok(());
    }

    let data = if qa_run { std::env::temp_dir().join("sofa-qa") } else { store::data_dir() };
    if qa_run {
        let _ = std::fs::create_dir_all(&data);
        let path = data.join("qa.log");
        let _ = std::fs::remove_file(&path);
        let _ = LOG.set(path);
    } else if has("--log") {
        let _ = LOG.set(std::env::temp_dir().join("sofa-native.log"));
    }
    let size = size_arg(&args);
    let windowed = has("--windowed");
    let _ = FULLSCREEN.set(size.is_none() && !windowed);
    let class = if qa_run { w!("SofaNativeQA") } else { w!("SofaNative") };
    let mut relaunch = false;

    // SAFETY: Win32 setup calls with valid arguments, in order, on the UI thread.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        // One Sofa at a time: a second start brings the first one forward.
        let mutex = CreateMutexW(None, true, if qa_run { w!("Local\\SofaNativeQA") } else { w!("Local\\SofaNative") })?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if has("--relaunch") {
                // Restarting after a graphics failure: wait for the old process to end.
                let _ = WaitForSingleObject(mutex, 10_000);
            } else {
                if let Ok(other) = FindWindowW(class, None) {
                    let mut pid = 0u32;
                    GetWindowThreadProcessId(other, Some(&mut pid));
                    let _ = AllowSetForegroundWindow(pid);
                    let _ = PostMessageW(Some(other), WM_SHOW, WPARAM(0), LPARAM(0));
                }
                return Ok(());
            }
        }
        let instance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1 as _)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let (style, x, y, w, h) = match (size, windowed) {
            (Some((w, h)), _) => (WS_POPUP, 40, 40, w, h),
            (None, true) => (WS_OVERLAPPEDWINDOW, 80, 60, 1600, 900),
            (None, false) => (WS_POPUP, 0, 0, sw, sh),
        };
        let hwnd = CreateWindowExW(WS_EX_APPWINDOW, class, w!("Sofa"), style, x, y, w, h, None, None, Some(instance.into()), None)?;

        let cache = data.join("cache");
        let _ = std::fs::create_dir_all(&cache);
        let asset = |name: &str, bytes: &[u8]| {
            let p = cache.join(name);
            if std::fs::metadata(&p).map_or(true, |m| m.len() != bytes.len() as u64) {
                let _ = std::fs::write(&p, bytes);
            }
            p
        };
        let icon_png = asset("sofa-icon.png", ICON_PNG);
        let welcome_jpg = asset("sofa-welcome.jpg", WELCOME_JPG);
        let started = now();
        let g = gfx::Gfx::new(hwnd, FONT)?;
        let inp = input::Input::new(hwnd, START.with(|s| *s));
        let sh = shell::Shell::new(hwnd.0 as isize, false);
        let mut app = ui::App::new(g, inp, sh, data, icon_png, welcome_jpg, qa_run, now());
        if !qa_run {
            shell::watch_foreground();
            let _ = RegisterHotKey(Some(hwnd), 1, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 0x48);
        }
        app.startup(now());
        app.boot_at = now();
        app.draw(now());
        log(&format!("ready in {:.0} ms ({} items)", now() - started, app.state_line()));
        APP.with(|a| *a.borrow_mut() = Some(app));
        if qa_run {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }

        let mut msg = MSG::default();
        'run: loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'run;
                }
                // Keys Sofa maps to actions make no characters (F opens search: no "f" in it).
                let mapped = msg.message == WM_KEYDOWN && key_action(VIRTUAL_KEY(msg.wParam.0 as u16)).is_some() && !with_app(|a| a.text_mode()).unwrap_or(false);
                if !mapped {
                    let _ = TranslateMessage(&msg);
                }
                DispatchMessageW(&msg);
            }
            for (m, w, l) in DEFERRED.with(|d| std::mem::take(&mut *d.borrow_mut())) {
                handle(hwnd, m, w, l);
            }
            let t = now();
            let front = GetForegroundWindow() == hwnd && !IsIconic(hwnd).as_bool();
            let visible = !IsIconic(hwnd).as_bool();
            let step = with_app(|app| {
                // Focus as Windows sees it (WM_ACTIVATEAPP can arrive while the app is busy).
                if !app.qa {
                    app.on_focus(front, t);
                }
                let mut events = Vec::new();
                if !app.qa {
                    for fg in shell::take_foreground_events() {
                        events.extend(app.shell.on_foreground(fg, t));
                    }
                    events.extend(app.shell.tick(t, app.focused));
                }
                for e in events {
                    log(&format!("shell {e:?}"));
                    match e {
                        shell::Event::BringBack => {
                            platform::focus(hwnd);
                            app.go_home(t);
                        }
                        shell::Event::SteamGrab => app.steam_grab(),
                        shell::Event::Ended(h) => app.running_ended(h),
                        shell::Event::Ready(h) => app.media_ready(h),
                    }
                }
                let mut home = false;
                let focused = app.focused;
                // The streaming window Sofa opened is in front: the controller is its remote.
                let remote = app.remote_active();
                // Polled in a QA run too (it keeps track of connected pads), but a QA run takes
                // input only from its own posted keys.
                let pads = app.input.poll(t, focused || remote);
                let qa_run = app.qa;
                for a in pads.into_iter().filter(|_| !qa_run) {
                    if a == Action::Home {
                        home = true;
                    } else if focused {
                        if !app.qa {
                            platform::user_active();
                        }
                        app.action(a, t);
                        log_state(app);
                    } else if remote {
                        app.remote_action(a);
                    }
                }
                // Sofa turns the controller into input itself: while the streaming window it
                // opened is in front, other programs' keys and clicks are held back; while Sofa
                // is in front with a pad it reads (a DualSense or an XInput pad), only their keys
                // that Sofa reads as actions are (Steam's desktop layout sent Enter for ○).
                let reads_pad = app.input.info().connected || app.input.xinput_active();
                let hold = if remote {
                    platform::Hold::All
                } else if !qa_run && focused && reads_pad {
                    platform::Hold::SofaKeys
                } else {
                    platform::Hold::Off
                };
                platform::hold_synthetic_input(hold);
                app.trim_if_idle(t);
                let anim = app.animating(t);
                // While Sofa's launch screen covers a starting browser, it keeps animating.
                let live = focused || app.qa || app.shell.covering();
                let draw = visible && if live { anim } else { app.dirty };
                let busy = app.input.holding() || app.shell.busy();
                let wait = match (live || remote, app.input.xinput_active()) {
                    (true, true) => 16,
                    (true, false) => 250,
                    (false, true) => 50,
                    (false, false) => 500,
                };
                (draw, if busy { wait.min(16) } else { wait }, home, app.draw_errors)
            });
            let Some((draw, wait, home, errors)) = step else { continue };
            if home {
                home_button(hwnd, false, t);
            }
            if errors >= 3 {
                log("graphics failed three times; restarting Sofa");
                relaunch = true;
                break 'run;
            }
            if draw {
                with_app(|app| app.draw(t));
            } else {
                MsgWaitForMultipleObjects(None, false, wait, QS_ALLINPUT);
            }
        }
        with_app(|app| app.save());
        if relaunch && let Ok(exe) = std::env::current_exe() {
            let mut rest: Vec<String> = args.iter().skip(1).filter(|a| *a != "--relaunch").cloned().collect();
            rest.push("--relaunch".into());
            let _ = std::process::Command::new(exe).args(rest).spawn();
        }
    }
    Ok(())
}
