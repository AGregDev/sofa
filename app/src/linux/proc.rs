//! Processes, from `/proc`: a process's program (a Wine or Proton game's own `.exe` as a Linux
//! path, so it matches the game's Steam folder), when it started, whether a program runs, and
//! ending one.

use std::path::{Path, PathBuf};

/// Every process ID.
pub fn pids() -> Vec<u32> {
    std::fs::read_dir("/proc").map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()).unwrap_or_default()
}

/// The short name the kernel keeps for a process ("steam", "steamwebhelper").
pub fn comm(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm")).map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn nul_list(path: String) -> Vec<String> {
    std::fs::read(path).map(|b| b.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()).unwrap_or_default()
}

/// A variable from a process's environment (our own processes only).
pub fn env(pid: u32, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    nul_list(format!("/proc/{pid}/environ")).into_iter().find_map(|kv| kv.strip_prefix(&prefix).map(str::to_owned))
}

/// A Windows path in a Wine prefix as a Linux path: `Z:\home\…` is `/home/…`; other drives
/// through the prefix's `dosdevices`.
pub fn wine_path(win: &str, prefix: Option<&str>) -> Option<String> {
    let b = win.as_bytes();
    if b.len() < 3 || b[1] != b':' || !(b[2] == b'\\' || b[2] == b'/') || !b[0].is_ascii_alphabetic() {
        return None;
    }
    let rest = win[3..].replace('\\', "/");
    let drive = (b[0] as char).to_ascii_lowercase();
    if drive == 'z' {
        return Some(format!("/{rest}"));
    }
    let base = std::fs::canonicalize(Path::new(prefix?).join("dosdevices").join(format!("{drive}:"))).ok()?;
    Some(format!("{}/{rest}", base.display()))
}

/// A process's program. For Wine (Proton) the program is the game's `.exe`, from its command
/// line, as a Linux path.
pub fn exe(pid: u32) -> String {
    let real = std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let base = real.rsplit('/').next().unwrap_or("").to_lowercase();
    if base.starts_with("wine") || base.contains("preloader") {
        let args = nul_list(format!("/proc/{pid}/cmdline"));
        if let Some(first) = args.first() {
            let prefix = env(pid, "WINEPREFIX");
            if let Some(p) = wine_path(first, prefix.as_deref()) {
                return p;
            }
            if first.starts_with('/') && first.to_lowercase().ends_with(".exe") {
                return first.clone();
            }
        }
    }
    real
}

/// When a process started (epoch ms).
pub fn started(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name (field 2) may hold spaces: fields count from after its closing bracket.
    let after = &stat[stat.rfind(')')? + 2..];
    let ticks: f64 = after.split_whitespace().nth(19)?.parse().ok()?;
    let boot: f64 = std::fs::read_to_string("/proc/stat").ok()?.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
    // SAFETY: plain query.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    Some((boot + ticks / hz) * 1000.0)
}

/// Whether a program with this name runs (`steam`: also `steam.exe`, its Windows name).
pub fn running(name: &str) -> bool {
    let want = name.to_lowercase();
    let want = want.strip_suffix(".exe").unwrap_or(&want);
    let me = std::process::id();
    pids().into_iter().any(|p| p != me && comm(p).to_lowercase() == want)
}

/// Ends a process (`SIGKILL` for a force close, `SIGTERM` to ask).
pub fn signal(pid: u32, force: bool) {
    if pid == 0 || pid == std::process::id() {
        return;
    }
    // SAFETY: a signal to another process; an ID that went away just fails.
    unsafe {
        libc::kill(pid as libc::pid_t, if force { libc::SIGKILL } else { libc::SIGTERM });
    }
}

/// This computer's name (Spotify names its Connect device after it).
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").or_else(|_| std::fs::read_to_string("/etc/hostname")).map(|s| s.trim().to_owned()).unwrap_or_default()
}

/// The folder a program file really lives in (links followed).
pub fn real_dir(p: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(p).ok()?.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_paths_map_to_linux() {
        assert_eq!(wine_path(r"Z:\home\me\.local\share\Steam\steamapps\common\Game\Game.exe", None).as_deref(), Some("/home/me/.local/share/Steam/steamapps/common/Game/Game.exe"));
        assert_eq!(wine_path("not a path", None), None);
        assert_eq!(wine_path(r"C:\x.exe", None), None, "other drives need the prefix");
    }

    #[test]
    fn this_process_is_seen() {
        let me = std::process::id();
        assert!(!exe(me).is_empty());
        assert!(started(me).is_some_and(|t| t > 1.0e12));
        assert!(!comm(me).is_empty());
    }
}
