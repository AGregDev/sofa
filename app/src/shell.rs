//! The console behaviour: Sofa is the home screen, not an app. Ported from Sofa 1.x's
//! `main.js` and `padwatch.ps1`, but driven by Windows' foreground-change events instead of a
//! helper process polling every 30 ms:
//! - Sofa stays on top while it is home, and steps down when it hands the screen to a game;
//! - the desktop, taskbar, Start menu or Search showing up brings Sofa back (the guard);
//! - a full-screen app that was in front closing brings Sofa back;
//! - Steam grabbing the screen right after the PS button is pushed back down;
//! - the PS button remembers the app it came from, for the Running card.
//!
//! The window manager's side is below for Windows; on Linux it is `linux/wm.rs` (EWMH).

#[cfg(target_os = "linux")]
pub use crate::linux::wm::{alive, close, exe_of, find_window, foreground, is_fullscreen, is_shell, is_steam, kill, minimize, pid, same_program, started, take_foreground_events, title, top_windows, topmost, watch_foreground, window};
#[cfg(windows)]
use std::cell::RefCell;
#[cfg(windows)]
use windows::Win32::Foundation::*;
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
#[cfg(windows)]
use windows::Win32::System::Threading::*;
#[cfg(windows)]
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::*;
#[cfg(windows)]
use windows::core::PWSTR;

/// On Windows nothing in front means the desktop showed; under Wayland it is any Wayland app.
const NOTHING_IS_SHELL: bool = cfg!(windows);

#[cfg(windows)]
thread_local! {
    /// Foreground changes reported by the hook, handled by the main loop.
    static FOREGROUND: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

#[cfg(windows)]
unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    FOREGROUND.with(|f| f.borrow_mut().push(hwnd.0 as isize));
}

/// Asks Windows to tell Sofa whenever another window comes to the front.
#[cfg(windows)]
pub fn watch_foreground() {
    // SAFETY: an out-of-context hook delivered on this thread's message loop.
    unsafe {
        let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
    }
}

#[cfg(windows)]
pub fn take_foreground_events() -> Vec<isize> {
    FOREGROUND.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// A window and the program behind it.
#[derive(Clone, Debug)]
pub struct Window {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
    /// The program's own name ("Firefox", "Google Chrome"), from its file description; empty
    /// when the window's title names the app better (Windows' app hosts, script runtimes).
    pub app: String,
}

/// A program's name from its version resource (FileDescription, else ProductName).
#[cfg(windows)]
pub fn describe(exe: &str) -> String {
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows::core::HSTRING;
    let lower = exe.to_lowercase();
    let base = lower.rsplit('\\').next().unwrap_or("");
    // These host other apps: the window's title is the app's name.
    if exe.is_empty()
        || matches!(
            base,
            "applicationframehost.exe" | "java.exe" | "javaw.exe" | "python.exe" | "pythonw.exe" | "rundll32.exe" | "dllhost.exe" | "mmc.exe" | "cmd.exe" | "conhost.exe" | "explorer.exe"
        )
    {
        return String::new();
    }
    let path = HSTRING::from(exe);
    // SAFETY: the version block is sized as Windows reports and outlives every pointer
    // VerQueryValueW returns into it; lengths come from Windows.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return String::new();
        }
        let mut block = vec![0u8; size as usize];
        if GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()).is_err() {
            return String::new();
        }
        let query = |sub: &str| -> Option<(*const u8, u32)> {
            let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
            let mut n = 0u32;
            VerQueryValueW(block.as_ptr().cast(), &HSTRING::from(sub), &mut p, &mut n).as_bool().then_some((p as *const u8, n)).filter(|(p, n)| !p.is_null() && *n > 0)
        };
        let mut langs: Vec<(u16, u16)> = Vec::new();
        if let Some((p, n)) = query("\\VarFileInfo\\Translation") {
            for i in 0..(n as usize / 4) {
                let w = p.add(i * 4).cast::<u16>();
                langs.push((w.read_unaligned(), w.add(1).read_unaligned()));
            }
        }
        langs.extend([(0x0409, 0x04b0), (0x0409, 0x04e4), (0x0000, 0x04b0)]);
        for key in ["FileDescription", "ProductName"] {
            for (lang, cp) in &langs {
                if let Some((p, n)) = query(&format!("\\StringFileInfo\\{lang:04x}{cp:04x}\\{key}")) {
                    let chars = std::slice::from_raw_parts(p.cast::<u16>(), n as usize);
                    let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
                    let s = String::from_utf16_lossy(&chars[..len]).replace(['®', '™'], "");
                    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !s.is_empty() {
                        return s;
                    }
                }
            }
        }
        String::new()
    }
}

/// The page or document a window shows, without the app's name ("AGregDev/sofa — Mozilla
/// Firefox" in Firefox is "AGregDev/sofa"). Empty when the title is only the app's name.
pub fn page_title(title: &str, app: &str) -> String {
    let t = title.trim();
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let app_n = norm(app);
    if app_n.is_empty() || norm(t) == app_n {
        return if app_n.is_empty() { t.to_owned() } else { String::new() };
    }
    for sep in [" \u{2014} ", " \u{2013} ", " - ", " | "] {
        if let Some((page, last)) = t.rsplit_once(sep) {
            let last_n = norm(last);
            if !last_n.is_empty() && (last_n.contains(&app_n) || app_n.contains(&last_n)) {
                return page.trim().to_owned();
            }
        }
    }
    t.to_owned()
}

#[cfg(windows)]
fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

#[cfg(windows)]
pub fn class_name(h: isize) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetClassNameW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

#[cfg(windows)]
pub fn title(h: isize) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetWindowTextW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

#[cfg(windows)]
pub fn pid(h: isize) -> u32 {
    let mut pid = 0u32;
    // SAFETY: writes into a local.
    unsafe { GetWindowThreadProcessId(hwnd(h), Some(&mut pid)) };
    pid
}

/// The full path of a process's program (empty when Windows does not say).
#[cfg(windows)]
pub fn exe_of(pid: u32) -> String {
    // SAFETY: a limited-query handle, closed before returning; the buffer is sized.
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(p);
        if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() }
    }
}

/// When a window's process started (epoch ms): how long a game has been running.
#[cfg(windows)]
pub fn started(h: isize) -> Option<f64> {
    // SAFETY: a limited-query handle, closed before returning; the times are locals.
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid(h)).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(p, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(p);
        // FILETIME counts 100 ns since 1601; the Unix epoch is 11 644 473 600 s later.
        let t = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        (ok && t > 0).then_some((t / 10_000) as f64 - 11_644_473_600_000.0)
    }
}

#[cfg(windows)]
fn exe_name(h: isize) -> String {
    let e = exe_of(pid(h)).to_lowercase();
    e.rsplit('\\').next().unwrap_or("").to_owned()
}

#[cfg(windows)]
pub fn window(h: isize) -> Window {
    let exe = exe_of(pid(h));
    let app = describe(&exe);
    Window { hwnd: h, title: title(h).replace('\t', " "), exe, app }
}

/// The desktop, taskbar, Start menu and Windows Search: the parts of Windows that Sofa stands
/// in for while it runs.
#[cfg(windows)]
pub fn is_shell(h: isize) -> bool {
    if h == 0 {
        return true;
    }
    let c = class_name(h);
    if matches!(c.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "NotifyIconOverflowWindow") {
        return true;
    }
    matches!(exe_name(h).as_str(), "searchhost.exe" | "startmenuexperiencehost.exe" | "shellexperiencehost.exe")
}

#[cfg(windows)]
pub fn is_steam(h: isize) -> bool {
    matches!(exe_name(h).as_str(), "steam.exe" | "steamwebhelper.exe")
}

#[cfg(windows)]
fn is_fullscreen(h: isize) -> bool {
    // SAFETY: plain queries into local structs.
    unsafe {
        let mut r = RECT::default();
        if GetWindowRect(hwnd(h), &mut r).is_err() {
            return false;
        }
        let m = MonitorFromWindow(hwnd(h), MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(m, &mut mi).as_bool() {
            return false;
        }
        r.left <= mi.rcMonitor.left && r.top <= mi.rcMonitor.top && r.right >= mi.rcMonitor.right && r.bottom >= mi.rcMonitor.bottom
    }
}

/// The first visible, titled top-level window of a program (`discord.exe`).
#[cfg(windows)]
pub fn find_window(exe: &str) -> Option<isize> {
    top_windows().into_iter().find(|&h| exe_name(h) == exe)
}

/// The visible, titled top-level windows, front to back.
#[cfg(windows)]
pub fn top_windows() -> Vec<isize> {
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> windows::core::BOOL {
        // SAFETY: `l` is the Vec passed below, alive for the whole enumeration.
        let list = unsafe { &mut *(l.0 as *mut Vec<isize>) };
        // SAFETY: plain query.
        if unsafe { IsWindowVisible(h) }.as_bool() && !title(h.0 as isize).is_empty() {
            list.push(h.0 as isize);
        }
        windows::core::BOOL(1)
    }
    let mut all: Vec<isize> = Vec::new();
    // SAFETY: the callback only writes into `all`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut Vec<isize> as isize));
    }
    all
}

#[cfg(windows)]
pub fn alive(h: isize) -> bool {
    // SAFETY: plain queries.
    unsafe { IsWindow(Some(hwnd(h))).as_bool() && IsWindowVisible(hwnd(h)).as_bool() }
}

/// Asks a window to close, as its own close button would.
#[cfg(windows)]
pub fn close(h: isize, sofa: isize) {
    if h != 0 && h != sofa && alive(h) {
        // SAFETY: posting WM_CLOSE to another window.
        unsafe {
            let _ = PostMessageW(Some(hwnd(h)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Ends the program behind a window that stopped responding. Never Windows' shared hosts.
#[cfg(windows)]
pub fn kill(h: isize) {
    let p = pid(h);
    // SAFETY: plain query.
    if p == 0 || p == unsafe { GetCurrentProcessId() } {
        return;
    }
    let exe = exe_of(p).to_lowercase();
    if exe.ends_with("\\explorer.exe") || exe.ends_with("\\applicationframehost.exe") || exe.ends_with("\\dwm.exe") {
        return;
    }
    // SAFETY: a terminate handle, closed after use.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, p) {
            let _ = TerminateProcess(handle, 1);
            let _ = CloseHandle(handle);
        }
    }
}

/// The window in front (0: none).
#[cfg(windows)]
pub fn foreground() -> isize {
    // SAFETY: plain query.
    unsafe { GetForegroundWindow() }.0 as isize
}

#[cfg(windows)]
pub fn minimize(h: isize) {
    // SAFETY: minimizing another program's window.
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_MINIMIZE);
    }
}

/// Sofa topmost, or not. Leaving topmost puts Sofa above every other window; when something
/// else is in front already, Sofa goes right behind it instead of covering it.
#[cfg(windows)]
fn topmost(sofa: isize, on: bool) {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    // SAFETY: changing our own window's z-order.
    unsafe {
        let _ = SetWindowPos(hwnd(sofa), Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, flags);
        let fg = GetForegroundWindow();
        if !on && !fg.is_invalid() && fg.0 as isize != sofa {
            let _ = SetWindowPos(hwnd(sofa), Some(fg), 0, 0, 0, 0, flags);
        }
    }
}

/// Whether the window `h` belongs to the program `exe` (lower case).
#[cfg(windows)]
fn same_program(h: isize, exe: &str) -> bool {
    exe_of(pid(h)).to_lowercase() == exe
}

/// What the main loop should do after a foreground change or a tick.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Bring Sofa back to the front (the desktop showed, or a game closed).
    BringBack,
    /// Steam took the screen; Sofa pushed it back. The UI offers the one-time fix.
    SteamGrab,
    /// A window Sofa tracked as running has closed.
    Ended(isize),
    /// The browser Sofa covered is ready and in front (0: it never showed; Sofa stepped aside
    /// anyway).
    Ready(isize),
}

/// A browser Sofa just started for a streaming service. Sofa's launch screen stays on top
/// until that browser's window is full screen and has had time to draw, so the person never
/// sees a window opening, resizing or painting.
struct Cover {
    exe: String,
    since: f64,
    /// The browser's window and when it was first seen full screen.
    seen: Option<(isize, Option<f64>)>,
    checked: isize,
}

pub struct Shell {
    pub sofa: isize,
    /// Keep Sofa in front (Settings).
    pub guard: bool,
    /// "Go to desktop": the guard rests until Sofa is home again.
    pub desktop: bool,
    allow_until: f64,
    home_at: f64,
    came_from: isize,
    prev: isize,
    shell_since: Option<f64>,
    session: Option<(isize, f64)>,
    blur_at: Option<f64>,
    top: bool,
    /// The last app in front that was not Sofa or Windows' shell, and whether it was full screen.
    last_app: Option<(isize, bool)>,
    /// When "nothing in front" may next bring Sofa back (retries back off: during a screen
    /// saver or a locked screen no window can take the focus).
    idle_retry: f64,
    cover: Option<Cover>,
    /// The window a streaming service plays in, once Sofa handed the screen to it.
    pub media: Option<isize>,
}

impl Shell {
    pub fn new(sofa: isize, guard: bool) -> Shell {
        Shell {
            sofa,
            guard,
            desktop: false,
            allow_until: 0.0,
            home_at: -1e9,
            came_from: 0,
            prev: sofa,
            shell_since: None,
            session: None,
            blur_at: None,
            top: false,
            last_app: None,
            idle_retry: 0.0,
            cover: None,
            media: None,
        }
    }

    /// Keeps Sofa's launch screen on top while the browser `exe` starts (see `Cover`).
    pub fn cover(&mut self, exe: &std::path::Path, now: f64) {
        self.cover = Some(Cover { exe: exe.to_string_lossy().to_lowercase(), since: now, seen: None, checked: 0 });
        self.media = None;
        self.set_top(true);
    }

    pub fn covering(&self) -> bool {
        self.cover.is_some()
    }

    /// Whether the streaming window Sofa started is in front (the controller drives it).
    pub fn media_in_front(&self) -> bool {
        let Some(m) = self.media else { return false };
        let fg = foreground();
        self.cover.is_none() && fg != 0 && alive(m) && (fg == m || pid(fg) == pid(m))
    }

    /// The covered browser's window, when one is in front: remembers it, and when it is full
    /// screen and has drawn for a moment, steps aside for it.
    fn check_cover(&mut self, fg: isize, now: f64) -> Option<Event> {
        let c = self.cover.as_mut()?;
        if fg != 0 && fg != self.sofa && fg != c.checked {
            c.checked = fg;
            if same_program(fg, &c.exe) && c.seen.is_none_or(|(h, _)| h != fg) {
                c.seen = Some((fg, None));
            }
        }
        if let Some((h, full)) = &mut c.seen
            && full.is_none()
            && alive(*h)
            && is_fullscreen(*h)
        {
            *full = Some(now);
        }
        let ready = c.seen.and_then(|(h, full)| full.filter(|t| now - t > 700.0).map(|_| h)).filter(|h| alive(*h));
        if ready.is_none() && now - c.since < 12_000.0 {
            return None;
        }
        self.cover = None;
        self.set_top(false);
        if let Some(h) = ready {
            self.media = Some(h);
            // A full-screen session: closing it brings Sofa back.
            self.session = Some((h, now));
            self.last_app = Some((h, true));
            crate::platform::focus_id(h);
        }
        Some(Event::Ready(ready.unwrap_or(0)))
    }

    /// Topmost while Sofa is the home screen, so nothing (Steam included) opens above it.
    pub fn set_top(&mut self, on: bool) {
        let on = on && self.guard;
        if on == self.top {
            return;
        }
        self.top = on;
        topmost(self.sofa, on);
    }

    /// Lets another window come in front of Sofa (resuming an app).
    pub fn hand_off(&mut self) {
        self.set_top(false);
    }

    /// Hands the screen to something Sofa launched: no longer topmost, and Steam may come
    /// forward for a while (a Steam game starting brings Steam up first).
    pub fn step_down(&mut self, now: f64) {
        self.allow_until = now + 20_000.0;
        self.set_top(false);
    }

    /// The app that was in front before Sofa, if it is still open (for the Running card).
    pub fn take_last_app(&mut self) -> Option<(Window, bool)> {
        self.last_app.take().filter(|(h, _)| alive(*h)).map(|(h, full)| (window(h), full))
    }

    pub fn on_focus(&mut self, focused: bool, now: f64) {
        if focused {
            self.desktop = false;
            self.blur_at = None;
            self.set_top(true);
        } else if self.cover.is_none() {
            self.blur_at = Some(now);
        }
    }

    /// The PS button. Returns the app that was in front (not Windows' shell, not Sofa).
    pub fn home_pressed(&mut self, now: f64) -> Option<Window> {
        let mut fg = foreground();
        // Steam may have answered the button first: the app before it is the one that ran.
        if is_steam(fg) {
            fg = self.last_app.map_or(0, |(h, _)| h);
        }
        self.home_at = now;
        self.came_from = fg;
        self.desktop = false;
        self.last_app = None;
        self.cover = None;
        (fg != 0 && fg != self.sofa && !is_shell(fg) && alive(fg)).then(|| window(fg))
    }

    pub fn on_foreground(&mut self, fg: isize, now: f64) -> Vec<Event> {
        let mut out = Vec::new();
        let prev = std::mem::replace(&mut self.prev, fg);
        if fg == self.sofa {
            self.shell_since = None;
            return out;
        }
        if self.cover.is_some() {
            out.extend(self.check_cover(fg, now));
            if self.cover.as_ref().is_some_and(|c| c.seen.is_some_and(|(h, _)| h == fg)) || out.iter().any(|e| matches!(e, Event::Ready(_))) {
                return out;
            }
        }
        // Steam taking the screen from Sofa, or right after the PS button, with nothing
        // launched just now, is Steam answering the PS button: send it back under Sofa.
        let after_home = now - self.home_at < 10_000.0 && fg != self.came_from;
        if (prev == self.sofa || after_home) && now > self.allow_until && is_steam(fg) {
            minimize(fg);
            crate::platform::focus_id(self.sofa);
            out.push(Event::SteamGrab);
            return out;
        }
        if is_shell(fg) {
            if self.guard && !self.desktop {
                self.shell_since.get_or_insert(now);
            }
        } else {
            self.shell_since = None;
            let full = is_fullscreen(fg);
            if full && self.session.is_none_or(|(s, _)| s != fg) {
                self.session = Some((fg, now));
            }
            if alive(fg) {
                self.last_app = Some((fg, full));
            }
        }
        out
    }

    /// Timers: the guard's grace period, the full-screen app closing, dropping topmost.
    pub fn tick(&mut self, now: f64, focused: bool) -> Vec<Event> {
        let mut out = Vec::new();
        if self.cover.is_some() {
            // The browser may be in front without an event.
            let fg = foreground();
            out.extend(self.check_cover(fg, now));
        }
        // Nothing in front at all (the app that was closed, or Sofa started in the background):
        // Windows sends no event for that, so check here.
        if NOTHING_IS_SHELL && !focused && self.guard && !self.desktop && self.shell_since.is_none() && now >= self.idle_retry && foreground() == 0 {
            self.shell_since = Some(now);
            self.idle_retry = now + 5000.0;
        }
        if let Some((s, since)) = self.session
            && !alive(s)
        {
            self.session = None;
            out.push(Event::Ended(s));
            if self.guard && !self.desktop && now - since > 2000.0 {
                out.push(Event::BringBack);
            }
        }
        if let Some(t) = self.shell_since
            && now - t > 600.0
        {
            self.shell_since = None;
            let fg = foreground();
            if is_shell(fg) && self.guard && !self.desktop {
                out.push(Event::BringBack);
            }
        }
        if let Some(t) = self.blur_at
            && !focused
            && now - t > 350.0
        {
            self.blur_at = None;
            self.set_top(false);
        }
        out
    }

    /// Whether a timer is pending (the loop should wake up soon).
    pub fn busy(&self) -> bool {
        self.shell_since.is_some() || self.blur_at.is_some() || self.session.is_some() || self.cover.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::page_title;

    #[test]
    fn page_title_drops_the_app_name() {
        assert_eq!(page_title("AGregDev/sofa: A controller-first home screen \u{2014} Mozilla Firefox", "Firefox"), "AGregDev/sofa: A controller-first home screen");
        assert_eq!(page_title("Inbox - Gmail - Google Chrome", "Google Chrome"), "Inbox - Gmail");
        assert_eq!(page_title("notes.txt - Notepad", "Notepad"), "notes.txt");
        assert_eq!(page_title("Spotify", "Spotify"), "");
        assert_eq!(page_title("Spotify Premium", "Spotify"), "Spotify Premium");
        assert_eq!(page_title("Settings", ""), "Settings");
    }
}
