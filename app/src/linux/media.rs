//! What is playing, on Linux: the MPRIS players on the session bus (Spotify, browsers, VLC,
//! anything the desktop's media controls show), read on the `media` thread, which a second
//! thread wakes on their PropertiesChanged signals. The quick menu shows the track and its art
//! and controls it through the same interface, without touching the player's window. The same
//! API as the Windows `media.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

/// WM_APP + 14: what is playing changed.
pub const WM_MEDIA: u32 = 0x8000 + 14;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// The app playing it ("Spotify", "Firefox").
    pub app: String,
    pub playing: bool,
    /// The album or video art, saved in Sofa's cache.
    pub art: Option<PathBuf>,
}

pub enum Cmd {
    Toggle,
    Next,
    Previous,
    /// A player reported a change: read it again.
    Refresh,
}

pub struct Media {
    tx: Option<Sender<Cmd>>,
    state: Arc<Mutex<Option<NowPlaying>>>,
}

impl Media {
    /// Starts the `media` thread (not when `enabled` is false: a QA run must not control the
    /// real players, and sets its own made-up track with `set`).
    pub fn start(cache: PathBuf, hwnd: isize, enabled: bool) -> Media {
        let state = Arc::new(Mutex::new(None));
        if !enabled {
            return Media { tx: None, state };
        }
        let (tx, rx) = channel();
        let (shared, events) = (state.clone(), tx.clone());
        let _ = std::thread::Builder::new().name("media".into()).spawn(move || media_thread(rx, events, shared, cache, hwnd));
        Media { tx: Some(tx), state }
    }

    pub fn send(&self, c: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(c);
        }
    }

    pub fn snapshot(&self) -> Option<NowPlaying> {
        self.state.lock().ok().and_then(|g| g.clone())
    }

    /// A QA run's made-up track (the real thread never runs in one).
    pub fn set(&self, np: Option<NowPlaying>) {
        if let Ok(mut g) = self.state.lock() {
            *g = np;
        }
    }
}

/// A player's name from its bus name ("spotify", "firefox.instance_1_23", "chromium.instance42").
pub(crate) fn app_name(id: &str) -> String {
    let l = id.to_lowercase();
    let known = [("spotify", "Spotify"), ("msedge", "Edge"), ("edge", "Edge"), ("chrome", "Chrome"), ("chromium", "Chromium"), ("firefox", "Firefox"), ("vlc", "VLC"), ("mpv", "mpv")];
    if let Some((_, n)) = known.iter().find(|(k, _)| l.contains(k)) {
        return (*n).to_owned();
    }
    String::new()
}

fn props(bus: &Connection, dest: &str, iface: &str) -> HashMap<String, OwnedValue> {
    bus.call_method(Some(dest), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(iface,)).ok().and_then(|m| m.body().deserialize::<HashMap<String, OwnedValue>>().ok()).unwrap_or_default()
}

fn text(v: Option<&OwnedValue>) -> String {
    v.and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok()).unwrap_or_default().trim().to_owned()
}

/// The players on the bus.
fn players(bus: &Connection) -> Vec<String> {
    bus.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .ok()
        .and_then(|m| m.body().deserialize::<Vec<String>>().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.starts_with(PREFIX))
        .collect()
}

/// Wakes the `media` thread on any player's change, and on players coming and going.
fn watch(tx: Sender<Cmd>) {
    for rule in [
        zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus.Properties").and_then(|r| r.member("PropertiesChanged")).and_then(|r| r.path(PATH)).map(|r| r.build()),
        zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface("org.freedesktop.DBus").and_then(|r| r.member("NameOwnerChanged")).and_then(|r| r.arg0ns("org.mpris.MediaPlayer2")).map(|r| r.build()),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        let _ = std::thread::Builder::new().name("mpris".into()).spawn(move || {
            let Ok(bus) = Connection::session() else { return };
            let Ok(it) = zbus::blocking::MessageIterator::for_match_rule(rule, &bus, Some(32)) else { return };
            for m in it {
                if m.is_err() || tx.send(Cmd::Refresh).is_err() {
                    break;
                }
            }
        });
    }
}

fn media_thread(rx: Receiver<Cmd>, tx: Sender<Cmd>, state: Arc<Mutex<Option<NowPlaying>>>, cache: PathBuf, hwnd: isize) {
    let Ok(bus) = Connection::session() else { return };
    watch(tx.clone());
    let _ = tx.send(Cmd::Refresh);
    // The player that played last stays the one shown while it is paused.
    let mut last: Option<String> = None;
    let mut art_for = String::new();
    for cmd in rx {
        let list = players(&bus);
        let status = |n: &String| text(props(&bus, n, PLAYER).get("PlaybackStatus"));
        let playing = list.iter().find(|n| status(n) == "Playing").cloned();
        let current = playing.clone().or_else(|| last.clone().filter(|l| list.contains(l))).or_else(|| list.iter().find(|n| status(n) == "Paused").cloned());
        if playing.is_some() {
            last = playing;
        }
        if let Some(p) = &current {
            let method = match cmd {
                Cmd::Toggle => Some("PlayPause"),
                Cmd::Next => Some("Next"),
                Cmd::Previous => Some("Previous"),
                Cmd::Refresh => None,
            };
            if let Some(m) = method {
                let _ = bus.call_method(Some(p.as_str()), PATH, Some(PLAYER), m, &());
            }
        }
        let now = current.as_ref().and_then(|p| read(&bus, p, &cache, &mut art_for));
        let differs = state.lock().map(|mut g| {
            let d = *g != now;
            *g = now;
            d
        });
        if differs.unwrap_or(false) {
            crate::post(hwnd, WM_MEDIA, 0);
        }
    }
}

/// The player's track, status and art (art saved once per track, the previous file removed).
fn read(bus: &Connection, name: &str, cache: &Path, art_for: &mut String) -> Option<NowPlaying> {
    let p = props(bus, name, PLAYER);
    let meta: HashMap<String, OwnedValue> = p.get("Metadata").and_then(|v| v.try_clone().ok()).and_then(|v| HashMap::try_from(v).ok()).unwrap_or_default();
    let title = text(meta.get("xesam:title"));
    if title.is_empty() {
        return None;
    }
    let artist = meta.get("xesam:artist").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).map(|a| a.join(", ")).unwrap_or_else(|| text(meta.get("xesam:artist")));
    let playing = text(p.get("PlaybackStatus")) == "Playing";
    let id = name.strip_prefix(PREFIX).unwrap_or(name);
    let mut app = app_name(id);
    if app.is_empty() {
        app = text(props(bus, name, "org.mpris.MediaPlayer2").get("Identity"));
    }
    let key = format!("{app}\n{title}\n{artist}");
    let file = cache.join(format!("nowplaying-{:016x}.img", fnv(&key)));
    if *art_for != key {
        if !art_for.is_empty() {
            let _ = std::fs::remove_file(cache.join(format!("nowplaying-{:016x}.img", fnv(art_for))));
        }
        *art_for = key;
        let _ = save_art(&text(meta.get("mpris:artUrl")), &file);
    }
    let art = file.exists().then_some(file);
    Some(NowPlaying { title, artist, app, playing, art })
}

/// The art behind `mpris:artUrl`: a local file (copied, players remove theirs) or a web image.
fn save_art(url: &str, file: &Path) -> Option<()> {
    let bytes = if let Some(path) = url.strip_prefix("file://") {
        std::fs::read(percent_decode(path)).ok()?
    } else {
        let rest = url.strip_prefix("https://")?;
        let (host, path) = rest.split_once('/').map_or((rest, "/".to_owned()), |(h, p)| (h, format!("/{p}")));
        match crate::social::request("GET", host, &path, "", &[]).ok()? {
            (200, body) => body,
            _ => return None,
        }
    };
    if bytes.is_empty() || bytes.len() > 8 << 20 {
        return None;
    }
    let _ = std::fs::create_dir_all(file.parent()?);
    std::fs::write(file, &bytes).ok()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// FNV-1a, for art file names.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_named_from_their_bus_names() {
        assert_eq!(app_name("spotify"), "Spotify");
        assert_eq!(app_name("firefox.instance_1_23"), "Firefox");
        assert_eq!(app_name("chromium.instance4221"), "Chromium");
        assert_eq!(app_name("rhythmbox"), "");
    }

    #[test]
    fn art_paths_are_decoded() {
        assert_eq!(percent_decode("/tmp/My%20Art.png"), "/tmp/My Art.png");
        assert_eq!(percent_decode("/plain"), "/plain");
    }
}
