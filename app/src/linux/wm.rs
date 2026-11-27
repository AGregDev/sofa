//! What `shell.rs` asks of the window manager, on Linux: the same functions the Windows half
//! of `shell.rs` provides, built on `x11` (EWMH), `proc` and the desktop entries.

use crate::linux::{desktop, proc, x11};
use crate::shell::Window;
use std::path::PathBuf;
use std::sync::Mutex;

/// A foreground change from the watch thread (Windows has a WinEvent hook instead).
pub const WM_FOREGROUND: u32 = 0x8000 + 16;

static EVENTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// The window in front (0: none, or one X11 cannot see: a Wayland app, the overview).
pub fn foreground() -> isize {
    x11::active()
}

/// Asks the window manager to tell Sofa whenever another window comes to the front.
pub fn watch_foreground() {
    x11::watch(|w| {
        if let Some(w) = w
            && let Ok(mut e) = EVENTS.lock()
        {
            e.push(w);
        }
        crate::post(0, WM_FOREGROUND, 0);
    });
}

pub fn take_foreground_events() -> Vec<isize> {
    EVENTS.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default()
}

pub fn title(h: isize) -> String {
    x11::title(h)
}

pub fn pid(h: isize) -> u32 {
    x11::pid(h)
}

pub fn exe_of(pid: u32) -> String {
    if pid == 0 { String::new() } else { proc::exe(pid) }
}

/// When a window's process started (epoch ms): how long a game has been running.
pub fn started(h: isize) -> Option<f64> {
    match pid(h) {
        0 => None,
        p => proc::started(p),
    }
}

fn exe_name(h: isize) -> String {
    let e = exe_of(pid(h)).to_lowercase();
    e.rsplit('/').next().unwrap_or("").to_owned()
}

pub fn window(h: isize) -> Window {
    let exe = exe_of(pid(h));
    let app = desktop::name_for(&exe, &x11::class(h));
    Window { hwnd: h, title: title(h).replace('\t', " "), exe, app }
}

/// The desktop and its panels: the parts of the desktop Sofa stands in for. (Nothing in front
/// is not counted: under Wayland that is any Wayland app, which the person may well be using.)
pub fn is_shell(h: isize) -> bool {
    if h == 0 {
        return false;
    }
    if x11::is_desktop_or_panel(h) {
        return true;
    }
    let (inst, class) = x11::class(h);
    ["plasmashell", "xfdesktop", "xfce4-panel", "nemo-desktop", "desktop_window", "cinnamon", "gnome-shell"].iter().any(|n| inst.eq_ignore_ascii_case(n) || class.eq_ignore_ascii_case(n))
}

pub fn is_steam(h: isize) -> bool {
    if h == 0 {
        return false;
    }
    let (inst, class) = x11::class(h);
    let steamish = |s: &str| s.eq_ignore_ascii_case("steam") || s.eq_ignore_ascii_case("steamwebhelper");
    steamish(&inst) || steamish(&class) || matches!(exe_name(h).as_str(), "steam" | "steamwebhelper")
}

pub fn is_fullscreen(h: isize) -> bool {
    h != 0 && x11::fullscreen(h)
}

/// Whether the window `h` belongs to the browser Sofa started from `exe` (lower case): the
/// class Sofa gave it, or a program in the same folder (`google-chrome` starts `chrome`).
pub fn same_program(h: isize, exe: &str) -> bool {
    let (inst, class) = x11::class(h);
    if class.eq_ignore_ascii_case(crate::platform::WEB_CLASS) || inst.eq_ignore_ascii_case(crate::platform::WEB_CLASS) {
        return true;
    }
    let mine = exe_of(pid(h));
    if mine.to_lowercase() == exe {
        return true;
    }
    let dir = |p: &str| proc::real_dir(std::path::Path::new(p));
    matches!((dir(&mine), dir(exe)), (Some(a), Some(b)) if a == b)
}

/// The first open window of a program (`discord.exe` or `discord`).
pub fn find_window(exe: &str) -> Option<isize> {
    let want = exe.to_lowercase();
    let want = want.strip_suffix(".exe").unwrap_or(&want).to_owned();
    top_windows().into_iter().find(|&h| {
        let (inst, class) = x11::class(h);
        exe_name(h) == want || inst.eq_ignore_ascii_case(&want) || class.eq_ignore_ascii_case(&want)
    })
}

/// The titled application windows, front to back.
pub fn top_windows() -> Vec<isize> {
    x11::stacking().into_iter().filter(|&h| !x11::is_desktop_or_panel(h) && !title(h).is_empty()).collect()
}

pub fn alive(h: isize) -> bool {
    x11::exists(h)
}

/// Asks a window to close, as its own close button would.
pub fn close(h: isize, sofa: isize) {
    if h != 0 && h != sofa && alive(h) {
        x11::close(h);
    }
}

/// Ends the program behind a window that stopped responding. Never the desktop itself.
pub fn kill(h: isize) {
    let p = pid(h);
    if p == 0 || p == std::process::id() {
        return;
    }
    let name = exe_name(h);
    if matches!(name.as_str(), "gnome-shell" | "plasmashell" | "kwin_x11" | "kwin_wayland" | "xwayland" | "xorg" | "mutter") {
        return;
    }
    proc::signal(p, true);
}

pub fn minimize(h: isize) {
    x11::minimize(h);
}

/// Sofa above the other windows, or back among them.
pub fn topmost(sofa: isize, on: bool) {
    x11::set_above(sofa, on);
}

/// The icon for a Running card: the program's desktop entry icon when it is a PNG (used where
/// it is), else `png` in Sofa's cache, written now from the window's own icon when the entry has
/// none (an SVG one is drawn into `png` by the `--scan` child).
pub fn running_icon(w: &Window, png: PathBuf, qa: bool) -> PathBuf {
    if png.exists() || qa {
        return png;
    }
    let icon = desktop::entry_for(&w.exe, &x11::class(w.hwnd)).and_then(|e| desktop::find_icon(&e.icon));
    match icon {
        Some(p) if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) => p,
        Some(_) => png,
        None => {
            if let Some((iw, ih, rgba)) = x11::icon(w.hwnd) {
                if let Some(d) = png.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = image::save_buffer(&png, &rgba, iw, ih, image::ExtendedColorType::Rgba8);
            }
            png
        }
    }
}
