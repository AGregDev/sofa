//! The console behaviour: Sofa is the home screen, not an app. Ported from Sofa 1.x's
//! `main.js` and `padwatch.ps1`, but driven by Windows' foreground-change events instead of a
//! helper process polling every 30 ms:
//! - Sofa stays on top while it is home, and steps down when it hands the screen to a game;
//! - the desktop, taskbar, Start menu or Search showing up brings Sofa back (the guard);
//! - a full-screen app that was in front closing brings Sofa back;
//! - Steam grabbing the screen right after the PS button is pushed back down;
//! - the PS button remembers the app it came from, for the Running card.

use std::cell::RefCell;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PWSTR;

thread_local! {
    /// Foreground changes reported by the hook, handled by the main loop.
    static FOREGROUND: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    FOREGROUND.with(|f| f.borrow_mut().push(hwnd.0 as isize));
}

/// Asks Windows to tell Sofa whenever another window comes to the front.
pub fn watch_foreground() {
    // SAFETY: an out-of-context hook delivered on this thread's message loop.
    unsafe {
        let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
    }
}

pub fn take_foreground_events() -> Vec<isize> {
    FOREGROUND.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// A window and the program behind it.
#[derive(Clone, Debug)]
pub struct Window {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

pub fn class_name(h: isize) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetClassNameW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn title(h: isize) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: the buffer is passed with its length.
    let n = unsafe { GetWindowTextW(hwnd(h), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn pid(h: isize) -> u32 {
    let mut pid = 0u32;
    // SAFETY: writes into a local.
    unsafe { GetWindowThreadProcessId(hwnd(h), Some(&mut pid)) };
    pid
}

/// The full path of a process's program (empty when Windows does not say).
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

fn exe_name(h: isize) -> String {
    let e = exe_of(pid(h)).to_lowercase();
    e.rsplit('\\').next().unwrap_or("").to_owned()
}

pub fn window(h: isize) -> Window {
    Window { hwnd: h, title: title(h).replace('\t', " "), exe: exe_of(pid(h)) }
}

/// The desktop, taskbar, Start menu and Windows Search: the parts of Windows that Sofa stands
/// in for while it runs.
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

pub fn is_steam(h: isize) -> bool {
    matches!(exe_name(h).as_str(), "steam.exe" | "steamwebhelper.exe")
}

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

pub fn alive(h: isize) -> bool {
    // SAFETY: plain queries.
    unsafe { IsWindow(Some(hwnd(h))).as_bool() && IsWindowVisible(hwnd(h)).as_bool() }
}

/// Asks a window to close, as its own close button would.
pub fn close(h: isize, sofa: isize) {
    if h != 0 && h != sofa && alive(h) {
        // SAFETY: posting WM_CLOSE to another window.
        unsafe {
            let _ = PostMessageW(Some(hwnd(h)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Ends the program behind a window that stopped responding. Never Windows' shared hosts.
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

/// What the main loop should do after a foreground change or a tick.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Bring Sofa back to the front (the desktop showed, or a game closed).
    BringBack,
    /// Steam took the screen; Sofa pushed it back. The UI offers the one-time fix.
    SteamGrab,
    /// A window Sofa tracked as running has closed.
    Ended(isize),
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
        }
    }

    /// Topmost while Sofa is the home screen, so nothing (Steam included) opens above it.
    pub fn set_top(&mut self, on: bool) {
        let on = on && self.guard;
        if on == self.top {
            return;
        }
        self.top = on;
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
        // SAFETY: changing our own window's z-order.
        unsafe {
            let _ = SetWindowPos(hwnd(self.sofa), Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, flags);
            // Leaving topmost puts Sofa above every other window; when something else is
            // in front already, Sofa goes right behind it instead of covering it.
            let fg = GetForegroundWindow();
            if !on && !fg.is_invalid() && fg.0 as isize != self.sofa {
                let _ = SetWindowPos(hwnd(self.sofa), Some(fg), 0, 0, 0, 0, flags);
            }
        }
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
        } else {
            self.blur_at = Some(now);
        }
    }

    /// The PS button. Returns the app that was in front (not Windows' shell, not Sofa).
    pub fn home_pressed(&mut self, now: f64) -> Option<Window> {
        // SAFETY: plain query.
        let mut fg = unsafe { GetForegroundWindow() }.0 as isize;
        // Steam may have answered the button first: the app before it is the one that ran.
        if is_steam(fg) {
            fg = self.last_app.map_or(0, |(h, _)| h);
        }
        self.home_at = now;
        self.came_from = fg;
        self.desktop = false;
        self.last_app = None;
        (fg != 0 && fg != self.sofa && !is_shell(fg) && alive(fg)).then(|| window(fg))
    }

    pub fn on_foreground(&mut self, fg: isize, now: f64) -> Vec<Event> {
        let mut out = Vec::new();
        let prev = std::mem::replace(&mut self.prev, fg);
        if fg == self.sofa {
            self.shell_since = None;
            return out;
        }
        // Steam taking the screen from Sofa, or right after the PS button, with nothing
        // launched just now, is Steam answering the PS button: send it back under Sofa.
        let after_home = now - self.home_at < 10_000.0 && fg != self.came_from;
        if (prev == self.sofa || after_home) && now > self.allow_until && is_steam(fg) {
            // SAFETY: minimizing Steam's window.
            unsafe {
                let _ = ShowWindow(hwnd(fg), SW_MINIMIZE);
            }
            crate::platform::focus(hwnd(self.sofa));
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
        // Nothing in front at all (the app that was closed, or Sofa started in the background):
        // Windows sends no event for that, so check here.
        // SAFETY: plain query.
        if !focused && self.guard && !self.desktop && self.shell_since.is_none() && now >= self.idle_retry && unsafe { GetForegroundWindow() }.is_invalid() {
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
            // SAFETY: plain query.
            let fg = unsafe { GetForegroundWindow() }.0 as isize;
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
        self.shell_since.is_some() || self.blur_at.is_some() || self.session.is_some()
    }
}
