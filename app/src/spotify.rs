//! Spotify from the controller: Liked Songs, your playlists and saved albums as tiles, played
//! on this PC. Spotify has no TV interface a PC browser can open and its own app is built for a
//! mouse, so Sofa shows the library itself and starts playback in the Spotify app through
//! Spotify's Web API.
//!
//! Spotify only lets an app control playback for the people who made it (Development Mode;
//! since February 2026 the app's owner needs Premium), so each person uses a Spotify app they
//! made: its Client ID and the redirect `http://127.0.0.1:7632/callback`. The sign-in is PKCE in
//! the browser, answered on 127.0.0.1 at that port: Spotify's dashboard refuses a loopback
//! redirect without a port ("not secure"), although its documentation says to register one that
//! way. Sofa never sees the password and keeps the refresh token encrypted for this Windows
//! account (DPAPI).
//!
//! Everything that talks to Spotify runs on the `spotify` thread: the UI sends a `Req`, reads
//! the shared `Music` and is woken with `WM_MUSIC`. Settings: `music.spotify` in `sofa.json`
//! (`clientId`, `refresh`, `user`).

use crate::social::{b64_encode, open, request, seal, url_encode};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Posted to Sofa's window when `Music` changed.
pub const WM_MUSIC: u32 = 0x8000 + 15;
/// The port Sofa answers Spotify's sign-in on (SOFA on a phone keypad), below Windows' range of
/// temporary ports.
const PORT: u16 = 7632;
/// The redirect the person adds to their Spotify app.
pub const REDIRECT: &str = "http://127.0.0.1:7632/callback";
pub const DASHBOARD: &str = "https://developer.spotify.com/dashboard";
const SCOPES: &str = "user-read-private user-read-playback-state user-modify-playback-state playlist-read-private playlist-read-collaborative user-library-read";
const API: &str = "api.spotify.com";
const ACCOUNTS: &str = "accounts.spotify.com";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Liked,
    Playlist,
    Album,
}

/// Something to play: Liked Songs, a playlist or an album.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub kind: Kind,
    /// `spotify:playlist:…`, `spotify:album:…`; Liked Songs is `spotify:user:<id>:collection`.
    pub uri: String,
    pub name: String,
    pub songs: Option<u32>,
    /// A playlist's owner, an album's artists.
    pub by: String,
    pub cover: Option<PathBuf>,
}

/// What Sofa shows of Spotify. Written by the `spotify` thread, read by the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Music {
    /// A sign-in is saved.
    pub configured: bool,
    /// The library was read.
    pub ready: bool,
    pub busy: bool,
    /// Waiting for the person to approve Sofa in the browser.
    pub signing_in: bool,
    pub error: Option<String>,
    pub user: Option<String>,
    /// Liked Songs, the playlists, then the saved albums.
    pub library: Vec<Pick>,
    /// The URI being started (its hero says so).
    pub starting: Option<String>,
    /// Taken by the UI: a page to open (the sign-in), a notice for a toast, a request to come
    /// back to the front (after the browser or Spotify took it), settings to save.
    pub open: Option<String>,
    pub notice: Option<(String, String)>,
    pub come_back: bool,
    pub save: Option<Value>,
}

/// What the UI takes from `Music` once: a page to open, a notice, coming back to the front,
/// settings to save.
#[derive(Default)]
pub struct Taken {
    pub open: Option<String>,
    pub notice: Option<(String, String)>,
    pub come_back: bool,
    pub save: Option<Value>,
}

pub enum Req {
    /// Reads the library again.
    Refresh,
    /// Signs in with this Client ID (in the browser), then reads the library.
    Connect(String),
    /// Plays something on this PC, in order or shuffled.
    Play { uri: String, name: String, shuffle: bool },
    /// Forgets the sign-in and the app.
    Forget,
}

/// The UI's handle on the `spotify` thread.
pub struct Spotify {
    tx: Option<Sender<Req>>,
    shared: Arc<Mutex<Music>>,
}

impl Spotify {
    /// Starts the `spotify` thread with `music.spotify` from `sofa.json`. Off (no thread, nothing
    /// touched) in a QA run, which shows a made-up library with `set`.
    pub fn start(data: PathBuf, cache: PathBuf, hwnd: isize, cfg: Value, enabled: bool) -> Spotify {
        let user = cfg.get("user").and_then(Value::as_str).map(str::to_owned);
        let shared = Arc::new(Mutex::new(Music { configured: configured(&cfg), user, ..Music::default() }));
        if !enabled {
            return Spotify { tx: None, shared };
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let state = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("spotify".into())
            .spawn(move || Worker { data, covers: cache.join("spotify"), hwnd, shared: state, cfg, access: None, user_id: None }.run(rx));
        Spotify { tx: spawned.is_ok().then_some(tx), shared }
    }

    pub fn send(&self, r: Req) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(r);
        }
    }

    pub fn snapshot(&self) -> Music {
        self.shared.lock().map(|m| m.clone()).unwrap_or_default()
    }

    /// The one-off parts, taken once.
    pub fn take(&self) -> Taken {
        self.shared.lock().map(|mut m| Taken { open: m.open.take(), notice: m.notice.take(), come_back: std::mem::take(&mut m.come_back), save: m.save.take() }).unwrap_or_default()
    }

    /// A QA run's made-up library (the thread never runs in one).
    pub fn set(&self, m: Music) {
        if let Ok(mut g) = self.shared.lock() {
            *g = m;
        }
    }
}

fn configured(cfg: &Value) -> bool {
    cfg.get("clientId").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) && cfg.get("refresh").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// A Client ID as Spotify shows it: 32 letters and digits.
pub fn client_id_ok(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// Random bytes from the kernel.
#[cfg(target_os = "linux")]
fn random(n: usize) -> Vec<u8> {
    crate::linux::secret::random(n)
}

#[cfg(target_os = "linux")]
fn sha256(data: &[u8]) -> [u8; 32] {
    crate::linux::secret::sha256(data)
}

/// Random bytes from Windows' system generator.
#[cfg(windows)]
fn random(n: usize) -> Vec<u8> {
    use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    let mut b = vec![0u8; n];
    // SAFETY: fills our own buffer.
    let _ = unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    b
}

#[cfg(windows)]
fn sha256(data: &[u8]) -> [u8; 32] {
    use windows::Win32::Security::Cryptography::{BCRYPT_SHA256_ALG_HANDLE, BCryptHash};
    let mut out = [0u8; 32];
    // SAFETY: Windows' built-in SHA-256 handle; input and output are our own buffers.
    let _ = unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) };
    out
}

/// Base64 for URLs, without padding (PKCE's code challenge).
pub(crate) fn b64url(b: &[u8]) -> String {
    b64_encode(b).trim_end_matches('=').replace('+', "-").replace('/', "_")
}

/// A PKCE code verifier: 64 characters Spotify accepts.
fn verifier() -> String {
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    random(64).iter().map(|b| SET[(*b as usize) % SET.len()] as char).collect()
}

/// `%2F` and `+` in a URL query, decoded.
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 2 < b.len()).then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match (hex, b[i]) {
            (Some(v), _) => {
                out.push(v);
                i += 3;
                continue;
            }
            (None, b'+') => out.push(b' '),
            (None, c) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `?a=b&c=d` part of a request line, decoded.
pub(crate) fn query(target: &str) -> Vec<(String, String)> {
    target
        .split_once('?')
        .map(|(_, q)| q.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').map_or((url_decode(p), String::new()), |(k, v)| (url_decode(k), url_decode(v)))).collect())
        .unwrap_or_default()
}

/// "42 songs", "1 song".
pub fn songs_text(n: u32) -> String {
    if n == 1 { "1 song".into() } else { format!("{n} songs") }
}

// ---------- The spotify thread ----------

struct Worker {
    data: PathBuf,
    covers: PathBuf,
    hwnd: isize,
    shared: Arc<Mutex<Music>>,
    cfg: Value,
    /// The access token and when it expires (epoch ms); never saved.
    access: Option<(String, f64)>,
    user_id: Option<String>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Req>) {
        // SAFETY: COM for this thread, for ShellExecute (starting Spotify).
        #[cfg(windows)]
        let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
        if configured(&self.cfg) {
            self.publish(|m| m.busy = true);
            self.refresh();
            self.publish(|m| m.busy = false);
        }
        for r in rx {
            self.publish(|m| m.busy = true);
            self.handle(r);
            self.publish(|m| m.busy = false);
        }
    }

    fn handle(&mut self, r: Req) {
        match r {
            Req::Refresh => self.refresh(),
            Req::Connect(id) => match self.sign_in(&id) {
                Ok(()) => {
                    self.refresh();
                    let who = self.snapshot().user.unwrap_or_else(|| "your account".into());
                    self.publish(|m| {
                        m.notice = Some(("Spotify connected".into(), format!("Signed in as {who}.")));
                        m.come_back = true;
                    });
                }
                Err(e) => self.publish(|m| {
                    m.signing_in = false;
                    m.error = Some(e);
                    m.come_back = true;
                }),
            },
            Req::Play { uri, name, shuffle } => {
                self.publish(|m| m.starting = Some(uri.clone()));
                let r = self.play(&uri, shuffle);
                self.publish(|m| {
                    m.starting = None;
                    m.notice = Some(match r {
                        Ok(()) => (format!("Playing {name}"), "Spotify, on this PC".into()),
                        Err(e) => (format!("Couldn't play {name}"), e),
                    });
                });
            }
            Req::Forget => {
                self.cfg = json!({});
                self.access = None;
                self.user_id = None;
                self.save();
                self.publish(|m| *m = Music { save: m.save.take(), ..Music::default() });
            }
        }
    }

    fn snapshot(&self) -> Music {
        self.shared.lock().map(|m| m.clone()).unwrap_or_default()
    }

    fn publish(&self, f: impl FnOnce(&mut Music)) {
        if let Ok(mut m) = self.shared.lock() {
            let before = m.clone();
            f(&mut m);
            if *m == before {
                return;
            }
        }
        crate::post(self.hwnd, WM_MUSIC, 0);
    }

    fn obj(&mut self) -> &mut serde_json::Map<String, Value> {
        if !self.cfg.is_object() {
            self.cfg = json!({});
        }
        self.cfg.as_object_mut().expect("INFALLIBLE: made an object just above")
    }

    fn save(&self) {
        let cfg = self.cfg.clone();
        self.publish(|m| {
            m.configured = configured(&cfg);
            m.save = Some(cfg);
        });
    }

    // ---------- Sign-in ----------

    /// PKCE in the browser: Sofa listens on 127.0.0.1 for Spotify's answer, then exchanges the
    /// code for a refresh token.
    fn sign_in(&mut self, client_id: &str) -> Result<(), String> {
        let id = client_id.trim().to_owned();
        if !client_id_ok(&id) {
            return Err("The Client ID is 32 letters and digits, from your Spotify app's settings.".into());
        }
        // The port in the registered redirect. If another program holds it, any free port:
        // Spotify does not compare a loopback redirect's port (2.2.1 signed in on a random port
        // against a redirect registered with 7632); only its dashboard insists on one.
        let listener = TcpListener::bind(("127.0.0.1", PORT)).or_else(|_| TcpListener::bind("127.0.0.1:0")).map_err(|_| "Sofa couldn't listen for Spotify's answer.".to_owned())?;
        let port = listener.local_addr().map_err(|_| "Sofa couldn't listen for Spotify's answer.".to_owned())?.port();
        crate::log(&format!("spotify sign-in answered on port {port}"));
        let redirect = format!("http://127.0.0.1:{port}/callback");
        let verifier = verifier();
        let state = b64url(&random(12));
        let url = format!(
            "https://{ACCOUNTS}/authorize?response_type=code&client_id={}&scope={}&code_challenge_method=S256&code_challenge={}&redirect_uri={}&state={}",
            url_encode(&id),
            url_encode(SCOPES),
            b64url(&sha256(verifier.as_bytes())),
            url_encode(&redirect),
            url_encode(&state)
        );
        self.publish(|m| {
            m.signing_in = true;
            m.error = None;
            m.open = Some(url);
        });
        let code = wait_for_code(&listener, &state, Duration::from_secs(240));
        self.publish(|m| m.signing_in = false);
        let code = code?;
        let form = [("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", redirect.as_str()), ("client_id", id.as_str()), ("code_verifier", verifier.as_str())];
        let t = token_request(&form)?;
        let access = t.get("access_token").and_then(Value::as_str).unwrap_or("").to_owned();
        let refresh = t.get("refresh_token").and_then(Value::as_str).unwrap_or("").to_owned();
        if access.is_empty() || refresh.is_empty() {
            return Err("Spotify sent an unreadable sign-in. Try again.".into());
        }
        self.access = Some((access, epoch_ms() + t.get("expires_in").and_then(Value::as_f64).unwrap_or(3600.0) * 1000.0 - 60_000.0));
        self.obj().insert("clientId".into(), Value::from(id));
        self.obj().insert("refresh".into(), Value::from(seal(&refresh)));
        self.save();
        Ok(())
    }

    /// A fresh access token: the one in memory, else one from the saved refresh token.
    fn token(&mut self) -> Result<String, String> {
        if let Some((t, until)) = &self.access
            && *until > epoch_ms()
        {
            return Ok(t.clone());
        }
        let id = self.cfg.get("clientId").and_then(Value::as_str).unwrap_or("").to_owned();
        let refresh = self.cfg.get("refresh").and_then(Value::as_str).and_then(|s| open(&self.data, s)).ok_or("Connect Spotify first: Music, Connect Spotify.")?;
        let t = token_request(&[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", id.as_str())])?;
        let access = t.get("access_token").and_then(Value::as_str).unwrap_or("").to_owned();
        if access.is_empty() {
            return Err("Spotify sent an unreadable sign-in. Sign in again from Music.".into());
        }
        // Spotify may hand out a new refresh token; the old one then stops working.
        if let Some(r) = t.get("refresh_token").and_then(Value::as_str).filter(|r| !r.is_empty() && *r != refresh) {
            self.obj().insert("refresh".into(), Value::from(seal(r)));
            self.save();
        }
        self.access = Some((access.clone(), epoch_ms() + t.get("expires_in").and_then(Value::as_f64).unwrap_or(3600.0) * 1000.0 - 60_000.0));
        Ok(access)
    }

    /// One Web API call with the access token (signed in again once when it was refused).
    fn api(&mut self, method: &str, path: &str, body: Option<&Value>) -> Result<(u32, Value), String> {
        for attempt in 0..2 {
            let token = self.token()?;
            let mut headers = format!("Authorization: Bearer {token}\r\n");
            let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
            if body.is_some() {
                headers.push_str("Content-Type: application/json\r\n");
            }
            let (status, data) = request(method, API, path, &headers, &bytes).map_err(|_| "No connection to Spotify.".to_owned())?;
            if status == 401 && attempt == 0 {
                self.access = None;
                continue;
            }
            if status == 429 {
                return Err("Spotify asks Sofa to slow down. Try again in a minute.".into());
            }
            return Ok((status, serde_json::from_slice(&data).unwrap_or(Value::Null)));
        }
        Err("Spotify refused the sign-in. Sign in again from Music.".into())
    }

    // ---------- The library ----------

    fn refresh(&mut self) {
        if !configured(&self.cfg) {
            self.publish(|m| {
                m.configured = false;
                m.ready = false;
            });
            return;
        }
        let me = match self.api("GET", "/v1/me", None) {
            Ok((200, v)) => v,
            Ok((status, v)) => return self.fail(status, &v),
            Err(e) => return self.publish(|m| m.error = Some(e)),
        };
        let user = me.get("display_name").and_then(Value::as_str).filter(|s| !s.is_empty()).or_else(|| me.get("id").and_then(Value::as_str)).map(str::to_owned);
        self.user_id = me.get("id").and_then(Value::as_str).map(str::to_owned);
        if user.is_some() && self.cfg.get("user").and_then(Value::as_str) != user.as_deref() {
            self.obj().insert("user".into(), Value::from(user.clone()));
            self.save();
        }
        let mut library = Vec::new();
        if let (Ok((200, v)), Some(uid)) = (self.api("GET", "/v1/me/tracks?limit=1", None), self.user_id.clone()) {
            let n = v.get("total").and_then(Value::as_u64).unwrap_or(0) as u32;
            if n > 0 {
                library.push(Pick { kind: Kind::Liked, uri: format!("spotify:user:{uid}:collection"), name: "Liked Songs".into(), songs: Some(n), by: String::new(), cover: None });
            }
        }
        if let Ok((200, v)) = self.api("GET", "/v1/me/playlists?limit=50", None) {
            for p in v.get("items").and_then(Value::as_array).into_iter().flatten() {
                let (Some(uri), Some(name)) = (p.get("uri").and_then(Value::as_str), p.get("name").and_then(Value::as_str)) else { continue };
                // Playlists call their songs `items` since February 2026 (`tracks` before).
                let songs = ["items", "tracks"].iter().find_map(|k| p.get(*k).and_then(|t| t.get("total")).and_then(Value::as_u64)).map(|n| n as u32);
                let by = p.pointer("/owner/display_name").and_then(Value::as_str).unwrap_or("").to_owned();
                let cover = self.cover(p.get("images"));
                library.push(Pick { kind: Kind::Playlist, uri: uri.into(), name: name.into(), songs, by, cover });
            }
        }
        if let Ok((200, v)) = self.api("GET", "/v1/me/albums?limit=50", None) {
            for a in v.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(|a| a.get("album")) {
                let (Some(uri), Some(name)) = (a.get("uri").and_then(Value::as_str), a.get("name").and_then(Value::as_str)) else { continue };
                let by = a.get("artists").and_then(Value::as_array).map(|l| l.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let songs = a.get("total_tracks").and_then(Value::as_u64).map(|n| n as u32);
                let cover = self.cover(a.get("images"));
                library.push(Pick { kind: Kind::Album, uri: uri.into(), name: name.into(), songs, by, cover });
            }
        }
        self.publish(|m| {
            m.configured = true;
            m.ready = true;
            m.error = None;
            m.user = user;
            m.library = library;
        });
    }

    /// Explains a refused call (Premium, a removed app, a sign-in that expired).
    fn fail(&mut self, status: u32, v: &Value) {
        let said = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("");
        let msg = if said.to_lowercase().contains("premium") {
            "Spotify needs Premium on the account that made the app.".to_owned()
        } else if status == 403 {
            "Spotify refused Sofa. Check that your Spotify account is listed in the app's User Management.".to_owned()
        } else {
            format!("Spotify answered {status}. Sign in again from Music.")
        };
        self.publish(|m| {
            m.ready = false;
            m.error = Some(msg);
        });
    }

    /// The image closest to 300 pixels wide, saved once (`cache\spotify`).
    fn cover(&self, images: Option<&Value>) -> Option<PathBuf> {
        let list = images?.as_array()?;
        let url = list.iter().min_by_key(|i| (i.get("width").and_then(Value::as_i64).unwrap_or(640) - 300).abs())?.get("url")?.as_str()?;
        let path = self.covers.join(format!("{}.jpg", crate::startmenu::hash(url)));
        if path.exists() {
            return Some(path);
        }
        let (host, rest) = url.strip_prefix("https://")?.split_once('/')?;
        let (status, bytes) = request("GET", host, &format!("/{rest}"), "", &[]).ok()?;
        if status != 200 || bytes.is_empty() {
            return None;
        }
        let _ = std::fs::create_dir_all(&self.covers);
        std::fs::write(&path, bytes).ok()?;
        Some(path)
    }

    // ---------- Playing ----------

    /// Plays `uri` on this PC: Spotify's app here is the device (started when it is not
    /// running), shuffle set as asked.
    fn play(&mut self, uri: &str, shuffle: bool) -> Result<(), String> {
        let device = match self.device()? {
            Some(d) => d,
            None => {
                start_app();
                self.publish(|m| m.come_back = true);
                self.wait_for_device(Duration::from_secs(20))?
            }
        };
        let _ = self.api("PUT", &format!("/v1/me/player/shuffle?state={shuffle}&device_id={}", url_encode(&device)), Some(&json!({})));
        let (status, v) = self.api("PUT", &format!("/v1/me/player/play?device_id={}", url_encode(&device)), Some(&json!({ "context_uri": uri })))?;
        match status {
            200..=299 => {
                self.publish(|m| m.come_back = true);
                Ok(())
            }
            403 if v.to_string().to_lowercase().contains("premium") => Err("Playing from Sofa needs Spotify Premium.".into()),
            404 => Err("Spotify on this PC didn't answer. Open Spotify once, then try again.".into()),
            _ => Err(v.pointer("/error/message").and_then(Value::as_str).map_or_else(|| format!("Spotify answered {status}."), |m| format!("Spotify said: {m}."))),
        }
    }

    /// This PC's Spotify app as a Connect device: named after the computer, else any computer.
    fn device(&mut self) -> Result<Option<String>, String> {
        let (status, v) = self.api("GET", "/v1/me/player/devices", None)?;
        if status != 200 {
            return Ok(None);
        }
        #[cfg(windows)]
        let pc = std::env::var("COMPUTERNAME").unwrap_or_default().to_lowercase();
        #[cfg(target_os = "linux")]
        let pc = crate::linux::proc::hostname().to_lowercase();
        let list: Vec<&Value> = v.get("devices").and_then(Value::as_array).map(|l| l.iter().collect()).unwrap_or_default();
        let computer = |d: &Value| d.get("type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("computer"));
        let computers = || list.iter().copied().filter(|d| computer(d));
        let pick = computers()
            .find(|d| d.get("name").and_then(Value::as_str).is_some_and(|n| n.to_lowercase() == pc))
            .or_else(|| computers().find(|d| d.get("is_active").and_then(Value::as_bool) == Some(true)))
            .or_else(|| computers().next());
        Ok(pick.and_then(|d| d.get("id").and_then(Value::as_str)).map(str::to_owned))
    }

    fn wait_for_device(&mut self, wait: Duration) -> Result<String, String> {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(800));
            if let Some(d) = self.device()? {
                return Ok(d);
            }
        }
        Err("Spotify didn't start on this PC. Open it once and sign in, then try again.".into())
    }
}

/// Spotify's app on this PC: the installed one (its package, Flatpak or Snap), else whatever
/// opens `spotify:`.
#[cfg(target_os = "linux")]
fn start_app() {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let found = crate::linux::desktop::which("spotify")
        .or_else(|| [home.join(".local/share/flatpak/exports/bin/com.spotify.Client"), PathBuf::from("/var/lib/flatpak/exports/bin/com.spotify.Client"), PathBuf::from("/snap/bin/spotify")].into_iter().find(|p| p.is_file()));
    match found {
        Some(exe) => {
            crate::platform::spawn_detached(std::process::Command::new(exe));
        }
        None => {
            crate::platform::open("spotify:");
        }
    }
}

/// Spotify's app on this PC: the installed one minimized, else whatever opens `spotify:`.
#[cfg(windows)]
fn start_app() {
    let exe = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Spotify").join("Spotify.exe");
    if exe.exists() {
        let _ = std::process::Command::new(exe).arg("--minimized").spawn();
    } else {
        crate::platform::open("spotify:");
    }
}

/// POSTs to Spotify's token endpoint.
fn token_request(form: &[(&str, &str)]) -> Result<Value, String> {
    let body: Vec<String> = form.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect();
    let (status, data) = request("POST", ACCOUNTS, "/api/token", "Content-Type: application/x-www-form-urlencoded\r\n", body.join("&").as_bytes()).map_err(|_| "No connection to Spotify.".to_owned())?;
    let v: Value = serde_json::from_slice(&data).unwrap_or(Value::Null);
    if status == 200 {
        return Ok(v);
    }
    Err(match v.get("error").and_then(Value::as_str) {
        Some("invalid_grant") => "Spotify's sign-in ran out. Sign in again from Music.".into(),
        Some("invalid_client") => "Spotify doesn't know that Client ID. Check it in your app's settings.".into(),
        _ => format!("Spotify refused the sign-in ({status}). Check the {REDIRECT} redirect in your app."),
    })
}

/// Waits for the browser to come back to 127.0.0.1 with Spotify's answer, and thanks it.
fn wait_for_code(listener: &TcpListener, state: &str, wait: Duration) -> Result<String, String> {
    let _ = listener.set_nonblocking(true);
    let until = Instant::now() + wait;
    while Instant::now() < until {
        let Ok((mut stream, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(120));
            continue;
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).unwrap_or(0);
        let head = String::from_utf8_lossy(&buf[..n]);
        let target = head.split_whitespace().nth(1).unwrap_or("");
        if !target.starts_with("/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            continue;
        }
        let q = query(target);
        let get = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        let (ok, result) = match (get("code"), get("state"), get("error")) {
            (Some(code), Some(s), _) if s == state => (true, Ok(code)),
            (_, _, Some(e)) if e == "access_denied" => (false, Err("The Spotify sign-in was cancelled.".to_owned())),
            (_, _, Some(e)) => (false, Err(format!("Spotify refused the sign-in ({e})."))),
            _ => (false, Err("Spotify's answer didn't match. Try again.".to_owned())),
        };
        let text = if ok { "Sofa is connected to Spotify. You can close this tab and pick up the controller." } else { "The sign-in didn't finish. Go back to Sofa and try again." };
        let page = format!(
            "<!doctype html><meta charset=\"utf-8\"><title>Sofa</title><body style=\"margin:0;height:100vh;display:grid;place-items:center;background:#161513;color:#efe9de;font:22px/1.5 'Segoe UI',system-ui,sans-serif\"><p>{text}</p></body>"
        );
        let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes());
        return result;
    }
    Err("The Spotify sign-in timed out. Try again.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        let challenge = b64url(&sha256(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn verifiers_use_only_allowed_characters() {
        let v = verifier();
        assert_eq!(v.len(), 64);
        assert!(v.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)));
        assert_ne!(v, verifier());
    }

    #[test]
    fn redirect_names_the_port_sofa_listens_on() {
        // The dashboard refuses a loopback redirect without a port.
        assert_eq!(REDIRECT, format!("http://127.0.0.1:{PORT}/callback"));
    }

    #[test]
    fn callback_queries_are_decoded() {
        let q = query("/callback?code=AQB%2Fx-1&state=abc_123&extra");
        assert_eq!(q[0], ("code".into(), "AQB/x-1".into()));
        assert_eq!(q[1], ("state".into(), "abc_123".into()));
        assert_eq!(q[2], ("extra".into(), String::new()));
        assert!(query("/callback").is_empty());
    }

    #[test]
    fn client_ids_are_32_letters_and_digits() {
        assert!(client_id_ok("0123456789abcdef0123456789ABCDEF"));
        assert!(!client_id_ok("0123456789abcdef"));
        assert!(!client_id_ok("0123456789abcdef0123456789abcde!"));
    }
}
