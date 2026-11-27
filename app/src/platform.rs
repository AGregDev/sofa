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
        keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        let _ = AttachThreadInput(me, fg_thread, true);
        let _ = SetForegroundWindow(h);
        let _ = BringWindowToTop(h);
        let _ = AttachThreadInput(me, fg_thread, false);
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
    if let Some(e) = firefox {
        out.push(Browser { key: "firefox", name: "Firefox", exe: e });
    }
    if let Some(e) = chrome {
        out.push(Browser { key: "chrome", name: "Chrome", exe: e });
    }
    if let Some(e) = edge {
        out.push(Browser { key: "edge", name: "Edge", exe: e });
    }
    out
}

/// The browser Sofa uses: the one picked in settings, else Firefox, Chrome, then Edge.
pub fn chosen_browser(pick: &str) -> Option<Browser> {
    let mut all = browsers();
    let i = all.iter().position(|b| b.key == pick).unwrap_or(0);
    (!all.is_empty()).then(|| all.swap_remove(i))
}

const TV_UA: &str = "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) 85.0.4183.93/6.5 TV Safari/537.36";

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
        ("layout.css.devPixelsPerPx", "\"1.5\""),
    ];
    let ua = format!("\"{TV_UA}\"");
    if tv {
        prefs.push(("general.useragent.override", &ua));
    }
    let lines: Vec<String> = prefs.iter().map(|(k, v)| format!("user_pref(\"{k}\", {v});")).collect();
    let _ = std::fs::write(dir.join("user.js"), lines.join("\n") + "\n");
    dir
}

/// Opens a site full screen in the chosen browser, with Sofa's own TV profile.
pub fn open_web(data: &Path, pick: &str, url: Option<&str>, tv: bool) -> bool {
    let Some(b) = chosen_browser(pick) else { return open(url.unwrap_or("https://duckduckgo.com")) };
    let mut c = Command::new(&b.exe);
    c.creation_flags(DETACHED);
    if b.key == "firefox" {
        c.arg("-profile").arg(firefox_profile(data, tv)).args(["-no-remote", "--kiosk", url.unwrap_or("about:home")]);
    } else {
        c.arg(format!("--app={}", url.unwrap_or("https://www.google.com")))
            .arg("--start-fullscreen")
            .arg(format!("--user-data-dir={}", data.join(if tv { "browser-tv" } else { "browser" }).display()));
        if tv {
            c.arg(format!("--user-agent={TV_UA}"));
        }
    }
    c.spawn().is_ok()
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
    let input = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 } } };
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
