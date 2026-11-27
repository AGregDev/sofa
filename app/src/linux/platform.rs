//! Linux integration, with the same API as the Windows `platform.rs`: bringing a window to the
//! front (EWMH), launching games and apps, browsers with Sofa's own (TV) profile (ADR-004),
//! power through systemd, the clipboard, and giving memory back.

use crate::linux::{desktop, proc, x11};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The window class Sofa gives the browsers it starts, so it knows their windows.
pub const WEB_CLASS: &str = "SofaWeb";

/// Brings a window to the front from the background (the PS button while a game runs).
pub fn focus_id(h: isize) {
    if h != 0 {
        x11::activate(h);
    }
}

/// Starts a program away from Sofa: its own process group, no terminal, reaped when it ends.
pub fn spawn_detached(mut c: Command) -> bool {
    use std::os::unix::process::CommandExt;
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    // Sofa's own environment (an AppImage's mount, a QA folder) stays with Sofa.
    for k in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
        c.env_remove(k);
    }
    match c.spawn() {
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("reap".into()).stack_size(64 * 1024).spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// Opens a URL or a scheme (`steam://`, `spotify:`) with the desktop's handler. Windows'
/// `ms-settings:` pages open the desktop's own settings.
pub fn open(target: &str) -> bool {
    if let Some(page) = target.strip_prefix("ms-settings:") {
        return open_settings(page);
    }
    let mut c = Command::new("xdg-open");
    c.arg(target);
    spawn_detached(c)
}

/// The desktop's settings for sound, the display or Bluetooth.
fn open_settings(page: &str) -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let page = if page.is_empty() { "sound" } else { page };
    let tries: Vec<(&str, Vec<&str>)> = if desktop.contains("kde") {
        let kcm = match page {
            "display" => "kcm_kscreen",
            "bluetooth" => "kcm_bluetooth",
            _ => "kcm_pulseaudio",
        };
        vec![("systemsettings", vec![kcm]), ("kcmshell6", vec![kcm]), ("kcmshell5", vec![kcm])]
    } else {
        let gnome = match page {
            "display" => "display",
            "bluetooth" => "bluetooth",
            _ => "sound",
        };
        let other = match page {
            "display" => ("xfce4-display-settings", vec![]),
            "bluetooth" => ("blueman-manager", vec![]),
            _ => ("pavucontrol", vec![]),
        };
        vec![("gnome-control-center", vec![gnome]), other]
    };
    tries.into_iter().any(|(prog, args)| {
        desktop::which(prog).is_some() && {
            let mut c = Command::new(prog);
            c.args(args);
            spawn_detached(c)
        }
    })
}

pub fn run(exe: &Path, args: &[String]) -> bool {
    let mut c = Command::new(exe);
    c.args(args);
    if let Some(dir) = exe.parent().filter(|d| d.is_dir()) {
        c.current_dir(dir);
    }
    spawn_detached(c)
}

/// Starts a desktop entry (the Start menu's stand-in: its ID is the entry's file).
pub fn apps_folder(app_id: &str) -> bool {
    desktop::launch(Path::new(app_id))
}

// ---------- Browsers ----------

pub struct Browser {
    pub key: &'static str,
    pub name: &'static str,
    pub exe: PathBuf,
}

/// A program in PATH, or a Flatpak's exported launcher.
fn find(names: &[&str], flatpak: &str) -> Option<PathBuf> {
    names.iter().find_map(|n| desktop::which(n)).or_else(|| {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        [home.join(".local/share/flatpak/exports/bin").join(flatpak), PathBuf::from("/var/lib/flatpak/exports/bin").join(flatpak)].into_iter().find(|p| p.is_file())
    })
}

pub fn browsers() -> Vec<Browser> {
    let mut out = Vec::new();
    // Chromium browsers first, as on Windows: YouTube's TV interface is built for Chromium.
    if let Some(e) = find(&["microsoft-edge-stable", "microsoft-edge"], "com.microsoft.Edge") {
        out.push(Browser { key: "edge", name: "Edge", exe: e });
    }
    if let Some(e) = find(&["google-chrome-stable", "google-chrome"], "com.google.Chrome") {
        out.push(Browser { key: "chrome", name: "Chrome", exe: e });
    }
    if let Some(e) = find(&["chromium", "chromium-browser"], "org.chromium.Chromium") {
        out.push(Browser { key: "chromium", name: "Chromium", exe: e });
    }
    if let Some(e) = find(&["firefox", "firefox-esr"], "org.mozilla.firefox") {
        out.push(Browser { key: "firefox", name: "Firefox", exe: e });
    }
    out
}

/// The browser Sofa uses: the one picked in settings, else Edge, Chrome, Chromium, then Firefox.
pub fn chosen_browser(pick: &str) -> Option<Browser> {
    let mut all = browsers();
    let i = all.iter().position(|b| b.key == pick).unwrap_or(0);
    (!all.is_empty()).then(|| all.swap_remove(i))
}

const TV_UA: &str = "Mozilla/5.0 (SMART-TV; LINUX; Tizen 6.5) AppleWebKit/537.36 (KHTML, like Gecko) 85.0.4183.93/6.5 TV Safari/537.36";

/// The page zoom for the screen: TV interfaces get 1080 CSS pixels of height, other sites 720.
fn web_scale(tv: bool) -> f32 {
    let h = x11::screen_size().1.max(720) as f32;
    let s = h / if tv { 1080.0 } else { 720.0 };
    (s.clamp(1.0, 4.0) * 1000.0).round() / 1000.0
}

/// Where a browser may keep Sofa's profile: a Snap or a Flatpak cannot open Sofa's own folder
/// (hidden folders in the home folder are outside their sandbox), so theirs is in their own.
fn profile_dir(data: &Path, exe: &Path, name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let real = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let file = exe.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    if exe.starts_with("/snap/bin") || real.starts_with("/snap") {
        return home.join("snap").join(&file).join("common").join(format!("sofa-{name}"));
    }
    if exe.to_string_lossy().contains("/flatpak/exports/bin/") {
        return home.join(".var/app").join(&file).join("data").join(format!("sofa-{name}"));
    }
    data.join(name)
}

/// Firefox profiles for Sofa: full screen kiosk, no first-run pages, DRM on; the TV profile
/// also reports a TV browser so YouTube shows its TV interface.
fn firefox_profile(dir: PathBuf, tv: bool) -> PathBuf {
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
/// YouTube), on X11 so Sofa can see its window and be its remote. Returns whether it started,
/// and the browser's program when Sofa started one itself (the launch screen covers the screen
/// until that browser's window is ready).
pub fn open_web(data: &Path, pick: &str, url: Option<&str>, tv: bool) -> (bool, Option<PathBuf>) {
    let Some(b) = chosen_browser(pick) else { return (open(url.unwrap_or("https://duckduckgo.com")), None) };
    let mut c = Command::new(&b.exe);
    if b.key == "firefox" {
        let dir = firefox_profile(profile_dir(data, &b.exe, if tv { "firefox-tv" } else { "firefox" }), tv);
        c.env("MOZ_ENABLE_WAYLAND", "0").arg("--class").arg(WEB_CLASS).arg("-profile").arg(dir).args(["-no-remote", "--kiosk", url.unwrap_or("about:home")]);
    } else {
        match url {
            // Chrome's kiosk mode is full screen from its first frame; Edge's kiosk mode is
            // always InPrivate, so Edge gets a full-screen app window instead.
            Some(u) if b.key != "edge" => c.args(["--kiosk", u]),
            _ => c.arg(format!("--app={}", url.unwrap_or("https://www.google.com"))).arg("--start-fullscreen"),
        };
        c.arg("--window-position=0,0")
            .arg(format!("--user-data-dir={}", profile_dir(data, &b.exe, if tv { "browser-tv" } else { "browser" }).display()))
            .arg(format!("--force-device-scale-factor={}", web_scale(tv)))
            .arg(format!("--class={WEB_CLASS}"))
            .args(["--ozone-platform=x11", "--password-store=basic", "--no-first-run", "--no-default-browser-check", "--hide-crash-restore-bubble", "--autoplay-policy=no-user-gesture-required"])
            // Keep drawing while Sofa's launch screen covers the window.
            .arg("--disable-backgrounding-occluded-windows");
        if tv {
            c.arg(format!("--user-agent={TV_UA}"));
        }
    }
    let ok = spawn_detached(c);
    (ok, ok.then_some(b.exe))
}

// ---------- Processes, clipboard, memory ----------

/// Whether a program with this name (`steam`; `steam.exe` reads the same) is running.
pub fn process_running(name: &str) -> bool {
    proc::running(name)
}

pub fn steam_running() -> bool {
    process_running("steam")
}

/// The clipboard's text (empty when it holds none).
pub fn clipboard_text() -> String {
    x11::clipboard_text().unwrap_or_default()
}

/// Hands memory Sofa freed back to the system (while it waits behind a game).
pub fn trim_working_set() {
    // SAFETY: plain call; glibc returns free heap pages to the kernel.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// The controller is in use: desktops do not count pad input as activity, so the screen would
/// blank while someone browses Sofa. Tells the screen saver the person is there, at most every
/// 20 s, from a thread of its own (a D-Bus call).
pub fn user_active() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if now.saturating_sub(LAST.load(Ordering::Relaxed)) < 20 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let _ = std::thread::Builder::new().name("active".into()).spawn(|| {
        let Ok(bus) = zbus::blocking::Connection::session() else { return };
        for (dest, path, iface) in [("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver", "org.gnome.ScreenSaver"), ("org.freedesktop.ScreenSaver", "/org/freedesktop/ScreenSaver", "org.freedesktop.ScreenSaver")] {
            let _ = bus.call_method(Some(dest), path, Some(iface), "SimulateUserActivity", &());
        }
    });
}

// ---------- Power ----------

fn systemctl(what: &str) {
    let mut c = Command::new("systemctl");
    c.arg(what);
    if !spawn_detached(c) {
        let mut c = Command::new("loginctl");
        c.arg(what);
        spawn_detached(c);
    }
}

pub fn sleep() {
    systemctl("suspend");
}

pub fn restart(shutdown: bool) {
    systemctl(if shutdown { "poweroff" } else { "reboot" });
}
