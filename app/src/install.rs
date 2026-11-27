//! Installing, starting with Windows and uninstalling. Sofa is one executable: running
//! `Sofa-Setup-<version>.exe` copies it to `%LOCALAPPDATA%\Programs\Sofa\Sofa.exe`, adds a
//! Start menu shortcut and an entry in Installed apps, replaces Sofa 1.x if it is there (its
//! settings in `%APPDATA%\sofa` stay), and starts it. `Sofa.exe --uninstall` undoes it all
//! except the settings.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::System::Com::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::*;

const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sofa";
const NO_WINDOW: u32 = 0x0800_0000;

pub fn install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Programs").join("Sofa")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("Sofa.exe")
}

fn this_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

/// Started as the setup file (its name contains "setup"), not as the installed app.
pub fn is_setup() -> bool {
    this_exe().file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("setup"))
}

fn set_string(key: HKEY, name: &str, value: &str) {
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the byte slice covers the whole NUL-terminated string.
    unsafe {
        let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
        let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(bytes));
    }
}

fn set_dword(key: HKEY, name: &str, value: u32) {
    // SAFETY: four bytes for a DWORD.
    unsafe {
        let _ = RegSetValueExW(key, &HSTRING::from(name), None, REG_DWORD, Some(&value.to_le_bytes()));
    }
}

fn open_key(path: &str, write: bool) -> Option<HKEY> {
    let mut key = HKEY::default();
    // SAFETY: creates or opens a key under the current user; the caller closes it.
    let r = unsafe {
        if write {
            RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE | KEY_READ, None, &mut key, None)
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut key)
        }
    };
    r.is_ok().then_some(key)
}

fn close(key: HKEY) {
    // SAFETY: closing a key we opened.
    unsafe {
        let _ = RegCloseKey(key);
    }
}

const GAME_BAR: &str = "Software\\Microsoft\\GameBar";

/// Whether the Xbox button opens Game Bar: Windows' "Open Xbox Game Bar using this button on a
/// controller" (`UseNexusForGameBarEnabled`, on when it was never set).
pub fn game_bar_button() -> bool {
    let Some(key) = open_key(GAME_BAR, false) else { return true };
    let (mut value, mut size) = (1u32, 4u32);
    // SAFETY: reads a DWORD into a local of that size.
    let ok = unsafe { RegQueryValueExW(key, w!("UseNexusForGameBarEnabled"), None, None, Some((&mut value as *mut u32).cast()), Some(&mut size)) }.is_ok();
    close(key);
    !ok || value != 0
}

/// Lets the Xbox button open Game Bar, or keeps it for Sofa.
pub fn set_game_bar_button(on: bool) {
    let Some(key) = open_key(GAME_BAR, true) else { return };
    set_dword(key, "UseNexusForGameBarEnabled", u32::from(on));
    close(key);
}

/// Start Sofa with Windows (the Run key under the current user).
pub fn set_autostart(on: bool) {
    let Some(key) = open_key(RUN, true) else { return };
    if on {
        let exe = if installed_exe().exists() { installed_exe() } else { this_exe() };
        set_string(key, "Sofa", &format!("\"{}\"", exe.display()));
    } else {
        // SAFETY: deleting our own value.
        unsafe {
            let _ = RegDeleteValueW(key, w!("Sofa"));
        }
    }
    // Sofa 1.x registered itself under Electron's name.
    // SAFETY: deleting the old app's value; absent is fine.
    unsafe {
        let _ = RegDeleteValueW(key, w!("electron.app.Sofa"));
    }
    close(key);
}

pub fn autostart() -> bool {
    let Some(key) = open_key(RUN, false) else { return false };
    let mut size = 0u32;
    // SAFETY: a size query.
    let ok = unsafe { RegQueryValueExW(key, w!("Sofa"), None, None, None, Some(&mut size)) }.is_ok();
    close(key);
    ok
}

fn shortcut(target: &Path, link: &Path) -> Result<()> {
    // SAFETY: COM shell link creation; objects released on drop.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        sl.SetPath(&HSTRING::from(target.as_os_str()))?;
        sl.SetDescription(w!("Sofa: your gaming PC as a console"))?;
        sl.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
        if let Some(dir) = target.parent() {
            sl.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        }
        let file: IPersistFile = sl.cast()?;
        file.Save(&HSTRING::from(link.as_os_str()), true)
    }
}

fn start_menu_link() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join(r"Microsoft\Windows\Start Menu\Programs\Sofa.lnk")
}

/// Removes Sofa 1.x (Electron) if it is installed in the same folder: stops it, then runs its
/// own uninstaller silently. Its settings (`%APPDATA%\sofa`) are kept and reused.
fn remove_sofa_1() {
    let dir = install_dir();
    let old_uninstaller = dir.join("Uninstall Sofa.exe");
    if !dir.join("resources").exists() || !old_uninstaller.exists() {
        return;
    }
    let me = std::process::id();
    // Stop 1.x and its PowerShell helper (both live under the old install folder).
    let _ = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne {me} -and (($_.ExecutablePath -like '{0}\\*') -or ($_.CommandLine -like '*{0}\\resources\\scripts\\padwatch.ps1*')) }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force }}",
                dir.display()
            ),
        ])
        .creation_flags(NO_WINDOW)
        .status();
    let _ = Command::new(&old_uninstaller).args(["/S", "/currentuser"]).creation_flags(NO_WINDOW).status();
    // The NSIS uninstaller copies itself away and keeps working after it returns; it removes
    // its own file last, then the folder.
    for _ in 0..90 {
        if !dir.join("resources").exists() && !old_uninstaller.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    std::thread::sleep(std::time::Duration::from_millis(1500));
}

/// Installs this executable and returns the installed copy's path. `autostart` is the
/// person's "Start Sofa with Windows" (on for a first install).
pub fn install(version: &str, autostart: bool) -> std::result::Result<PathBuf, String> {
    remove_sofa_1();
    let dir = install_dir();
    let target = installed_exe();
    // A running Sofa 2 holds its file: stop it first.
    let _ = Command::new("taskkill").args(["/IM", "Sofa.exe", "/F"]).creation_flags(NO_WINDOW).status();
    std::thread::sleep(std::time::Duration::from_millis(400));
    // The old uninstaller may still be removing its folder: try a few times.
    let mut copied = Err(String::new());
    for _ in 0..10 {
        copied = std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(this_exe(), &target)).map(|_| ()).map_err(|e| format!("Couldn't copy Sofa: {e}"));
        if copied.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
    copied?;
    let _ = shortcut(&target, &start_menu_link());
    if let Some(key) = open_key(UNINSTALL, true) {
        set_string(key, "DisplayName", "Sofa");
        set_string(key, "DisplayVersion", version);
        set_string(key, "Publisher", "AGregDev");
        set_string(key, "DisplayIcon", &target.display().to_string());
        set_string(key, "InstallLocation", &dir.display().to_string());
        set_string(key, "UninstallString", &format!("\"{}\" --uninstall", target.display()));
        set_string(key, "URLInfoAbout", "https://github.com/AGregDev/sofa");
        let size = std::fs::metadata(&target).map(|m| (m.len() / 1024) as u32).unwrap_or(0);
        set_dword(key, "EstimatedSize", size);
        set_dword(key, "NoModify", 1);
        set_dword(key, "NoRepair", 1);
        close(key);
    }
    set_autostart(autostart);
    Ok(target)
}

/// Removes Sofa: shortcut, start with Windows, Installed apps entry and the program folder.
/// Settings in `%APPDATA%\sofa` stay, so a reinstall picks up where it left off.
pub fn uninstall() {
    // A running Sofa holds its file: stop it (not this process).
    let _ = Command::new("taskkill").args(["/F", "/IM", "Sofa.exe", "/FI", &format!("PID ne {}", std::process::id())]).creation_flags(NO_WINDOW).status();
    set_autostart(false);
    let _ = std::fs::remove_file(start_menu_link());
    // SAFETY: deleting our own key.
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL));
    }
    // The running file cannot delete itself: a detached shell finishes after it exits.
    let dir = install_dir();
    // cmd.exe does not understand the `\"` quoting Rust gives arguments, so the line goes in
    // as written. `timeout` needs a console; ping waits without one.
    let _ = Command::new("cmd.exe")
        .raw_arg(format!("/C ping -n 4 127.0.0.1 >nul & rmdir /s /q \"{}\"", dir.display()))
        .creation_flags(NO_WINDOW | 0x0000_0008)
        .spawn();
}
