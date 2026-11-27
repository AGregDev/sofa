//! Windows integration from `main.js` and `scripts/padwatch.ps1`: bringing Sofa to the front,
//! launching games and apps, browsers (with the TV profile YouTube uses, ADR-004), and power.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::*;
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_MENU, keybd_event};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const DETACHED: u32 = 0x0000_0008 | 0x0000_0200; // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP

/// Marks the keys and clicks Sofa itself synthesizes (`dwExtraInfo`), so `hold_synthetic_input`
/// lets them through.
pub const SOFA_MARK: usize = 0x50FA_50FA;

// ---------- Holding back other programs' synthetic input ----------

static HOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static HOOK_THREAD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
const HOOK_ON: u32 = 0x8000 + 20;
const HOOK_OFF: u32 = 0x8000 + 21;

unsafe extern "system" fn key_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && HOLD.load(std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: for WH_KEYBOARD_LL, lparam points to a KBDLLHOOKSTRUCT.
        let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        if k.flags.contains(LLKHF_INJECTED) && k.dwExtraInfo != SOFA_MARK {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the event on.
    unsafe { CallNextHookEx(None, code, w, l) }
}

unsafe extern "system" fn mouse_hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 && HOLD.load(std::sync::atomic::Ordering::Relaxed) {
        // SAFETY: for WH_MOUSE_LL, lparam points to a MSLLHOOKSTRUCT.
        let m = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
        if m.flags & LLMHF_INJECTED != 0 && m.dwExtraInfo != SOFA_MARK {
            return LRESULT(1);
        }
    }
    // SAFETY: passing the event on.
    unsafe { CallNextHookEx(None, code, w, l) }
}

/// While a streaming window Sofa opened is in front, Sofa is the only thing turning the
/// controller into input: keys and clicks other programs synthesize (Steam's desktop layout
/// sends arrows, Space and clicks for the same buttons) are held back. The person's real
/// keyboard and mouse always pass. The hooks live on their own thread, installed only while
/// this is on.
pub fn hold_synthetic_input(on: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    if HOLD.swap(on, Relaxed) == on {
        return;
    }
    let id = match HOOK_THREAD.get() {
        Some(id) => *id,
        None => {
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("inputhold".into()).spawn(move || {
                let mut msg = MSG::default();
                // SAFETY: creates this thread's message queue before its id is handed out;
                // hooks are installed and removed on this thread only.
                unsafe {
                    let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                    let _ = tx.send(GetCurrentThreadId());
                    let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
                    let mut hooks: Vec<HHOOK> = Vec::new();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        match msg.message {
                            HOOK_ON if hooks.is_empty() => {
                                hooks.extend(SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), module, 0).ok());
                                hooks.extend(SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0).ok());
                            }
                            HOOK_OFF => {
                                for h in hooks.drain(..) {
                                    let _ = UnhookWindowsHookEx(h);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            });
            let Some(id) = spawned.ok().and_then(|_| rx.recv().ok()) else { return };
            *HOOK_THREAD.get_or_init(|| id)
        }
    };
    // SAFETY: posting to our own hook thread.
    unsafe {
        let _ = PostThreadMessageW(id, if on { HOOK_ON } else { HOOK_OFF }, WPARAM(0), LPARAM(0));
    }
}

/// Brings a window to the front from the background (the PS button while a game runs).
pub fn focus(h: HWND) {
    // SAFETY: Win32 calls on window handles; AttachThreadInput is undone before returning.
    unsafe {
        if !IsWindow(Some(h)).as_bool() {
            return;
        }
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let fg = GetForegroundWindow();
        if fg == h {
            let _ = BringWindowToTop(h);
            return;
        }
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        // Tapping Alt lets a background process take the foreground.
        keybd_event(VK_MENU.0 as u8, 0, Default::default(), SOFA_MARK);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, SOFA_MARK);
        let _ = AttachThreadInput(me, fg_thread, true);
        let _ = SetForegroundWindow(h);
        let _ = BringWindowToTop(h);
        let _ = AttachThreadInput(me, fg_thread, false);
        // A minimized window can be the foreground window and still not show: restore it
        // once more now that it may be activated.
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
    }
}

pub fn open(target: &str) -> bool {
    let t = HSTRING::from(target);
    // SAFETY: ShellExecute with valid strings.
    let r = unsafe { ShellExecuteW(None, w!("open"), &t, None, None, SW_SHOWNORMAL) };
    r.0 as isize > 32
}

pub fn run(exe: &Path, args: &[String]) -> bool {
    let mut c = Command::new(exe);
    c.args(args).creation_flags(DETACHED);
    if let Some(dir) = exe.parent() {
        c.current_dir(dir);
    }
    c.spawn().is_ok()
}

pub fn apps_folder(app_id: &str) -> bool {
    Command::new("explorer.exe").arg(format!("shell:AppsFolder\\{app_id}")).creation_flags(DETACHED).spawn().is_ok()
}

// ---------- Browsers ----------

pub struct Browser {
    pub key: &'static str,
    pub name: &'static str,
    pub exe: PathBuf,
}

pub fn browsers() -> Vec<Browser> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let (pf, pf86, local) = (env("ProgramFiles"), env("ProgramFiles(x86)"), env("LOCALAPPDATA"));
    let find = |cands: Vec<Option<PathBuf>>| cands.into_iter().flatten().find(|p| p.exists());
    let mut out = Vec::new();
    let firefox = find(vec![
        pf.as_ref().map(|p| p.join(r"Mozilla Firefox\firefox.exe")),
        pf86.as_ref().map(|p| p.join(r"Mozilla Firefox\firefox.exe")),
        local.as_ref().map(|p| p.join(r"Mozilla Firefox\firefox.exe")),
    ]);
    let chrome = find(vec![
        pf.as_ref().map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
        pf86.as_ref().map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
        local.as_ref().map(|p| p.join(r"Google\Chrome\Application\chrome.exe")),
    ]);
    let edge = find(vec![
        pf86.as_ref().map(|p| p.join(r"Microsoft\Edge\Application\msedge.exe")),
        pf.as_ref().map(|p| p.join(r"Microsoft\Edge\Application\msedge.exe")),
    ]);
    // Chromium browsers first: YouTube's TV interface is built for Chromium, Edge plays
    // Netflix and Disney+ above 720p (PlayReady), and a second Firefox profile next to the
    // person's own Firefox stops at "Firefox needs a quick restart" whenever an update waits.
    if let Some(e) = edge {
        out.push(Browser { key: "edge", name: "Edge", exe: e });
    }
    if let Some(e) = chrome {
        out.push(Browser { key: "chrome", name: "Chrome", exe: e });
    }
    if let Some(e) = firefox {
        out.push(Browser { key: "firefox", name: "Firefox", exe: e });
    }
    out
}

/// The browser Sofa uses: the one picked in settings, else Edge, Chrome, then Firefox.
pub fn chosen_browser(pick: &str) -> Option<Browser> {
    let mut all = browsers();
    let i = all.iter().position(|b| b.key == pick).unwrap_or(0);
    (!all.is_empty()).then(|| all.swap_remove(i))
}

const TV_UA: &str = "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) 85.0.4183.93/6.5 TV Safari/537.36";

/// The page zoom for the screen Sofa is on. TV interfaces get exactly 1080 CSS pixels of
/// height (YouTube's TV layout then draws at the screen's own resolution: sharp on 1440p and
/// 4K, not a smaller layout scaled up); other sites get 720, readable from the sofa.
fn web_scale(tv: bool) -> f32 {
    // SAFETY: plain query (Sofa is per-monitor DPI aware, so these are real pixels).
    let h = unsafe { GetSystemMetrics(SM_CYSCREEN) }.max(720) as f32;
    let s = h / if tv { 1080.0 } else { 720.0 };
    (s.clamp(1.0, 4.0) * 1000.0).round() / 1000.0
}

/// Firefox profiles in Sofa's folder: full screen kiosk, no first-run pages, DRM on; the TV
/// profile also reports a TV browser so YouTube shows its TV interface.
fn firefox_profile(data: &Path, tv: bool) -> PathBuf {
    let dir = data.join(if tv { "firefox-tv" } else { "firefox" });
    let _ = std::fs::create_dir_all(&dir);
    let mut prefs = vec![
        ("browser.aboutwelcome.enabled", "false"),
        ("browser.shell.checkDefaultBrowser", "false"),
        ("browser.startup.homepage_override.mstone", "\"ignore\""),
        ("datareporting.policy.dataSubmissionPolicyBypassNotification", "true"),
        ("toolkit.telemetry.reportingpolicy.firstRun", "false"),
        ("trailhead.firstrun.didSeeAboutWelcome", "true"),
        ("media.eme.enabled", "true"),
        ("media.gmp-widevinecdm.enabled", "true"),
        ("media.gmp-widevinecdm.visible", "true"),
        ("full-screen-api.warning.timeout", "0"),
        ("browser.tabs.warnOnClose", "false"),
        ("browser.sessionstore.resume_from_crash", "false"),
    ];
    let scale = format!("\"{}\"", web_scale(tv));
    prefs.push(("layout.css.devPixelsPerPx", &scale));
    let ua = format!("\"{TV_UA}\"");
    if tv {
        prefs.push(("general.useragent.override", &ua));
    }
    let lines: Vec<String> = prefs.iter().map(|(k, v)| format!("user_pref(\"{k}\", {v});")).collect();
    let _ = std::fs::write(dir.join("user.js"), lines.join("\n") + "\n");
    dir
}

/// Opens a site full screen in the chosen browser, with Sofa's own profile (a TV profile for
/// YouTube). Returns whether it started, and the browser's program when Sofa started one
/// itself (Sofa's launch screen covers the screen until that browser's window is ready).
pub fn open_web(data: &Path, pick: &str, url: Option<&str>, tv: bool) -> (bool, Option<PathBuf>) {
    let Some(b) = chosen_browser(pick) else { return (open(url.unwrap_or("https://duckduckgo.com")), None) };
    let mut c = Command::new(&b.exe);
    c.creation_flags(DETACHED);
    if b.key == "firefox" {
        c.arg("-profile").arg(firefox_profile(data, tv)).args(["-no-remote", "--kiosk", url.unwrap_or("about:home")]);
    } else {
        match url {
            // Chrome's kiosk mode is full screen from its first frame, without the "press and
            // hold Esc to exit full screen" bubble (an always-on-top popup that shows even over
            // Sofa's launch screen). Edge's kiosk mode is always InPrivate, so sign-ins would
            // not stay: Edge gets a full-screen app window instead.
            Some(u) if b.key == "chrome" => c.args(["--kiosk", u]),
            _ => c.arg(format!("--app={}", url.unwrap_or("https://www.google.com"))).arg("--start-fullscreen"),
        };
        c.arg("--window-position=0,0")
            .arg(format!("--user-data-dir={}", data.join(if tv { "browser-tv" } else { "browser" }).display()))
            .arg(format!("--force-device-scale-factor={}", web_scale(tv)))
            .args(["--no-first-run", "--no-default-browser-check", "--hide-crash-restore-bubble", "--autoplay-policy=no-user-gesture-required"])
            // Keep drawing while Sofa's launch screen covers the window, so it is ready
            // the moment Sofa steps aside (no white or empty frame).
            .args(["--disable-features=CalculateNativeWinOcclusion", "--disable-backgrounding-occluded-windows"]);
        if tv {
            c.arg(format!("--user-agent={TV_UA}"));
        }
    }
    let ok = c.spawn().is_ok();
    (ok, ok.then_some(b.exe))
}

// ---------- Processes, clipboard, memory ----------

/// Whether a program with this file name (`steam.exe`) is running.
pub fn process_running(name: &str) -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    // SAFETY: a process snapshot read into a local entry, closed before returning.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(name) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub fn steam_running() -> bool {
    process_running("steam.exe")
}

/// The clipboard's text (empty when it holds none).
pub fn clipboard_text() -> String {
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    let mut out = String::new();
    // SAFETY: the clipboard is opened and closed here; the locked text is NUL-terminated and
    // copied before the lock is released.
    unsafe {
        if OpenClipboard(None).is_err() {
            return out;
        }
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while n < 8192 && *p.add(n) != 0 {
                    n += 1;
                }
                out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
    }
    out
}

/// Hands Sofa's unused memory pages back to Windows (while it waits behind a game).
pub fn trim_working_set() {
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain call on our own process.
    unsafe {
        let _ = EmptyWorkingSet(GetCurrentProcess());
    }
}

/// The controller is in use: Windows does not count pad input as activity, so the display
/// would turn off (or the screen saver start) while someone browses Sofa. Resets the idle
/// timers, at most every 20 s.
pub fn user_active() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows::Win32::System::Power::{ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if now.saturating_sub(LAST.load(Ordering::Relaxed)) < 20 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let input = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: SOFA_MARK } } };
    // SAFETY: plain calls; the input array lives on the stack for the call. A zero-length
    // mouse move is the input the screen saver's idle timer listens to.
    unsafe {
        SetThreadExecutionState(ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

// ---------- Power ----------

pub fn sleep() {
    // SAFETY: plain call; Windows decides whether sleep is possible.
    unsafe {
        let _ = SetSuspendState(false, false, false);
    }
}

pub fn restart(shutdown: bool) {
    let _ = Command::new("shutdown.exe").args([if shutdown { "/s" } else { "/r" }, "/t", "0"]).creation_flags(0x0800_0000).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    fn press(up: bool, mark: usize) {
        let flags = if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) };
        let i = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_F24, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: mark } } };
        // SAFETY: one input on the stack for the call.
        unsafe { SendInput(&[i], std::mem::size_of::<INPUT>() as i32) };
        std::thread::sleep(Duration::from_millis(80));
    }

    fn f24_down() -> bool {
        // SAFETY: plain query.
        let state = unsafe { GetAsyncKeyState(i32::from(VK_F24.0)) };
        state < 0
    }

    /// Sends F24 (a key nothing uses) system-wide, so it runs only when asked:
    /// `cargo test --bin sofa -- --ignored`.
    #[test]
    #[ignore]
    fn held_input_stops_other_programs_keys_but_not_sofas() {
        hold_synthetic_input(true);
        std::thread::sleep(Duration::from_millis(300));
        press(false, 0);
        let others = f24_down();
        press(true, 0);
        press(false, SOFA_MARK);
        let sofas = f24_down();
        press(true, SOFA_MARK);
        hold_synthetic_input(false);
        std::thread::sleep(Duration::from_millis(300));
        press(false, 0);
        let released = f24_down();
        press(true, 0);
        assert!(!others, "another program's key got through while held");
        assert!(sofas, "Sofa's own key was held back");
        assert!(released, "keys stay held back after the hold ended");
    }
}
