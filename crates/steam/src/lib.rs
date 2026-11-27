//! Keeps Steam off the PS button. Steam brings itself forward on the PS (guide) button while
//! its "Guide button focuses Steam" setting is on. Steam keeps that setting per account in
//! `userdata/<id>/config/localconfig.vdf` as `Controller_CheckGuideButton` (steamui.dll maps
//! `controller_guide_button_focus_steam` to that key; absent means on).
//!
//! Steam rewrites localconfig.vdf from memory when it exits, so the file is only changed while
//! Steam is closed. A running Steam is shut down first and started again after, and only when
//! the caller says so (the person agreed). Ported from Sofa 1.x's `src/steamguide.js`.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const KEY: &str = "Controller_CheckGuideButton";
static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\t"Controller_CheckGuideButton"\t\t"([0-9]+)""#).expect("valid pattern"));

/// Every account's localconfig.vdf under a Steam folder.
pub fn config_files(steam_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(steam_root.join("userdata")) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
        .map(|e| e.path().join("config").join("localconfig.vdf"))
        .filter(|f| f.exists())
        .collect();
    files.sort();
    files
}

/// "0" (off), "1" (on) or None (never changed: Steam's default, on).
pub fn read_value(text: &str) -> Option<String> {
    LINE.captures(text).map(|c| c[1].to_owned())
}

fn root_store(text: &str) -> serde_json::Map<String, serde_json::Value> {
    let parsed = vdf::parse(text);
    let root = parsed.as_object().cloned().unwrap_or_default();
    let store = root.get("UserLocalConfigStore").or_else(|| root.values().next()).cloned();
    store.and_then(|s| s.as_object().cloned()).unwrap_or_default()
}

/// The file's text with the setting turned off, checked by parsing it back: the key reads "0"
/// and every other top-level section is still there.
pub fn with_guide_off(text: &str) -> Result<String, String> {
    let out = if LINE.is_match(text) {
        LINE.replace(text, format!("\t\"{KEY}\"\t\t\"0\"")).into_owned()
    } else {
        // A new top-level key goes just before the closing brace of the root block.
        let end = text.rfind('}').ok_or("localconfig.vdf has no root block")?;
        let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let before = text[..end].trim_end_matches([' ', '\t']);
        format!("{before}\t\"{KEY}\"\t\t\"0\"{nl}{}", &text[end..])
    };
    let store = root_store(&out);
    if store.get(KEY).and_then(|v| v.as_str()) != Some("0") {
        return Err("Could not verify the change".into());
    }
    for k in root_store(text).keys() {
        if k != KEY && !store.contains_key(k) {
            return Err(format!("Change would drop {k}"));
        }
    }
    Ok(out)
}

/// Turns the setting off in one file. Keeps the original as `.sofa-backup` (once), writes
/// through a temp file. Returns whether anything changed.
pub fn set_off(file: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    if read_value(&text).as_deref() == Some("0") {
        return Ok(false);
    }
    let out = with_guide_off(&text)?;
    let backup = file.with_extension("vdf.sofa-backup");
    if !backup.exists() {
        std::fs::copy(file, &backup).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("vdf.sofa-tmp");
    std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())?;
    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub installed: bool,
    pub accounts: usize,
    /// Every account has the setting off.
    pub off: bool,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    Changed { restarted: bool },
    AlreadyOff,
    NoSteam,
    NoAccount,
    /// Steam is open and the caller did not allow a restart.
    Running,
    StillRunning,
    Write(String),
}

/// Whether steam.exe is running. Tests pass their own check.
pub type IsRunning<'a> = &'a dyn Fn() -> bool;

pub fn status(steam_root: &Path, is_running: IsRunning) -> Status {
    if !steam_root.join("steam.exe").exists() {
        return Status { installed: false, accounts: 0, off: false, running: false };
    }
    let files = config_files(steam_root);
    let off = !files.is_empty() && files.iter().all(|f| std::fs::read_to_string(f).ok().and_then(|t| read_value(&t)).as_deref() == Some("0"));
    Status { installed: true, accounts: files.len(), off, running: is_running() }
}

/// Turns the setting off for every account. With `restart`, a running Steam is shut down
/// (`steam.exe -shutdown`), changed, and started again in the background (`-silent`).
pub fn fix(steam_root: &Path, restart: bool, is_running: IsRunning) -> Fix {
    let s = status(steam_root, is_running);
    if !s.installed {
        return Fix::NoSteam;
    }
    if s.accounts == 0 {
        return Fix::NoAccount;
    }
    if s.off {
        return Fix::AlreadyOff;
    }
    let exe = steam_root.join("steam.exe");
    if s.running {
        if !restart {
            return Fix::Running;
        }
        let _ = Command::new(&exe).arg("-shutdown").spawn();
        let until = Instant::now() + Duration::from_secs(45);
        while is_running() {
            if Instant::now() > until {
                return Fix::StillRunning;
            }
            std::thread::sleep(Duration::from_millis(700));
        }
        std::thread::sleep(Duration::from_millis(800)); // let Steam finish writing its files
    }
    for f in config_files(steam_root) {
        if let Err(e) = set_off(&f) {
            return Fix::Write(e);
        }
    }
    if s.running {
        let _ = Command::new(&exe).arg("-silent").spawn();
    }
    Fix::Changed { restarted: s.running }
}
