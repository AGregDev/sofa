//! Desktop entries (`.desktop` files, the freedesktop.org menu): what stands in for the Start
//! menu on Linux. Reading them, launching one, the program behind one (to find its window and
//! to name a Running card), and its icon from the icon theme.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, Default)]
pub struct Entry {
    /// The desktop file ID ("org.gnome.Nautilus.desktop").
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub categories: String,
    pub wm_class: String,
    pub path_dir: String,
    pub terminal: bool,
    pub hidden: bool,
}

impl Entry {
    /// The program the entry starts, lower case and without its folder ("firefox"; a
    /// Flatpak's app ID, "com.spotify.client").
    pub fn program(&self) -> String {
        program_of(&exec_args(&self.exec, self))
    }
}

/// `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`, most important first, with Flatpak's and Snap's
/// exports when the session does not list them.
pub fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut dirs = vec![std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".local/share"))];
    let sys = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    for extra in [home.join(".local/share/flatpak/exports/share"), PathBuf::from("/var/lib/flatpak/exports/share"), PathBuf::from("/var/lib/snapd/desktop")] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

/// The desktops this session is (`XDG_CURRENT_DESKTOP`, "ubuntu:GNOME" lists two).
fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect()
}

/// The person's language for `Name[xx]` (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`).
fn languages() -> Vec<String> {
    let raw = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX")).unwrap_or_default();
    let mut out = Vec::new();
    for l in raw.split(':') {
        let l = l.split(['.', '@']).next().unwrap_or("");
        if l.is_empty() {
            continue;
        }
        out.push(l.to_owned());
        if let Some((lang, _)) = l.split_once('_') {
            out.push(lang.to_owned());
        }
    }
    out
}

/// Reads one desktop file's `[Desktop Entry]` group.
pub fn parse(path: &Path, id: &str) -> Option<Entry> {
    let text = std::fs::read_to_string(path).ok()?;
    let langs = languages();
    let mut e = Entry { id: id.to_owned(), path: path.to_path_buf(), ..Entry::default() };
    let (mut in_group, mut kind, mut name_rank) = (false, String::new(), usize::MAX);
    let (mut only, mut not, mut try_exec) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_group = l == "[Desktop Entry]";
            continue;
        }
        if !in_group || l.starts_with('#') {
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if let Some(loc) = k.strip_prefix("Name[").and_then(|r| r.strip_suffix(']')) {
            if let Some(rank) = langs.iter().position(|x| x == loc)
                && rank < name_rank
            {
                name_rank = rank;
                e.name = unescape(v);
            }
            continue;
        }
        match k {
            "Type" => kind = v.to_owned(),
            "Name" if name_rank == usize::MAX => e.name = unescape(v),
            "Exec" => e.exec = v.to_owned(),
            "TryExec" => try_exec = v.to_owned(),
            "Icon" => e.icon = v.to_owned(),
            "Categories" => e.categories = v.to_owned(),
            "StartupWMClass" => e.wm_class = v.to_owned(),
            "Path" => e.path_dir = v.to_owned(),
            "Terminal" => e.terminal = v.eq_ignore_ascii_case("true"),
            "NoDisplay" | "Hidden" if v.eq_ignore_ascii_case("true") => e.hidden = true,
            "OnlyShowIn" => only = v.to_lowercase(),
            "NotShowIn" => not = v.to_lowercase(),
            _ => {}
        }
    }
    if kind != "Application" || e.exec.is_empty() || e.name.is_empty() {
        return None;
    }
    let desktops = current_desktops();
    let listed = |list: &str| list.split(';').any(|d| !d.is_empty() && desktops.iter().any(|c| c == d));
    if (!only.is_empty() && !listed(&only)) || (!not.is_empty() && listed(&not)) {
        e.hidden = true;
    }
    if !try_exec.is_empty() && which(&try_exec).is_none() {
        return None;
    }
    Some(e)
}

/// `\s`, `\n`, `\t`, `\r` and `\\` in desktop-file strings.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Every application entry, by desktop file ID, the first of each ID winning (the person's own
/// entries before the system's).
pub fn all() -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in data_dirs() {
        let apps = dir.join("applications");
        let mut stack = vec![(apps.clone(), String::new())];
        while let Some((d, prefix)) = stack.pop() {
            let Ok(list) = std::fs::read_dir(&d) else { continue };
            for f in list.flatten() {
                let p = f.path();
                let name = f.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    stack.push((p, format!("{prefix}{name}-")));
                } else if name.ends_with(".desktop") {
                    let id = format!("{prefix}{name}");
                    if seen.insert(id.clone())
                        && let Some(e) = parse(&p, &id)
                    {
                        out.push(e);
                    }
                }
            }
        }
    }
    out
}

/// The entries, read once per process (to name and match running programs).
fn index() -> &'static [Entry] {
    static INDEX: OnceLock<Vec<Entry>> = OnceLock::new();
    INDEX.get_or_init(|| all().into_iter().filter(|e| !e.hidden || !e.wm_class.is_empty()).collect())
}

/// A program file in PATH (or the path itself when it has a folder).
pub fn which(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return p.is_file().then_some(p);
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(prog)).find(|p| p.is_file()))
}

/// The arguments of an `Exec` line: quoting undone, field codes (%f, %U...) dropped, %i, %c
/// and %k filled in.
pub fn exec_args(exec: &str, e: &Entry) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut any) = (false, false);
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            ' ' | '\t' if !quoted => {
                if any || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            '%' => match it.next() {
                Some('%') => cur.push('%'),
                Some('i') if !e.icon.is_empty() => {
                    if !cur.is_empty() {
                        args.push(std::mem::take(&mut cur));
                    }
                    args.push("--icon".into());
                    cur = e.icon.clone();
                    any = true;
                }
                Some('c') => cur.push_str(&e.name),
                Some('k') => cur.push_str(&e.path.to_string_lossy()),
                _ => {}
            },
            _ => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any || !cur.is_empty() {
        args.push(cur);
    }
    args.retain(|a| !a.is_empty());
    args
}

/// The program an argument list runs, past `env VAR=...` and `flatpak run` ("firefox",
/// "com.spotify.client").
pub fn program_of(args: &[String]) -> String {
    let mut i = 0;
    if args.first().is_some_and(|a| a == "env" || a.ends_with("/env")) {
        i = 1;
        while args.get(i).is_some_and(|a| a.contains('=') || a.starts_with('-')) {
            i += 1;
        }
    }
    let Some(first) = args.get(i) else { return String::new() };
    let base = first.rsplit('/').next().unwrap_or(first).to_lowercase();
    if base == "flatpak" && args.get(i + 1).is_some_and(|a| a == "run") {
        return args[i + 2..].iter().find(|a| !a.starts_with('-')).map(|a| a.to_lowercase()).unwrap_or_default();
    }
    base
}

/// Starts an entry, detached from Sofa.
pub fn launch(path: &Path) -> bool {
    let id = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(e) = parse(path, &id) else { return false };
    let args = exec_args(&e.exec, &e);
    let Some((prog, rest)) = args.split_first() else { return false };
    let mut c = std::process::Command::new(prog);
    c.args(rest);
    if !e.path_dir.is_empty() && Path::new(&e.path_dir).is_dir() {
        c.current_dir(&e.path_dir);
    }
    crate::platform::spawn_detached(c)
}

/// The entry for a running program: by its window class (StartupWMClass, or the desktop file's
/// own name), then by the program file's name.
pub fn entry_for(exe: &str, class: &(String, String)) -> Option<&'static Entry> {
    let idx = index();
    let lc = |s: &str| s.to_lowercase();
    let (inst, cls) = (lc(&class.0), lc(&class.1));
    let stem = |e: &Entry| lc(e.id.strip_suffix(".desktop").unwrap_or(&e.id));
    if !cls.is_empty() || !inst.is_empty() {
        let by_class = idx.iter().find(|e| {
            let w = lc(&e.wm_class);
            !w.is_empty() && (w == cls || w == inst)
        });
        if by_class.is_some() {
            return by_class;
        }
        let by_id = idx.iter().find(|e| {
            let s = stem(e);
            s == cls || s == inst || s.rsplit('.').next().is_some_and(|t| t == cls || t == inst)
        });
        if by_id.is_some() {
            return by_id;
        }
    }
    let base = lc(exe.rsplit('/').next().unwrap_or(exe));
    if base.is_empty() {
        return None;
    }
    idx.iter().find(|e| !e.hidden && e.program() == base)
}

/// The open window of the program a desktop entry starts (its class, else its program).
pub fn window_for(path: &Path) -> Option<isize> {
    let id = path.file_name()?.to_string_lossy().into_owned();
    let e = parse(path, &id)?;
    let (prog, class) = (e.program(), e.wm_class.to_lowercase());
    if prog.is_empty() || prog == "sofa" {
        return None;
    }
    crate::shell::top_windows().into_iter().find(|&h| {
        let (inst, cls) = crate::linux::x11::class(h);
        let (inst, cls) = (inst.to_lowercase(), cls.to_lowercase());
        let exe = crate::shell::exe_of(crate::shell::pid(h)).to_lowercase();
        (!class.is_empty() && (inst == class || cls == class)) || exe.rsplit('/').next() == Some(prog.as_str()) || inst == prog || cls == prog
    })
}

/// The name an entry gives a running program ("Firefox").
pub fn name_for(exe: &str, class: &(String, String)) -> String {
    entry_for(exe, class).map(|e| e.name.clone()).unwrap_or_default()
}

// ---------- Icons ----------

/// The icon theme the person uses (GNOME, then KDE), with the themes it inherits from, ending
/// with hicolor.
fn themes(bases: &[PathBuf]) -> Vec<String> {
    let mut first = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_matches('\'').to_owned())
        .filter(|s| !s.is_empty());
    if first.is_none() {
        let kde = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".config/kdeglobals");
        first = std::fs::read_to_string(kde).ok().and_then(|t| {
            let mut in_icons = false;
            t.lines().find_map(|l| {
                let l = l.trim();
                if l.starts_with('[') {
                    in_icons = l == "[Icons]";
                    return None;
                }
                l.strip_prefix("Theme=").filter(|_| in_icons).map(str::to_owned)
            })
        });
    }
    let mut out: Vec<String> = Vec::new();
    let mut queue: Vec<String> = first.into_iter().collect();
    queue.extend(["Adwaita".to_owned(), "breeze".to_owned()]);
    while let Some(t) = queue.first().cloned() {
        queue.remove(0);
        if out.contains(&t) || t == "hicolor" {
            continue;
        }
        out.push(t.clone());
        for b in bases {
            if let Ok(text) = std::fs::read_to_string(b.join(&t).join("index.theme"))
                && let Some(inh) = text.lines().find_map(|l| l.trim().strip_prefix("Inherits="))
            {
                for i in inh.split(',').map(str::trim).filter(|s| !s.is_empty()).rev() {
                    queue.insert(0, i.to_owned());
                }
                break;
            }
        }
    }
    out.push("hicolor".into());
    out
}

fn icon_bases() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut b = vec![home.join(".icons")];
    b.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    b
}

/// The file for an icon name: the largest PNG up to 512 px, or the scalable SVG, in the
/// person's theme first. A full path is taken as it is.
pub fn find_icon(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    static CACHE: OnceLock<std::sync::Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(name).cloned()) {
        return hit;
    }
    static THEMES: OnceLock<(Vec<PathBuf>, Vec<String>)> = OnceLock::new();
    let (bases, list) = THEMES.get_or_init(|| {
        let bases = icon_bases();
        let list = themes(&bases);
        (bases, list)
    });
    let found = (|| {
        const SIZES: [&str; 9] = ["512x512", "256x256", "scalable", "192x192", "128x128", "96x96", "64x64", "48x48", "32x32"];
        const BREEZE: [&str; 3] = ["scalable", "64", "48"];
        for theme in list {
            for size in SIZES {
                for b in bases {
                    for cat in ["apps", "applications"] {
                        for ext in ["png", "svg"] {
                            let p = b.join(theme).join(size).join(cat).join(format!("{name}.{ext}"));
                            if p.is_file() {
                                return Some(p);
                            }
                        }
                    }
                }
            }
            // Breeze and others put the category first ("apps/48/name.svg").
            for size in BREEZE {
                for b in bases {
                    let p = b.join(theme).join("apps").join(size).join(format!("{name}.svg"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        for b in data_dirs() {
            for ext in ["png", "svg"] {
                let p = b.join("pixmaps").join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    })();
    if let Ok(mut c) = cache.lock() {
        c.insert(name.to_owned(), found.clone());
    }
    found
}

/// Renders an SVG icon to a 256 px PNG (only in the `--scan` child: resvg stays out of Sofa's
/// own memory).
pub fn svg_to_png(svg: &Path, out: &Path) -> bool {
    use resvg::{tiny_skia, usvg};
    let Ok(data) = std::fs::read(svg) else { return false };
    let Ok(tree) = usvg::Tree::from_data(&data, &usvg::Options::default()) else { return false };
    let size = tree.size();
    let s = 256.0 / size.width().max(size.height()).max(1.0);
    let Some(mut pm) = tiny_skia::Pixmap::new(256, 256) else { return false };
    let (dx, dy) = ((256.0 - size.width() * s) / 2.0, (256.0 - size.height() * s) / 2.0);
    resvg::render(&tree, tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut pm.as_mut());
    if let Some(d) = out.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    pm.save_png(out).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(exec: &str) -> Entry {
        Entry { name: "Files".into(), icon: "org.gnome.Nautilus".into(), exec: exec.into(), ..Entry::default() }
    }

    #[test]
    fn exec_lines_lose_their_field_codes() {
        let e = entry("nautilus --new-window %U");
        assert_eq!(exec_args(&e.exec, &e), ["nautilus", "--new-window"]);
        let e = entry(r#""/opt/My App/app" --name="%c" %% %F"#);
        assert_eq!(exec_args(&e.exec, &e), ["/opt/My App/app", "--name=Files", "%"]);
        let e = entry("app %i");
        assert_eq!(exec_args(&e.exec, &e), ["app", "--icon", "org.gnome.Nautilus"]);
    }

    #[test]
    fn programs_are_found_past_env_and_flatpak() {
        let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(program_of(&args("/usr/bin/firefox %u")), "firefox");
        assert_eq!(program_of(&args("env GDK_BACKEND=x11 spotify")), "spotify");
        assert_eq!(program_of(&args("/usr/bin/flatpak run --branch=stable --arch=x86_64 com.spotify.Client")), "com.spotify.client");
    }

    #[test]
    fn entries_are_read_with_their_names_and_rules() {
        let dir = std::env::temp_dir().join(format!("sofa-desktop-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("a.desktop");
        std::fs::write(&f, "[Desktop Entry]\nType=Application\nName=Game\\sCentre\nExec=sh -c true\nCategories=Game;\nNoDisplay=false\n\n[Desktop Action x]\nName=Other\n").expect("write");
        let e = parse(&f, "a.desktop").expect("an application");
        assert_eq!(e.name, "Game Centre");
        assert_eq!(e.categories, "Game;");
        assert!(!e.hidden);
        std::fs::write(&f, "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.com\n").expect("write");
        assert!(parse(&f, "a.desktop").is_none(), "links are not applications");
        let _ = std::fs::remove_dir_all(dir);
    }
}
