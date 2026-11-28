//! Installing, starting with the session and uninstalling, on Linux. Sofa is one file: run as
//! `Sofa-Setup-<version>-x86_64.AppImage` (any name with "setup" in it), or as any build with
//! `--install`, it copies itself to `~/.local/bin/sofa`, adds its desktop entry (with an
//! Uninstall action) and its icon, and starts the installed copy. Start with your session is an
//! entry in `~/.config/autostart`. `sofa --uninstall` undoes it all except the settings in
//! `~/.config/sofa`. Nothing needs root.

use crate::linux::{config_home, data_home, home, proc};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub fn install_dir() -> PathBuf {
    home().join(".local/bin")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("sofa")
}

/// The file this Sofa runs from: the AppImage itself when it is one.
fn this_file() -> PathBuf {
    std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()).unwrap_or_else(|| std::env::current_exe().unwrap_or_default())
}

/// Started as the setup file (its name contains "setup") or with `--install`, and not already
/// the installed copy.
pub fn is_setup() -> bool {
    let me = this_file();
    let named = me.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("setup"));
    let asked = std::env::args().any(|a| a == "--install");
    (named || asked) && std::fs::canonicalize(&me).ok() != std::fs::canonicalize(installed_exe()).ok()
}

fn entry_path() -> PathBuf {
    data_home().join("applications/sofa.desktop")
}

fn autostart_path() -> PathBuf {
    config_home().join("autostart/sofa.desktop")
}

fn icon_path() -> PathBuf {
    data_home().join("icons/hicolor/512x512/apps/sofa.png")
}

/// The desktop entry for `exe` (with the Uninstall action when `actions`).
fn entry(exe: &std::path::Path, actions: bool) -> String {
    let exe = exe.display().to_string().replace('"', "\\\"");
    let mut e = format!(
        "[Desktop Entry]\nType=Application\nName=Sofa\nGenericName=Console home screen\nComment=Your gaming PC as a console, from the couch\nExec=\"{exe}\"\nIcon=sofa\nTerminal=false\nCategories=Game;\nStartupWMClass=Sofa\nX-GNOME-Autostart-enabled=true\n"
    );
    if actions {
        e.push_str(&format!("Actions=uninstall;\n\n[Desktop Action uninstall]\nName=Uninstall Sofa\nExec=\"{exe}\" --uninstall\n"));
    }
    e
}

fn write(path: &PathBuf, text: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// Stops other Sofas (not this one): the installed copy, or an AppImage of Sofa.
fn stop_others() {
    let me = std::process::id();
    let installed = installed_exe();
    let mut stopped = Vec::new();
    for p in proc::pids() {
        if p == me || proc::comm(p) != "sofa" {
            continue;
        }
        let exe = PathBuf::from(proc::exe(p));
        if exe == installed || exe.to_string_lossy().contains("/.mount_") || proc::env(p, "APPIMAGE").is_some_and(|a| a.to_lowercase().contains("sofa")) {
            proc::signal(p, false);
            stopped.push(p);
        }
    }
    // Up to 3 s for them to save and go.
    for _ in 0..30 {
        if stopped.iter().all(|p| !std::path::Path::new(&format!("/proc/{p}")).exists()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Installs this file and returns the installed copy's path. `autostart` is the person's
/// "Start Sofa when you sign in" (on for a first install).
pub fn install(_version: &str, autostart: bool) -> Result<PathBuf, String> {
    stop_others();
    let target = installed_exe();
    std::fs::create_dir_all(install_dir()).map_err(|e| format!("Couldn't make {}: {e}", install_dir().display()))?;
    // Copy next to it, then rename over it: a running copy keeps its old file.
    let tmp = target.with_extension("new");
    std::fs::copy(this_file(), &tmp).map_err(|e| format!("Couldn't copy Sofa: {e}"))?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("Couldn't copy Sofa: {e}"))?;
    write(&icon_path(), crate::ICON_PNG)?;
    write(&entry_path(), entry(&target, true).as_bytes())?;
    set_autostart(autostart);
    // Desktops that cache the menu pick the entry up now.
    let _ = std::process::Command::new("update-desktop-database").arg(data_home().join("applications")).output();
    Ok(target)
}

/// Removes Sofa: its program, desktop entry, icon and start with the session. The settings in
/// `~/.config/sofa` stay, so a reinstall picks up where it left off.
pub fn uninstall() {
    stop_others();
    set_autostart(false);
    let _ = std::fs::remove_file(entry_path());
    let _ = std::fs::remove_file(icon_path());
    let _ = std::fs::remove_file(installed_exe());
    let _ = std::process::Command::new("update-desktop-database").arg(data_home().join("applications")).output();
}

/// Start Sofa with the session (an autostart entry).
pub fn set_autostart(on: bool) {
    let path = autostart_path();
    if on {
        let exe = if installed_exe().exists() { installed_exe() } else { this_file() };
        let _ = write(&path, entry(&exe, false).as_bytes());
    } else {
        let _ = std::fs::remove_file(path);
    }
}

pub fn autostart() -> bool {
    autostart_path().exists()
}

/// Windows' Game Bar has no Linux counterpart: the Xbox button is Sofa's.
pub fn game_bar_button() -> bool {
    false
}

pub fn set_game_bar_button(_on: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_runs_the_installed_copy_and_offers_uninstall() {
        let e = entry(std::path::Path::new("/home/me/.local/bin/sofa"), true);
        assert!(e.contains("Exec=\"/home/me/.local/bin/sofa\"\n"));
        assert!(e.contains("StartupWMClass=Sofa"));
        assert!(e.contains("Exec=\"/home/me/.local/bin/sofa\" --uninstall"));
        assert!(!entry(std::path::Path::new("/x/sofa"), false).contains("Actions"));
    }
}
