//! The apps Sofa lists on Linux (the Start menu's stand-in): the desktop entries, with their
//! icons from the icon theme, written to `cache/startmenu.json` in the Windows scan's shape
//! (plus each entry's own category). Runs only in the `--scan` child, so the icon theme
//! lookups and SVG drawing never stay in Sofa's memory.

use crate::linux::desktop::{self, Entry};
use serde_json::{Value, json};
use std::path::Path;

/// A stable short name for a cache file.
pub fn hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.to_lowercase().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The category an entry gives itself: games, media, or hidden for the system's own tools.
fn category(e: &Entry) -> &'static str {
    let cats: Vec<&str> = e.categories.split(';').map(str::trim).collect();
    let has = |c: &str| cats.iter().any(|x| x.eq_ignore_ascii_case(c));
    if has("Game") {
        "games"
    } else if has("AudioVideo") || has("Audio") || has("Video") || has("Player") || has("Music") || has("TV") {
        "media"
    } else if has("Settings") || has("System") || has("PackageManager") || has("TerminalEmulator") || has("Development") && has("Debugger") {
        "hidden"
    } else {
        "apps"
    }
}

/// An entry's icon as a file Sofa can draw: a PNG where it is, an SVG drawn into the cache.
fn icon_file(e: &Entry, cache: &Path) -> Option<String> {
    let found = desktop::find_icon(&e.icon)?;
    let ext = found.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" => Some(found.to_string_lossy().into_owned()),
        "svg" | "svgz" => {
            let out = cache.join("icons").join(format!("start-{}.png", hash(&found.to_string_lossy())));
            (out.exists() || desktop::svg_to_png(&found, &out)).then(|| out.to_string_lossy().into_owned())
        }
        _ => None,
    }
}

/// Lists the desktop entries into `cache/startmenu.json`. Returns how many.
pub fn scan(cache: &Path) -> usize {
    let mut list = Vec::new();
    for e in desktop::all() {
        // Not shown in menus, terminal tools, Steam's own game shortcuts (Steam's library has
        // them), and Sofa itself.
        let program = e.program();
        if e.hidden || e.terminal || e.exec.contains("steam://rungameid") || program == "sofa" || e.id == "sofa.desktop" {
            continue;
        }
        let mut v = json!({ "name": e.name, "appId": e.path.to_string_lossy(), "category": category(&e) });
        if let Some(icon) = icon_file(&e, cache) {
            v["icon"] = Value::from(icon);
        }
        list.push(v);
    }
    let n = list.len();
    let _ = std::fs::create_dir_all(cache);
    let tmp = cache.join("startmenu.json.tmp");
    if std::fs::write(&tmp, Value::Array(list).to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, cache.join("startmenu.json"));
    }
    n
}

/// Saves the icon of a program (the browser's tile, a Running card) as a PNG, from the desktop
/// entry that starts it.
pub fn save_exe_icon(exe: &str, out: &Path) -> bool {
    if out.exists() {
        return true;
    }
    let Some(e) = desktop::entry_for(exe, &Default::default()) else { return false };
    let Some(found) = desktop::find_icon(&e.icon) else { return false };
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    match found.extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref() {
        Some("svg" | "svgz") => desktop::svg_to_png(&found, out),
        _ => std::fs::copy(&found, out).is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(cats: &str) -> Entry {
        Entry { categories: cats.into(), ..Entry::default() }
    }

    #[test]
    fn entries_sort_themselves() {
        assert_eq!(category(&e("Game;ArcadeGame;")), "games");
        assert_eq!(category(&e("AudioVideo;Player;")), "media");
        assert_eq!(category(&e("GNOME;GTK;Settings;")), "hidden");
        assert_eq!(category(&e("Network;WebBrowser;")), "apps");
    }
}
