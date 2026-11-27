//! Sofa on Linux: its X11 window (winit; XWayland under Wayland, so the window manager calls in
//! `x11.rs` reach it), the event loop, one Sofa at a time, and the setup, uninstall and scan
//! modes. As on Windows (`main.rs`), Sofa draws only while something moves and otherwise sleeps
//! until an event, a pad, another thread's message or the next timer.

use crate::input::Action;
use crate::{gfx, input, install, media, platform, shell, social, spotify, startmenu, store, ui};
use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Icon, Window, WindowId};

/// Quit Sofa, from the UI or a signal.
pub const WM_QUIT: u32 = 0x8000 + 17;

/// What other threads hand the loop.
pub enum Msg {
    /// One of Sofa's window messages (`WM_APP + n`) and its parameter.
    Post(u32, usize),
    /// A QA run's command, from its control pipe.
    Qa(String),
}

static PROXY: OnceLock<Mutex<EventLoopProxy<Msg>>> = OnceLock::new();

/// Hands the loop one of Sofa's messages, from any thread.
pub fn post(msg: u32, w: usize) {
    if let Some(p) = PROXY.get()
        && let Ok(p) = p.lock()
    {
        let _ = p.send_event(Msg::Post(msg, w));
    }
}

/// A key as Sofa reads it: from the keyboard, or posted by a QA run.
struct KeyIn {
    key: Key,
    text: Option<String>,
    repeat: bool,
}

/// The keys Sofa reads as actions (as `key_action` on Windows).
fn key_action(k: &Key) -> Option<Action> {
    use Action::*;
    match k {
        Key::Named(n) => Some(match n {
            NamedKey::ArrowUp => Up,
            NamedKey::ArrowDown => Down,
            NamedKey::ArrowLeft => Left,
            NamedKey::ArrowRight => Right,
            NamedKey::Enter | NamedKey::Space => Accept,
            NamedKey::Escape | NamedKey::Backspace => Back,
            NamedKey::Home => Home,
            _ => return None,
        }),
        Key::Character(c) => Some(match c.to_lowercase().as_str() {
            "m" => Menu,
            "f" => Search,
            "o" => Options,
            "q" | "[" => TabPrev,
            "e" | "]" => TabNext,
            "-" => VolDown,
            "=" => VolUp,
            _ => return None,
        }),
        _ => None,
    }
}

/// The number a key Sofa reads is tracked by while it is down (`Input::key_event`).
fn key_id(a: Action) -> u16 {
    a as u16 + 1
}

/// A QA run's key name (`qa.sh` uses the same names as `qa.ps1`).
fn qa_key(name: &str) -> Option<Key> {
    Some(match name {
        "Left" => Key::Named(NamedKey::ArrowLeft),
        "Up" => Key::Named(NamedKey::ArrowUp),
        "Right" => Key::Named(NamedKey::ArrowRight),
        "Down" => Key::Named(NamedKey::ArrowDown),
        "Enter" => Key::Named(NamedKey::Enter),
        "Esc" => Key::Named(NamedKey::Escape),
        "Back" => Key::Named(NamedKey::Backspace),
        "Home" => Key::Named(NamedKey::Home),
        "Space" => Key::Named(NamedKey::Space),
        "VolUp" => Key::Character("=".into()),
        "VolDown" => Key::Character("-".into()),
        n if n.chars().count() == 1 => Key::Character(n.to_lowercase().into()),
        _ => return None,
    })
}

struct Runner {
    args: Vec<String>,
    data: PathBuf,
    qa: bool,
    window: Option<Rc<Window>>,
    app: Option<ui::App>,
    id: isize,
    focused: bool,
    mods: ModifiersState,
    /// When the next frame may be drawn (60 a second while something moves).
    next_frame: f64,
    relaunch: bool,
}

const FRAME_MS: f64 = 1000.0 / 60.0;

fn log_state(app: &ui::App) {
    if crate::LOG.get().is_some() {
        crate::log(&app.state_line());
    }
}

impl Runner {
    fn front(&self) -> bool {
        self.focused && !self.window.as_ref().and_then(|w| w.is_minimized()).unwrap_or(false)
    }

    fn create(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let has = |f: &str| self.args.iter().any(|a| a == f);
        let size = crate::size_arg(&self.args);
        let windowed = has("--windowed");
        let _ = crate::FULLSCREEN.set(size.is_none() && !windowed);
        let (general, instance) = if self.qa { ("SofaQA", "sofa-qa") } else { ("Sofa", "sofa") };
        let mut attrs = Window::default_attributes().with_title("Sofa").with_name(general, instance);
        if let Ok(img) = image::load_from_memory(crate::ICON_PNG) {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            attrs = attrs.with_window_icon(Icon::from_rgba(rgba.into_raw(), w, h).ok());
        }
        attrs = match (size, windowed) {
            (Some((w, h)), _) => attrs.with_decorations(false).with_inner_size(PhysicalSize::new(w.max(1) as u32, h.max(1) as u32)).with_position(PhysicalPosition::new(40, 40)),
            (None, true) => attrs.with_inner_size(PhysicalSize::new(1600u32, 900u32)).with_position(PhysicalPosition::new(80, 60)),
            (None, false) => attrs.with_fullscreen(Some(Fullscreen::Borderless(None))),
        };
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        window.set_cursor_visible(false);
        self.id = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xlib(h)) => h.window as isize,
            Ok(RawWindowHandle::Xcb(h)) => h.window.get() as isize,
            _ => 0,
        };
        let cache = self.data.join("cache");
        let _ = std::fs::create_dir_all(&cache);
        let asset = |name: &str, bytes: &[u8]| {
            let p = cache.join(name);
            if std::fs::metadata(&p).map_or(true, |m| m.len() != bytes.len() as u64) {
                let _ = std::fs::write(&p, bytes);
            }
            p
        };
        let icon_png = asset("sofa-icon.png", crate::ICON_PNG);
        let welcome_jpg = asset("sofa-welcome.jpg", crate::WELCOME_JPG);
        let started = crate::now();
        let g = gfx::Gfx::new(window.clone(), crate::FONT)?;
        let inp = input::Input::new(crate::START.with(|s| *s));
        let sh = shell::Shell::new(self.id, false);
        let mut app = ui::App::new(g, inp, sh, self.data.clone(), icon_png, welcome_jpg, self.qa, crate::now());
        if !self.qa {
            shell::watch_foreground();
        }
        app.startup(crate::now());
        app.boot_at = crate::now();
        app.draw(crate::now());
        crate::log(&format!("ready in {:.0} ms ({} items)", crate::now() - started, app.state_line()));
        self.focused = !self.qa;
        if !self.qa {
            window.focus_window();
            platform::focus_id(self.id);
        }
        self.window = Some(window);
        self.app = Some(app);
        Ok(())
    }

    /// The Home button, from a controller (`ps`: the PS button itself, which can be turned off).
    fn home_button(&mut self, ps: bool, t: f64) {
        let front = self.front();
        let id = self.id;
        let Some(app) = self.app.as_mut() else { return };
        if ps && !app.input.ps_home {
            return;
        }
        // One press can arrive twice (hidraw and evdev, or a pad and Steam's copy of it).
        if t - app.last_home < 350.0 {
            return;
        }
        app.last_home = t;
        if front || app.qa {
            app.action(Action::Home, t);
        } else {
            let came = app.shell.home_pressed(t);
            platform::focus_id(id);
            app.ps_home(came, t);
        }
        log_state(app);
    }

    fn key_down(&mut self, k: KeyIn, t: f64) {
        let ctrl = self.mods.control_key();
        let Some(app) = self.app.as_mut() else { return };
        let action = key_action(&k.key);
        if let Some(a) = action {
            app.input.key_event(key_id(a), true);
        }
        if app.text_mode() {
            // Typing: these keys edit or move; everything else types its text.
            match &k.key {
                Key::Named(NamedKey::Backspace) => {
                    app.erase();
                    app.kb_mark_typed();
                }
                Key::Character(c) if ctrl && c.eq_ignore_ascii_case("v") => {
                    app.paste();
                    app.kb_mark_typed();
                }
                Key::Named(NamedKey::Enter) => app.kb_enter(t),
                Key::Named(NamedKey::Escape) => app.action(Action::Back, t),
                Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown | NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                    if let Some(a) = action {
                        app.action(a, t);
                    }
                }
                _ if !ctrl => {
                    for c in k.text.as_deref().unwrap_or("").chars().filter(|c| !c.is_control()) {
                        app.type_char(c);
                    }
                }
                _ => {}
            }
            log_state(app);
            return;
        }
        let Some(a) = action else { return };
        let dir = matches!(a, Action::Up | Action::Down | Action::Left | Action::Right);
        if !k.repeat || dir {
            // A QA run reads only its own keys: someone playing with the controller during the
            // run must not make it drop them as that pad's presses.
            if let Some(a) = if app.qa { Some(a) } else { app.input.key(a, t) } {
                app.action(a, t);
            }
            log_state(app);
        }
    }

    fn key_up(&mut self, k: &Key) {
        if let (Some(app), Some(a)) = (self.app.as_mut(), key_action(k)) {
            app.input.key_event(key_id(a), false);
        }
    }

    /// One of Sofa's messages from another thread.
    fn message(&mut self, msg: u32, w: usize, el: &ActiveEventLoop, t: f64) {
        let qa = self.qa;
        match msg {
            input::WM_PAD_HOME => {
                if !qa {
                    self.home_button(w == 0, t);
                }
                return;
            }
            WM_QUIT => {
                if let Some(app) = self.app.as_mut() {
                    app.save();
                }
                el.exit();
                return;
            }
            _ => {}
        }
        let id = self.id;
        let Some(app) = self.app.as_mut() else { return };
        match msg {
            gfx::WM_IMAGE => {
                if app.gfx.take_decoded() {
                    app.dirty = true;
                }
            }
            input::WM_PAD_MUTE if !qa => {
                app.mute_button(t);
                log_state(app);
            }
            ui::WM_LIBRARY => {
                app.library_changed();
                crate::log(&format!("library: {} items", app.state_line()));
            }
            ui::WM_ICON => {
                app.gfx.retry_failed();
                app.dirty = true;
            }
            ui::WM_STEAM => app.steam_result(w),
            social::WM_SOCIAL => {
                app.social_changed(t);
                log_state(app);
            }
            media::WM_MEDIA => app.media_changed(),
            spotify::WM_MUSIC => {
                app.music_changed(t);
                log_state(app);
            }
            crate::WM_SHOW => {
                if !app.qa {
                    platform::focus_id(id);
                }
                app.go_home(t);
            }
            // Pads changed, or the window manager has news: the loop polls and ticks next.
            _ => {}
        }
    }

    /// A QA run's command: keys, text, the PS button, screenshots and the QA messages.
    fn qa_command(&mut self, line: &str, el: &ActiveEventLoop, t: f64) {
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        match cmd {
            "key" => {
                let times = rest.split_whitespace().nth(1).and_then(|n| n.parse().ok()).unwrap_or(1);
                let name = rest.split_whitespace().next().unwrap_or("");
                for _ in 0..times {
                    if let Some(k) = qa_key(name) {
                        let text = match &k {
                            Key::Character(c) => Some(c.to_string()),
                            Key::Named(NamedKey::Space) => Some(" ".into()),
                            _ => None,
                        };
                        self.key_down(KeyIn { key: k.clone(), text, repeat: false }, t);
                        self.key_up(&k);
                    }
                }
            }
            // A key held (R4, R5: a window chosen with a press waits for its release).
            "down" | "up" => {
                if let Some(k) = qa_key(rest.trim()) {
                    if cmd == "down" {
                        self.key_down(KeyIn { key: k, text: None, repeat: false }, t);
                    } else {
                        self.key_up(&k);
                    }
                }
            }
            "type" => {
                for c in rest.chars() {
                    self.key_down(KeyIn { key: Key::Character(c.to_string().into()), text: Some(c.to_string()), repeat: false }, t);
                }
            }
            "home" => self.home_button(false, t),
            "trim" => {
                if let Some(app) = self.app.as_mut() {
                    app.trim();
                }
            }
            "sleep" => {
                if let Some(app) = self.app.as_mut() {
                    app.qa_sleep_soon();
                    app.animating(t);
                    log_state(app);
                }
            }
            "run" | "ps" => {
                if let (Some(app), Ok(h)) = (self.app.as_mut(), rest.trim().parse::<isize>()) {
                    let win = shell::window(h);
                    if cmd == "run" {
                        app.qa_running(win, t);
                    } else {
                        app.ps_home(Some(win), t);
                    }
                    log_state(app);
                }
            }
            "shot" => {
                if let Some(app) = self.app.as_mut() {
                    app.draw(t);
                    let (w, h, rgba) = app.gfx.snapshot();
                    let dir = self.data.join("shots");
                    let _ = std::fs::create_dir_all(&dir);
                    let name: String = rest.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
                    let ok = image::save_buffer(dir.join(format!("{name}.png")), &rgba, w, h, image::ExtendedColorType::Rgba8).is_ok();
                    crate::log(&format!("shot {name} {}", if ok { "saved" } else { "failed" }));
                }
            }
            "quit" => {
                if let Some(app) = self.app.as_mut() {
                    app.save();
                }
                el.exit();
            }
            _ => crate::log(&format!("qa: unknown command {line}")),
        }
    }

    /// The loop's step: focus, the window manager's news, pads, timers; then a frame or a wait.
    fn step(&mut self, el: &ActiveEventLoop) {
        let t = crate::now();
        let front = self.front();
        let visible = !self.window.as_ref().and_then(|w| w.is_minimized()).unwrap_or(false);
        let id = self.id;
        let Some(app) = self.app.as_mut() else { return };
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
            crate::log(&format!("shell {e:?}"));
            match e {
                shell::Event::BringBack => {
                    platform::focus_id(id);
                    app.go_home(t);
                }
                shell::Event::SteamGrab => app.steam_grab(),
                shell::Event::Ended(h) => app.running_ended(h),
                shell::Event::Ready(h) => app.media_ready(h),
            }
        }
        let mut home = false;
        let focused = app.focused;
        let remote = app.remote_active();
        let pads = app.input.poll(t, focused || remote);
        let qa = app.qa;
        for a in pads.into_iter().filter(|_| !qa) {
            if a == Action::Home {
                home = true;
            } else if focused {
                platform::user_active();
                app.action(a, t);
                log_state(app);
            } else if remote {
                app.remote_action(a);
            }
        }
        app.trim_if_idle(t);
        let anim = app.animating(t);
        // While Sofa's launch screen covers a starting browser, it keeps animating.
        let live = focused || app.qa || app.shell.covering();
        let draw = visible && if live { anim } else { app.dirty };
        let busy = app.input.holding() || app.shell.busy() || app.resume_waiting();
        let wait = if live || remote { 250.0 } else { 500.0 };
        let wait = if busy { 16.0 } else { wait };
        let errors = app.draw_errors;
        if home {
            self.home_button(false, t);
        }
        if errors >= 3 {
            crate::log("graphics failed three times; restarting Sofa");
            self.relaunch = true;
            el.exit();
            return;
        }
        let until = if draw {
            if t >= self.next_frame {
                if let Some(app) = self.app.as_mut() {
                    app.draw(t);
                }
                self.next_frame = t + FRAME_MS;
            }
            self.next_frame
        } else {
            t + wait
        };
        let wake = std::time::Instant::now() + std::time::Duration::from_secs_f64(((until - crate::now()).max(1.0)) / 1000.0);
        el.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}

impl ApplicationHandler<Msg> for Runner {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.create(el) {
            crate::log(&format!("start failed: {e}"));
            eprintln!("Sofa couldn't open its window: {e}");
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let t = crate::now();
        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                if let Some(app) = self.app.as_mut() {
                    app.save();
                }
                el.exit();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(app) = self.app.as_mut() {
                    let _ = app.gfx.resize();
                    app.dirty = true;
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(app) = self.app.as_mut() {
                    app.dirty = true;
                }
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                let front = self.front();
                if let Some(app) = self.app.as_mut()
                    && !app.qa
                {
                    app.on_focus(front, t);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event: KeyEvent { logical_key, text, state, repeat, .. }, is_synthetic, .. } => {
                // Keys held while the focus arrived are not presses.
                if is_synthetic {
                    return;
                }
                if state == ElementState::Pressed {
                    self.key_down(KeyIn { key: logical_key, text: text.map(|s| s.to_string()), repeat }, t);
                } else {
                    self.key_up(&logical_key);
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, msg: Msg) {
        let t = crate::now();
        match msg {
            Msg::Post(m, w) => self.message(m, w, el, t),
            Msg::Qa(line) if self.qa => self.qa_command(line.trim(), el, t),
            Msg::Qa(_) => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.step(el);
    }
}

/// The socket a second Sofa uses to bring the first forward.
fn socket_path(data: &Path, qa: bool) -> PathBuf {
    if qa {
        return data.join("sofa.sock");
    }
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(dir) => dir.join("sofa.sock"),
        // SAFETY: plain query.
        None => std::env::temp_dir().join(format!("sofa-{}.sock", unsafe { libc::getuid() })),
    }
}

/// One Sofa at a time: the first listens; a second start asks it to come forward and ends.
/// Returns false when another Sofa answered.
fn single_instance(path: &Path, relaunch: bool) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match UnixStream::connect(path) {
            Ok(mut s) if !relaunch => {
                let _ = s.write_all(b"show\n");
                return false;
            }
            // Restarting after a graphics failure: wait for the old process to end.
            Ok(_) if std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(200)),
            _ => break,
        }
    }
    let _ = std::fs::remove_file(path);
    let Ok(listener) = UnixListener::bind(path) else { return true };
    let _ = std::thread::Builder::new().name("instance".into()).spawn(move || {
        for s in listener.incoming().flatten() {
            let mut line = String::new();
            if std::io::BufReader::new(s).read_line(&mut line).is_ok() && line.trim() == "show" {
                post(crate::WM_SHOW, 0);
            }
        }
    });
    true
}

/// A QA run's control pipe (`qa.sh` writes one command per line).
fn qa_pipe(data: &Path, proxy: EventLoopProxy<Msg>) {
    let fifo = data.join("control");
    let _ = std::fs::remove_file(&fifo);
    let Ok(c) = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()) else { return };
    // SAFETY: creates a named pipe at a path we own.
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
        return;
    }
    let _ = std::thread::Builder::new().name("qa".into()).spawn(move || {
        loop {
            let Ok(f) = std::fs::File::open(&fifo) else { return };
            for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
                if proxy.send_event(Msg::Qa(line)).is_err() {
                    return;
                }
            }
        }
    });
}

/// SIGTERM (the session ending), SIGINT and SIGHUP save and quit like Quit Sofa. They are
/// blocked in every thread and taken by one of their own; programs Sofa starts get a clean mask
/// from `std::process`.
fn catch_signals() {
    // SAFETY: a signal set on the stack, blocked for this thread (and the threads it starts).
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, s);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        let _ = std::thread::Builder::new().name("signals".into()).spawn(move || {
            let mut sig = 0;
            if libc::sigwait(&set, &mut sig) == 0 {
                post(WM_QUIT, 0);
            }
        });
    }
}

/// Asks a yes or no question on the desktop (zenity or kdialog), else in the terminal.
fn ask(title: &str, text: &str) -> bool {
    use std::process::Command;
    for (prog, args) in [("zenity", vec!["--question", "--title", title, "--text", text]), ("kdialog", vec!["--title", title, "--yesno", text])] {
        if crate::linux::desktop::which(prog).is_some() {
            return Command::new(prog).args(args).status().is_ok_and(|s| s.success());
        }
    }
    // SAFETY: plain query.
    if unsafe { libc::isatty(0) } == 1 {
        print!("{text} [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        return std::io::stdin().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("y");
    }
    false
}

/// Shows an error on the desktop (zenity or kdialog) and in the terminal.
fn tell(title: &str, text: &str) {
    eprintln!("{text}");
    for (prog, args) in [("zenity", vec!["--error", "--title", title, "--text", text]), ("kdialog", vec!["--title", title, "--error", text])] {
        if crate::linux::desktop::which(prog).is_some() {
            let _ = std::process::Command::new(prog).args(args).status();
            return;
        }
    }
}

pub fn main() -> std::process::ExitCode {
    use std::process::ExitCode;
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let qa = has("--qa");
    let _ = crate::QA.set(qa);

    // `--scan <cache> [--jobs <file>] [--no-startmenu]`: the background child that lists the
    // desktop entries and draws their icons, then exits (see `ui::spawn_scan`).
    if let Some(i) = args.iter().position(|a| a == "--scan") {
        let Some(cache) = args.get(i + 1).map(PathBuf::from) else { return ExitCode::SUCCESS };
        if !has("--no-startmenu") {
            startmenu::scan(&cache);
        }
        if let Some(file) = args.iter().position(|a| a == "--jobs").and_then(|j| args.get(j + 1)).map(PathBuf::from) {
            let jobs: Vec<(String, String)> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            for (exe, png) in jobs {
                startmenu::save_exe_icon(&exe, Path::new(&png));
            }
            let _ = std::fs::remove_file(&file);
        }
        return ExitCode::SUCCESS;
    }

    if has("--uninstall") {
        // `--quiet` skips the question (scripts).
        if has("--quiet") || ask("Uninstall Sofa", "Remove Sofa from this computer? Your settings stay, so installing it again picks up where you left off.") {
            install::uninstall();
        }
        return ExitCode::SUCCESS;
    }
    if install::is_setup() && !qa {
        let autostart = store::Store::open(&store::data_dir()).flag("launchAtLogin");
        match install::install(env!("CARGO_PKG_VERSION"), autostart) {
            Ok(exe) => {
                platform::spawn_detached(std::process::Command::new(&exe));
            }
            Err(e) => tell("Sofa", &format!("Sofa couldn't be installed. {e}")),
        }
        return ExitCode::SUCCESS;
    }

    let data = if qa { std::env::temp_dir().join("sofa-qa") } else { store::data_dir() };
    let _ = std::fs::create_dir_all(&data);
    if qa {
        let path = data.join("qa.log");
        let _ = std::fs::remove_file(&path);
        let _ = crate::LOG.set(path);
    } else if has("--log") {
        let _ = crate::LOG.set(std::env::temp_dir().join("sofa-native.log"));
    }
    // SAFETY: called once, before any thread reads the locale; only the time format changes.
    unsafe {
        libc::setlocale(libc::LC_TIME, c"".as_ptr());
    }
    if !single_instance(&socket_path(&data, qa), has("--relaunch")) {
        return ExitCode::SUCCESS;
    }
    catch_signals();
    if !crate::linux::x11::available() {
        tell("Sofa", "Sofa needs an X11 or XWayland display. Start it from your desktop session.");
        return ExitCode::FAILURE;
    }
    let el = match EventLoop::<Msg>::with_user_event().with_x11().build() {
        Ok(el) => el,
        Err(e) => {
            tell("Sofa", &format!("Sofa couldn't open its window: {e}"));
            return ExitCode::FAILURE;
        }
    };
    let _ = PROXY.set(Mutex::new(el.create_proxy()));
    if qa {
        qa_pipe(&data, el.create_proxy());
    }
    el.set_control_flow(ControlFlow::Wait);
    let mut runner = Runner { args: args.clone(), data, qa, window: None, app: None, id: 0, focused: false, mods: ModifiersState::default(), next_frame: 0.0, relaunch: false };
    let _ = el.run_app(&mut runner);
    if let Some(app) = runner.app.as_mut() {
        app.save();
    }
    if runner.relaunch
        && let Ok(exe) = std::env::current_exe()
    {
        let mut rest: Vec<String> = args.iter().skip(1).filter(|a| *a != "--relaunch").cloned().collect();
        rest.push("--relaunch".into());
        let mut c = std::process::Command::new(exe);
        c.args(rest);
        platform::spawn_detached(c);
    }
    ExitCode::SUCCESS
}
