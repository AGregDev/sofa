//! Sofa's library: Steam games, RecompLauncher ports, Start menu apps, streaming services and
//! system actions, each sorted into a category. A port of `src/library.js` that reads Steam's
//! files through the verified `vdf-rs` port. On Linux the Start menu is the desktop entries
//! (`linux/startmenu.rs` writes the same `cache/startmenu.json`) and Steam is the native,
//! Flatpak or Snap one.

use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use vdf::{get, parse_vdf};

#[derive(Clone, Debug, Default)]
pub struct Art {
    pub capsule: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub header: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    /// The capsule is a cover the person picked.
    pub custom: bool,
}

#[derive(Clone, Debug)]
pub enum Launch {
    Url(String),
    AppsFolder(String),
    /// A program and its arguments (RecompLauncher's games, Windows only so far).
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Exec { exe: PathBuf, args: Vec<String> },
    Web { url: Option<String>, tv: bool },
    System { action: String, uri: Option<String> },
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub source: &'static str, // steam | recomp | start | web | system | nav
    pub category: String,     // games | media | apps | system | hidden
    pub launch: Launch,
    pub art: Art,
    pub color: Option<String>,
    pub glyph: Option<&'static str>,
    pub install_path: Option<String>,
    /// A program whose icon stands in for missing art (RecompLauncher games, the browser).
    pub icon_from: Option<PathBuf>,
    pub project: Option<String>,
    pub confirm: bool,
    pub favorite: bool,
    pub last_played: f64,
    /// Minutes played in all (Steam keeps it; 0 when unknown).
    pub playtime: u32,
}

// ---------- vdf helpers ----------

fn read_vdf(file: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(file).ok()?;
    parse_vdf(Value::String(text)).ok()
}

/// `get(o, key)` through the verified port; JS `undefined` comes back as None.
fn field(o: &Value, key: &str) -> Option<Value> {
    let v = get(o.clone(), Value::String(key.into())).ok()?;
    let undefined = v.as_object().is_some_and(|m| m.contains_key("$undefined"));
    (!undefined && !v.is_null()).then_some(v)
}

fn text_field(o: &Value, key: &str) -> Option<String> {
    field(o, key).and_then(|v| v.as_str().map(str::to_owned)).filter(|s| !s.is_empty())
}

// ---------- Steam ----------

/// Steam's folder on Linux: the native client's, the Flatpak's or the Snap's, whichever has a
/// library.
#[cfg(target_os = "linux")]
pub fn steam_root() -> Option<PathBuf> {
    let home = crate::linux::home();
    [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join(".var/app/com.valvesoftware.Steam/.steam/steam"),
        home.join("snap/steam/common/.local/share/Steam"),
    ]
    .into_iter()
    .filter_map(|p| std::fs::canonicalize(p).ok())
    .find(|p| p.join("steamapps").is_dir())
}

#[cfg(windows)]
pub fn steam_root() -> Option<PathBuf> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::w;
    let mut buf = [0u16; 512];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte length are valid for the call.
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
    }
    .is_ok();
    if ok {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let p = PathBuf::from(String::from_utf16_lossy(&buf[..n]).replace('/', "\\"));
        if p.exists() {
            return Some(p);
        }
    }
    let fallback = PathBuf::from(r"C:\Program Files (x86)\Steam");
    fallback.exists().then_some(fallback)
}

static STEAM_SKIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)redistributable|steamworks common|proton|steam linux runtime|steamvr|dedicated server|\bsdk\b").unwrap());
const STEAM_SKIP_IDS: [&str; 4] = ["228980", "1070560", "1391110", "1628350"];

/// Each Steam game's last start (epoch ms) and minutes played in all, from every account on the
/// PC (the latest start and the most minutes win).
fn steam_last_played(root: &Path) -> HashMap<String, (f64, u32)> {
    let mut result = HashMap::new();
    let Ok(users) = std::fs::read_dir(root.join("userdata")) else { return result };
    for u in users.flatten() {
        let Some(cfg) = read_vdf(&u.path().join("config").join("localconfig.vdf")) else { continue };
        let apps = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"].iter().try_fold(cfg, |o, k| field(&o, k));
        let Some(Value::Object(apps)) = apps else { continue };
        for (id, a) in apps {
            let t = text_field(&a, "LastPlayed").and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) * 1000.0;
            let minutes = text_field(&a, "Playtime").and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let e = result.entry(id).or_insert((0.0, 0));
            e.0 = e.0.max(t);
            e.1 = e.1.max(minutes);
        }
    }
    result
}

fn steam_art(root: &Path, appid: &str) -> Art {
    let mut art = Art::default();
    let dir = root.join("appcache").join("librarycache").join(appid);
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let slot = match name.as_str() {
                "library_600x900.jpg" | "library_capsule.jpg" => &mut art.capsule,
                "library_hero.jpg" => &mut art.hero,
                "logo.png" => &mut art.logo,
                "header.jpg" | "library_header.jpg" => &mut art.header,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(p);
            }
        }
    }
    art
}

fn scan_steam(root: &Path) -> Vec<Item> {
    let mut libs = vec![root.to_path_buf()];
    if let Some(lf) = read_vdf(&root.join("steamapps").join("libraryfolders.vdf"))
        && let Some(Value::Object(folders)) = field(&lf, "libraryfolders")
    {
        for v in folders.values() {
            if let Some(p) = text_field(v, "path") {
                let p = PathBuf::from(p.replace("\\\\", "\\"));
                if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
                    libs.push(p);
                }
            }
        }
    }
    let played = steam_last_played(root);
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for lib in libs {
        let apps = lib.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&apps) else { continue };
        for e in entries.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            let Some(st) = read_vdf(&e.path()).and_then(|v| field(&v, "AppState")) else { continue };
            let (Some(appid), Some(name)) = (text_field(&st, "appid"), text_field(&st, "name")) else { continue };
            if seen.contains(&appid) || STEAM_SKIP_IDS.contains(&appid.as_str()) || STEAM_SKIP.is_match(&name) {
                continue;
            }
            seen.insert(appid.clone());
            let install = text_field(&st, "installdir").map(|d| apps.join("common").join(d).to_string_lossy().to_lowercase());
            items.push(Item {
                id: format!("steam:{appid}"),
                category: if name.to_lowercase().contains("wallpaper engine") { "apps" } else { "games" }.into(),
                name,
                source: "steam",
                launch: Launch::Url(format!("steam://rungameid/{appid}")),
                art: steam_art(root, &appid),
                color: None,
                glyph: None,
                install_path: install,
                icon_from: None,
                project: None,
                confirm: false,
                favorite: false,
                last_played: played.get(&appid).map_or(0.0, |p| p.0),
                playtime: played.get(&appid).map_or(0, |p| p.1),
            });
        }
    }
    items
}

// ---------- RecompLauncher ----------

#[cfg(windows)]
const RECOMP_TITLES: [(&str, &str); 10] = [
    ("longshot", "The Legend of Zelda: Ocarina of Time"),
    ("shipwright", "The Legend of Zelda: Ocarina of Time"),
    ("zelda64recomp", "The Legend of Zelda: Majora's Mask"),
    ("2ship2harkinian", "The Legend of Zelda: Majora's Mask"),
    ("spaghettikart", "Mario Kart 64"),
    ("starship", "Star Fox 64"),
    ("banjorecomp", "Banjo-Kazooie"),
    ("donkey-kong-64-recompiled", "Donkey Kong 64"),
    ("perfect-dark", "Perfect Dark"),
    ("unleashedrecomp", "Sonic Unleashed"),
];

/// RecompLauncher is a Windows launcher.
#[cfg(target_os = "linux")]
fn scan_recomp(_cache: &Path) -> Vec<Item> {
    Vec::new()
}

#[cfg(windows)]
fn scan_recomp(cache: &Path) -> Vec<Item> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from).unwrap_or_default();
    let data = env("APPDATA").join("com.recomplauncher.app");
    let launcher = env("LOCALAPPDATA").join("RecompLauncher").join("recomplauncher.exe");
    let Some(lib) = std::fs::read_to_string(data.join("library.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return vec![];
    };
    let Some(installed) = lib.get("installed").and_then(Value::as_object).filter(|_| launcher.exists()) else { return vec![] };
    let mut items = Vec::new();
    for g in installed.values() {
        let (Some(id), Some(path)) = (g.get("id").and_then(Value::as_str), g.get("path").and_then(Value::as_str)) else { continue };
        let cover = g.get("cover_file").and_then(Value::as_str).map(|c| {
            let p = PathBuf::from(c);
            if p.is_absolute() { p } else { data.join("covers").join(c) }
        });
        let name = RECOMP_TITLES.iter().find(|(k, _)| *k == id).map(|(_, v)| (*v).to_owned());
        let project = g.get("name").and_then(Value::as_str).map(str::to_owned);
        let mut art = Art { capsule: cover.filter(|c| c.exists()), ..Art::default() };
        // The game's own program supplies the icon: in its folder or one level down.
        let game_exe = g.get("exe").and_then(Value::as_str).and_then(|exe| {
            let direct = PathBuf::from(path).join(exe);
            if direct.exists() {
                return Some(direct);
            }
            let files = g.get("files").and_then(Value::as_array)?;
            files.iter().filter_map(Value::as_str).find(|f| Path::new(f).file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(exe))).map(|f| PathBuf::from(path).join(f))
        });
        if art.capsule.is_none() {
            art.icon = game_exe.as_ref().map(|e| icon_cache(cache, e)).filter(|p| p.exists());
        }
        items.push(Item {
            id: format!("recomp:{id}"),
            name: name.or_else(|| project.clone()).unwrap_or_else(|| id.to_owned()),
            source: "recomp",
            category: "games".into(),
            launch: Launch::Exec { exe: launcher.clone(), args: vec!["--play".into(), id.into()] },
            art,
            color: None,
            glyph: None,
            install_path: Some(path.to_lowercase()),
            icon_from: game_exe,
            project,
            confirm: false,
            favorite: false,
            last_played: g.get("last_played").and_then(Value::as_f64).unwrap_or(0.0) * 1000.0,
            playtime: 0,
        });
    }
    items
}

/// Where a program's extracted icon is cached.
pub fn icon_cache(cache: &Path, exe: &Path) -> PathBuf {
    {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in exe.to_string_lossy().to_lowercase().bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        cache.join("icons").join(format!("exe-{h:016x}.png"))
    }
}

/// Programs whose icons are still to be extracted (run on the Start menu thread).
pub fn missing_icons(items: &[Item], cache: &Path) -> Vec<(String, PathBuf)> {
    items
        .iter()
        .filter(|i| i.art.icon.is_none() && i.art.capsule.is_none())
        .filter_map(|i| i.icon_from.as_ref().map(|e| (e.to_string_lossy().to_string(), icon_cache(cache, e))))
        .collect()
}

// ---------- Start menu ----------

static HIDE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("(?i){}", [
        "uninstall", "readme", "read me", "documentation", "manual", "release notes", "samples", "reference", "faq",
        "website", "\\bhelp\\b", "support center", "localization", "updater", "error report", "bug report", "crash",
        "install additional", "command prompt", "powershell", "^run$", "registry", "^services$", "task scheduler",
        "event viewer", "performance monitor", "resource monitor", "system configuration", "system information",
        "computer management", "component services", "disk cleanup", "defragment", "iscsi", "odbc", "local security",
        "memory diagnostic", "firewall", "recovery drive", "windows tools", "character map", "magnifier", "narrator",
        "voice access", "live captions", "on-screen keyboard", "app recovery", "check for updates", "get started",
        "click to do", "control panel", "about java", "configure java", "desktop overlay host", "^idle", "^python",
        "node\\.js", "^git ", "private browsing", "skinned", "reset preferences", "remote desktop", "module docs",
        "windows backup", "^terminal$", "snipping tool", "game bar", "phone link", "translucenttb",
    ].join("|")))
    .unwrap()
});
static GAME_NAMES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^ea$|riot client|league of legends|valorant|rockstar games launcher|minecraft|ankama|epic games|ubisoft connect|battle\.net|gog galaxy|^xbox$|recomplauncher|playnite|heroic|itch|retroarch|dolphin|pcsx2|rpcs3|ryujinx|cemu|ppsspp|duckstation|launcher$").unwrap());
static GAME_PATHS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\\(games|steamlibrary|steamapps|epic games|gog games|riot games)\\").unwrap());
static MEDIA_NAMES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)spotify|vlc|netflix|plex|kodi|jellyfin|disney|prime video|youtube|twitch|^photos$|media player|movies & tv|apple tv|apple music|itunes|crunchyroll|hulu|^max$|deezer|tidal|amazon music|mpc-|potplayer|stremio|audible|soundcloud").unwrap());
static NOT_APP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(url|pdf|txt|html?|chm|msc)$").unwrap());
const SKIP_DIRS: [&str; 8] = ["bin", "binaries", "x64", "win64", "win32", "game", "retail", "common"];

pub fn friendly_name(name: &str, app_id: &str) -> String {
    let clean = name.replace(['®', '™'], "").replace('\u{A789}', ":").split_whitespace().collect::<Vec<_>>().join(" ");
    let looks_like_exe = !clean.contains(' ') && (clean.contains('_') || clean.chars().count() <= 4);
    if !looks_like_exe || !app_id.contains(['\\', '/']) {
        return clean;
    }
    let parts: Vec<&str> = app_id.split(['\\', '/']).collect();
    parts[..parts.len() - 1]
        .iter()
        .rev()
        .find(|p| !p.is_empty() && !SKIP_DIRS.contains(&p.to_lowercase().as_str()) && !p.starts_with('{') && !p.ends_with(':'))
        .map_or(clean, |d| (*d).to_owned())
}

pub fn categorize_start(name: &str, app_id: &str) -> &'static str {
    let lid = app_id.to_lowercase();
    if lid.starts_with("http:") || lid.starts_with("https:") || lid.starts_with("steam:") || NOT_APP.is_match(app_id) || HIDE.is_match(name) {
        "hidden"
    } else if MEDIA_NAMES.is_match(name) {
        "media"
    } else if GAME_NAMES.is_match(name) || GAME_PATHS.is_match(app_id) {
        "games"
    } else {
        "apps"
    }
}

/// Start menu apps as Sofa last listed them (`cache\startmenu.json`, from `startmenu::scan`;
/// before the first scan finishes, Sofa 1.x's `cache\startapps.json`), with the category a
/// desktop entry gives itself (Linux only).
fn start_apps(cache: &Path) -> Vec<(String, String, Option<PathBuf>, Option<String>)> {
    let text = std::fs::read_to_string(cache.join("startmenu.json")).or_else(|_| std::fs::read_to_string(cache.join("startapps.json")));
    let Ok(text) = text else { return vec![] };
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return vec![] };
    list.iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?.to_owned();
            let id = a.get("appId")?.as_str()?.to_owned();
            let icon = a.get("icon").and_then(Value::as_str).map(PathBuf::from).filter(|p| p.exists());
            let category = a.get("category").and_then(Value::as_str).map(str::to_owned);
            Some((name, id, icon, category))
        })
        .collect()
}

// ---------- Streaming + system ----------

pub const WEB: [(&str, &str, Option<&str>, bool, &str); 7] = [
    ("youtube", "YouTube", Some("https://www.youtube.com/tv"), true, "#ff2d55"),
    ("netflix", "Netflix", Some("https://www.netflix.com"), false, "#e50914"),
    ("disney", "Disney+", Some("https://www.disneyplus.com"), false, "#1f5bff"),
    ("prime", "Prime Video", Some("https://www.primevideo.com"), false, "#00a8e1"),
    ("crunchyroll", "Crunchyroll", Some("https://www.crunchyroll.com"), false, "#f47521"),
    ("twitch", "Twitch", Some("https://www.twitch.tv"), false, "#9146ff"),
    ("browser", "Browser", None, false, "#f0a24b"),
];

pub const SYSTEM: [(&str, &str, bool, Option<&str>); 11] = [
    ("sleep", "Sleep", false, None),
    ("restart", "Restart", true, None),
    ("shutdown", "Shut down", true, None),
    ("desktop", "Go to desktop", false, None),
    ("quit", "Quit Sofa", false, None),
    ("bigpicture", "Steam Big Picture", false, None),
    ("settings", "Sofa settings", false, None),
    ("rescan", "Rescan library", false, None),
    ("display", "Display settings", false, Some("ms-settings:display")),
    ("sound", "Sound output", false, None),
    ("bluetooth", "Bluetooth & controllers", false, Some("ms-settings:bluetooth")),
];

/// Builds the library. `overrides` and `recent` come from `sofa.json`.
pub fn build(cache: &Path, overrides: &serde_json::Map<String, Value>, recent: &serde_json::Map<String, Value>, browser: Option<PathBuf>) -> (Option<PathBuf>, Vec<Item>) {
    let root = steam_root();
    let mut items: Vec<Item> = root.as_deref().map(scan_steam).unwrap_or_default();
    items.extend(scan_recomp(cache));
    let game_paths: Vec<String> = items.iter().filter_map(|i| i.install_path.clone()).collect();
    let mut seen_names = HashSet::new();
    for (name, id, icon, hint) in start_apps(cache) {
        let lid = id.to_lowercase();
        // Sofa's own shortcut: Sofa is already open.
        if lid.starts_with("steam:") || lid.ends_with("\\sofa.exe") || game_paths.iter().any(|p| lid.starts_with(p.as_str())) {
            continue;
        }
        let name = friendly_name(&name, &id);
        if !seen_names.insert(name.to_lowercase()) {
            continue;
        }
        // A desktop entry's own category (Game, AudioVideo) sorts what the names do not.
        let category = match (categorize_start(&name, &id), hint.as_deref()) {
            ("apps", Some(h)) => h.to_owned(),
            (c, _) => c.to_owned(),
        };
        items.push(Item {
            id: format!("start:{id}"),
            category,
            name,
            source: "start",
            launch: Launch::AppsFolder(id),
            art: Art { icon, ..Art::default() },
            color: None,
            glyph: None,
            install_path: None,
            icon_from: None,
            project: None,
            confirm: false,
            favorite: false,
            last_played: 0.0,
            playtime: 0,
        });
    }
    for (key, name, url, tv, color) in WEB {
        // The Browser tile shows the browser Sofa will use, and only when there is one.
        if key == "browser" && browser.is_none() {
            continue;
        }
        let icon_from = if key == "browser" { browser.clone() } else { None };
        let icon = icon_from.as_ref().map(|e| icon_cache(cache, e)).filter(|p| p.exists());
        items.push(Item {
            id: format!("web:{key}"),
            name: name.into(),
            source: "web",
            category: if key == "browser" { "apps" } else { "media" }.into(),
            launch: Launch::Web { url: url.map(str::to_owned), tv },
            art: Art { icon, ..Art::default() },
            color: Some(color.into()),
            glyph: None,
            install_path: None,
            icon_from,
            project: None,
            confirm: false,
            favorite: false,
            last_played: 0.0,
            playtime: 0,
        });
    }
    for (key, name, confirm, uri) in SYSTEM {
        items.push(Item {
            id: format!("sys:{key}"),
            name: name.into(),
            source: "system",
            category: "system".into(),
            launch: Launch::System { action: key.into(), uri: uri.map(str::to_owned) },
            art: Art::default(),
            color: None,
            glyph: Some(key),
            install_path: None,
            icon_from: None,
            project: None,
            confirm,
            favorite: false,
            last_played: 0.0,
            playtime: 0,
        });
    }
    // Colours the Electron app computed from each item's art.
    let colors: serde_json::Map<String, Value> = std::fs::read_to_string(cache.join("colors.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for it in &mut items {
        // Person's overrides: name, category, hidden, favorite, custom cover.
        if let Some(ov) = overrides.get(&it.id).and_then(Value::as_object) {
            if let Some(n) = ov.get("name").and_then(Value::as_str) {
                it.name = n.into();
            }
            if let Some(c) = ov.get("category").and_then(Value::as_str) {
                it.category = c.into();
            }
            if ov.get("hidden").and_then(Value::as_bool) == Some(true) {
                it.category = "hidden".into();
            }
            it.favorite = ov.get("favorite").and_then(Value::as_bool) == Some(true);
            if let Some(c) = ov.get("cover").and_then(Value::as_str) {
                it.art.capsule = Some(PathBuf::from(c));
                it.art.custom = true;
            }
        }
        if let Some(t) = recent.get(&it.id).and_then(Value::as_f64) {
            it.last_played = it.last_played.max(t);
        }
        if it.color.is_none() {
            let src = it.art.capsule.as_ref().or(it.art.header.as_ref()).or(it.art.icon.as_ref());
            it.color = src.and_then(|s| colors.get(&s.to_string_lossy().to_string())).and_then(Value::as_str).map(str::to_owned);
        }
    }
    (root, items)
}

/// Which library item a running program belongs to, from its executable path. A port of 1.x's
/// `identify` (main.js): install folders first, then Start menu entries, then the name.
pub fn identify(items: &[Item], exe: &str) -> Option<usize> {
    let e = exe.to_lowercase();
    if e.is_empty() {
        return None;
    }
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let windir = env("SystemRoot");
    let known: [(&str, String); 5] = [
        ("{6D809377-6AF0-444B-8957-A3773F02200E}", env("ProgramFiles")),
        ("{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}", env("ProgramFiles(x86)")),
        ("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}", format!("{windir}\\System32")),
        ("{D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27}", format!("{windir}\\SysWOW64")),
        ("{F38BF404-1D43-42F2-9305-67DE0B28FC23}", windir.clone()),
    ];
    for (i, it) in items.iter().enumerate() {
        if it.install_path.as_ref().is_some_and(|p| e.starts_with(&format!("{p}{}", std::path::MAIN_SEPARATOR))) {
            return Some(i);
        }
        let Launch::AppsFolder(id) = &it.launch else { continue };
        let mut path = id.clone();
        if let Some((guid, rest)) = id.split_once('\\')
            && let Some((_, dir)) = known.iter().find(|(g, _)| g.eq_ignore_ascii_case(guid))
        {
            path = format!("{dir}\\{rest}");
        }
        if path.to_lowercase() == e {
            return Some(i);
        }
        // Store apps: Name_publisher!App lives under \WindowsApps\Name_..._publisher\.
        if let Some((pkg, _)) = id.split_once('!')
            && let Some((name, publisher)) = pkg.rsplit_once('_')
            && e.contains(&format!("\\windowsapps\\{}_", name.to_lowercase()))
            && e.contains(&format!("__{}", publisher.to_lowercase()))
        {
            return Some(i);
        }
    }
    // Last resort: the program is named like the app (discord.exe is Discord).
    let base: String = Path::new(&e).file_stem().map(|s| s.to_string_lossy().chars().filter(char::is_ascii_alphanumeric).collect()).unwrap_or_default();
    items.iter().position(|it| it.source == "start" && base.len() > 2 && it.name.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect::<String>() == base)
}

/// A Start menu app by the program's own name (its file description): Start menu entries
/// that carry an app ID instead of a path (Firefox, Chrome) are found this way.
pub fn identify_name(items: &[Item], app: &str) -> Option<usize> {
    let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    let a = norm(app);
    if a.len() < 3 {
        return None;
    }
    items.iter().position(|it| it.source == "start" && it.category != "hidden" && norm(&it.name) == a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_item(name: &str, id: &str, category: &str) -> Item {
        Item {
            id: format!("start:{id}"),
            name: name.into(),
            source: "start",
            category: category.into(),
            launch: Launch::AppsFolder(id.into()),
            art: Art::default(),
            color: None,
            glyph: None,
            install_path: None,
            icon_from: None,
            project: None,
            confirm: false,
            favorite: false,
            last_played: 0.0,
            playtime: 0,
        }
    }

    #[test]
    fn running_apps_are_found_by_their_own_name() {
        let items = vec![
            start_item("Firefox Private Browsing", "308046B0AF4A39CB;PrivateBrowsingAUMID", "hidden"),
            start_item("Firefox", "308046B0AF4A39CB", "apps"),
            start_item("Google Chrome", "Chrome", "apps"),
        ];
        assert_eq!(identify_name(&items, "Firefox"), Some(1));
        assert_eq!(identify_name(&items, "Google Chrome"), Some(2));
        assert_eq!(identify_name(&items, "Firefox Private Browsing"), None, "hidden entries never name a card");
        assert_eq!(identify_name(&items, "VS"), None, "too short to trust");
        assert_eq!(identify_name(&items, ""), None);
    }
}
