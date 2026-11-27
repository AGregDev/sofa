//! Discord voice through Discord's local RPC pipe: a port of Sofa 1.x's `src/social.js`.
//! Discord only lets an app control voice for the people who made it, so each person uses a
//! Discord application they made (its Client ID and Client Secret). The secret and the
//! sign-in are kept encrypted for this Windows account (DPAPI); the ones Sofa 1.x saved
//! (Electron's safeStorage) still open, so an upgrade keeps the sign-in (ADR-003). On Linux the
//! pipe is Discord's Unix socket and the secrets are sealed with a key only the person's account
//! can read (`linux/secret.rs`).
//!
//! Everything that talks to Discord runs on the `discord` thread: the UI sends it a `Req`,
//! reads the shared `Voice`, and is woken with `WM_SOCIAL` whenever that changed. Settings
//! keep 1.x's shape (`social.discord`: `clientId`, `secret`, `token`, `recent`).

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::Foundation::*;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::*;
#[cfg(windows)]
use windows::core::*;

/// Posted to Sofa's window when `Voice` changed.
pub const WM_SOCIAL: u32 = 0x8000 + 11;
/// The redirect the person adds to their Discord application (OAuth2 page).
pub const REDIRECT: &str = "http://127.0.0.1";
pub const DEVELOPERS: &str = "https://discord.com/developers/applications";
const SCOPES: [&str; 5] = ["rpc", "rpc.voice.read", "rpc.voice.write", "identify", "guilds"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: String,
    pub avatar: Option<PathBuf>,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// The server's name.
    pub guild: String,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<PathBuf>,
}

/// What Sofa shows of Discord. Written by the `discord` thread, read by the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Voice {
    /// A Client ID and secret are saved.
    pub configured: bool,
    pub ready: bool,
    pub error: Option<String>,
    pub not_running: bool,
    /// Discord shows its "Authorize" prompt (Sofa steps aside for it).
    pub authorizing: bool,
    /// A request is on its way (Connect shows "Waiting for Discord").
    pub busy: bool,
    pub user: Option<String>,
    pub guilds: Vec<Guild>,
    /// The voice channel the person is in.
    pub current: Option<Channel>,
    pub mute: bool,
    pub deaf: bool,
    /// Channels joined recently, with who is in them now (Home's quick-join row).
    pub recent: Vec<Channel>,
    /// A server's voice channels, for its sheet: (server id, channels).
    pub server: Option<(String, Vec<Channel>)>,
    /// `social.discord` to save in `sofa.json` (taken by the UI).
    pub save: Option<Value>,
}

pub enum Req {
    /// Reads Discord's state again; `true` may show Discord's Authorize prompt.
    Refresh(bool),
    /// Saves an application (and its secret, when given), then connects with the prompt.
    Configure { client_id: String, secret: Option<String> },
    /// Joins a voice channel, or leaves voice (`None`).
    Join(Option<String>),
    SetVoice { mute: Option<bool>, deaf: Option<bool> },
    /// Lists a server's voice channels (`Voice::server`).
    Server(String),
    /// Forgets the application and its sign-in (recent channels stay).
    Forget,
}

/// The UI's handle on the `discord` thread.
pub struct Social {
    tx: Option<Sender<Req>>,
    shared: Arc<Mutex<Voice>>,
}

impl Social {
    /// Starts the `discord` thread with `social.discord` from `sofa.json`. Off (no thread,
    /// nothing touched) in a QA run.
    pub fn start(data: PathBuf, cache: PathBuf, hwnd: isize, cfg: Value, enabled: bool) -> Social {
        let shared = Arc::new(Mutex::new(Voice { configured: configured(&cfg), ..Voice::default() }));
        if !enabled {
            return Social { tx: None, shared };
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let state = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("discord".into())
            .spawn(move || Worker { data, icons: cache.join("discord"), hwnd, shared: state, cfg, pipe: None, nonce: 0, ready: false, user: None, watching: None, dispatch: false }.run(rx));
        Social { tx: spawned.is_ok().then_some(tx), shared }
    }

    pub fn send(&self, r: Req) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(r);
        }
    }

    pub fn snapshot(&self) -> Voice {
        self.shared.lock().map(|v| v.clone()).unwrap_or_default()
    }

    /// Settings to save, once.
    pub fn take_save(&self) -> Option<Value> {
        self.shared.lock().ok().and_then(|mut v| v.save.take())
    }
}

fn configured(cfg: &Value) -> bool {
    cfg.get("clientId").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) && cfg.get("secret").and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

// ---------- The discord thread ----------

struct Worker {
    data: PathBuf,
    icons: PathBuf,
    hwnd: isize,
    shared: Arc<Mutex<Voice>>,
    cfg: Value,
    pipe: Option<Pipe>,
    nonce: u64,
    ready: bool,
    user: Option<String>,
    watching: Option<String>,
    /// Discord sent an event since the last refresh.
    dispatch: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<Req>) {
        let mut last_refresh = Instant::now();
        loop {
            // Connected: wake twice a second to read Discord's events. Otherwise sleep until
            // the UI asks for something.
            let req = if self.pipe.is_some() {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(r) => Some(r),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.recv() {
                    Ok(r) => Some(r),
                    Err(_) => return,
                }
            };
            match req {
                Some(r) => {
                    self.publish(|v| v.busy = true);
                    self.handle(r);
                    self.publish(|v| v.busy = false);
                    last_refresh = Instant::now();
                }
                None => {
                    self.read_events();
                    if self.dispatch && last_refresh.elapsed() > Duration::from_millis(300) {
                        self.dispatch = false;
                        self.refresh(false);
                        last_refresh = Instant::now();
                    }
                }
            }
        }
    }

    fn handle(&mut self, r: Req) {
        match r {
            Req::Refresh(interactive) => self.refresh(interactive),
            Req::Configure { client_id, secret } => {
                let id = client_id.trim().to_owned();
                if !id.is_empty() && self.cfg.get("clientId").and_then(Value::as_str) != Some(id.as_str()) {
                    self.obj().insert("clientId".into(), Value::from(id));
                    self.obj().remove("token");
                }
                if let Some(s) = secret.filter(|s| !s.trim().is_empty()) {
                    self.obj().insert("secret".into(), Value::from(seal(s.trim())));
                }
                self.save();
                self.close();
                self.refresh(true);
            }
            Req::Join(id) => {
                match self.call("SELECT_VOICE_CHANNEL", json!({ "channel_id": id, "force": true }), None) {
                    Ok(r) => {
                        if let Some(id) = id {
                            // Remember it for Home's quick-join row.
                            let guild = r.get("guild_id").cloned().unwrap_or(Value::Null);
                            let mut list: Vec<Value> = self.cfg.get("recent").and_then(Value::as_array).cloned().unwrap_or_default();
                            list.retain(|c| c.get("id").and_then(Value::as_str) != Some(id.as_str()));
                            list.insert(0, json!({ "id": id, "guildId": guild }));
                            list.truncate(6);
                            self.obj().insert("recent".into(), Value::Array(list));
                            self.save();
                        }
                    }
                    Err(e) => self.publish(|v| v.error = Some(e)),
                }
                self.refresh(false);
            }
            Req::SetVoice { mute, deaf } => {
                let mut patch = serde_json::Map::new();
                if let Some(m) = mute {
                    patch.insert("mute".into(), Value::Bool(m));
                }
                if let Some(d) = deaf {
                    patch.insert("deaf".into(), Value::Bool(d));
                }
                if let Err(e) = self.call("SET_VOICE_SETTINGS", Value::Object(patch), None) {
                    self.publish(|v| v.error = Some(e));
                }
                self.refresh(false);
            }
            Req::Server(guild) => {
                let list = self.server(&guild);
                self.publish(|v| v.server = Some((guild, list)));
            }
            Req::Forget => {
                self.close();
                let recent = self.cfg.get("recent").cloned();
                self.cfg = recent.map_or_else(|| json!({}), |r| json!({ "recent": r }));
                self.save();
                self.publish(|v| *v = Voice { save: v.save.take(), ..Voice::default() });
            }
        }
    }

    fn obj(&mut self) -> &mut serde_json::Map<String, Value> {
        if !self.cfg.is_object() {
            self.cfg = json!({});
        }
        self.cfg.as_object_mut().expect("INFALLIBLE: made an object just above")
    }

    fn publish(&self, f: impl FnOnce(&mut Voice)) {
        if let Ok(mut v) = self.shared.lock() {
            let before = v.clone();
            f(&mut v);
            if *v == before {
                return;
            }
        }
        crate::post(self.hwnd, WM_SOCIAL, 0);
    }

    fn save(&self) {
        let cfg = self.cfg.clone();
        self.publish(|v| {
            v.configured = configured(&cfg);
            v.save = Some(cfg);
        });
    }

    fn secret(&self, key: &str) -> Option<String> {
        self.cfg.get(key).and_then(Value::as_str).and_then(|s| open(&self.data, s))
    }

    fn close(&mut self) {
        self.pipe = None;
        self.ready = false;
        self.watching = None;
    }

    // ---------- RPC ----------

    /// A command and its answer (events that arrive meanwhile are noted for a refresh).
    fn call(&mut self, cmd: &str, args: Value, evt: Option<&str>) -> std::result::Result<Value, String> {
        self.nonce += 1;
        let nonce = format!("sofa-{}-{}", std::process::id(), self.nonce);
        let mut msg = json!({ "cmd": cmd, "args": args, "nonce": nonce });
        if let Some(e) = evt {
            msg["evt"] = Value::from(e);
        }
        let Some(pipe) = &self.pipe else { return Err("Not connected to Discord".into()) };
        if !pipe.send(1, &msg) {
            self.close();
            return Err("Discord closed the connection".into());
        }
        let wait = if cmd == "AUTHORIZE" { 120_000 } else { 8000 };
        let until = Instant::now() + Duration::from_millis(wait);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("Discord did not answer {cmd}"));
            }
            let Some(pipe) = &self.pipe else { return Err("Not connected to Discord".into()) };
            match pipe.next(left) {
                Err(()) => {
                    self.close();
                    return Err("Discord closed the connection".into());
                }
                Ok(None) => {}
                Ok(Some((2, m))) => {
                    self.close();
                    return Err(m.get("message").and_then(Value::as_str).unwrap_or("Discord closed the connection").to_owned());
                }
                Ok(Some((_, m))) => {
                    if m.get("nonce").and_then(Value::as_str) == Some(nonce.as_str()) {
                        if m.get("evt").and_then(Value::as_str) == Some("ERROR") {
                            return Err(m.pointer("/data/message").and_then(Value::as_str).unwrap_or("Discord error").to_owned());
                        }
                        return Ok(m.get("data").cloned().unwrap_or(Value::Null));
                    }
                    if m.get("cmd").and_then(Value::as_str) == Some("DISPATCH") {
                        self.dispatch = true;
                    }
                }
            }
        }
    }

    /// Discord's events while idle: any of them means something visible may have changed.
    fn read_events(&mut self) {
        let Some(pipe) = &self.pipe else { return };
        loop {
            match pipe.next(Duration::ZERO) {
                Err(()) => {
                    self.close();
                    self.publish(|v| {
                        v.ready = false;
                        v.current = None;
                        v.not_running = true;
                        v.error = Some("Discord is not running".into());
                    });
                    return;
                }
                Ok(None) => return,
                Ok(Some((_, m))) => {
                    if m.get("cmd").and_then(Value::as_str) == Some("DISPATCH") {
                        self.dispatch = true;
                    }
                }
            }
        }
    }

    /// Exchanges an authorization code (or the saved refresh token) for a sign-in.
    fn token(&mut self, code: Option<&str>) -> std::result::Result<String, String> {
        let id = self.cfg.get("clientId").and_then(Value::as_str).unwrap_or("").to_owned();
        let secret = self.secret("secret").unwrap_or_default();
        let mut form = vec![("client_id", id), ("client_secret", secret)];
        match code {
            Some(c) => form.extend([("grant_type", "authorization_code".to_owned()), ("code", c.to_owned()), ("redirect_uri", REDIRECT.to_owned())]),
            None => {
                let refresh = self.cfg.pointer("/token/refresh").and_then(Value::as_str).and_then(|s| open(&self.data, s)).unwrap_or_default();
                form.extend([("grant_type", "refresh_token".to_owned()), ("refresh_token", refresh)]);
            }
        }
        let body: Vec<String> = form.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect();
        let (status, text) = https("POST", "discord.com", "/api/oauth2/token", Some(&body.join("&")))?;
        if status != 200 {
            return Err(format!("Discord refused the sign-in ({status}). Check the client secret and the {REDIRECT} redirect."));
        }
        let t: Value = serde_json::from_slice(&text).map_err(|_| "Discord sent an unreadable sign-in".to_owned())?;
        let access = t.get("access_token").and_then(Value::as_str).unwrap_or("").to_owned();
        let refresh = t.get("refresh_token").and_then(Value::as_str).unwrap_or("");
        let expires = epoch_ms() + t.get("expires_in").and_then(Value::as_f64).unwrap_or(0.0) * 1000.0 - 60_000.0;
        self.obj().insert("token".into(), json!({ "access": seal(&access), "refresh": seal(refresh), "expires": expires }));
        self.save();
        Ok(access)
    }

    /// Connects and signs in: the saved sign-in first, then a silent refresh; Discord's own
    /// "Authorize" prompt only when `interactive` (Connect) and neither works.
    fn connect(&mut self, interactive: bool) -> std::result::Result<(), String> {
        if self.ready && self.pipe.is_some() {
            return Ok(());
        }
        if !configured(&self.cfg) {
            return Err("Discord is not set up".into());
        }
        self.close();
        let id = self.cfg.get("clientId").and_then(Value::as_str).unwrap_or("").to_owned();
        let pipe = Pipe::connect().ok_or("Discord is not running")?;
        if !pipe.send(0, &json!({ "v": 1, "client_id": id })) {
            return Err("Discord is not running".into());
        }
        // The handshake's answer: READY, or a close frame with the reason.
        let until = Instant::now() + Duration::from_secs(6);
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match pipe.next(left) {
                Ok(Some((2, m))) => return Err(m.get("message").and_then(Value::as_str).unwrap_or("Discord refused the connection").to_owned()),
                Ok(Some((_, m))) if m.get("evt").and_then(Value::as_str) == Some("READY") => break,
                Ok(Some(_)) => {}
                Ok(None) if left.is_zero() => return Err("Discord did not answer".into()),
                Ok(None) => {}
                Err(()) => return Err("Discord closed the connection".into()),
            }
        }
        self.pipe = Some(pipe);
        let saved = self.cfg.get("token").cloned();
        let fresh = saved.as_ref().and_then(|t| t.get("expires")).and_then(Value::as_f64).is_some_and(|e| e > epoch_ms());
        let mut auth = None;
        if fresh && let Some(access) = saved.as_ref().and_then(|t| t.get("access")).and_then(Value::as_str).and_then(|s| open(&self.data, s)) {
            auth = self.call("AUTHENTICATE", json!({ "access_token": access }), None).ok();
        }
        if auth.is_none()
            && saved.as_ref().is_some_and(|t| t.get("refresh").is_some())
            && let Ok(access) = self.token(None)
        {
            auth = self.call("AUTHENTICATE", json!({ "access_token": access }), None).ok();
        }
        if auth.is_none() {
            if !interactive {
                self.close();
                return Err("Sign in again: Social, Connect Discord".into());
            }
            self.publish(|v| v.authorizing = true);
            let code = self.call("AUTHORIZE", json!({ "client_id": id, "scopes": SCOPES }), None);
            self.publish(|v| v.authorizing = false);
            let code = code?.get("code").and_then(Value::as_str).unwrap_or("").to_owned();
            let access = self.token(Some(&code))?;
            auth = Some(self.call("AUTHENTICATE", json!({ "access_token": access }), None)?);
        }
        let user = auth.as_ref().and_then(|a| a.get("user"));
        self.user = user.and_then(|u| u.get("global_name").and_then(Value::as_str).or_else(|| u.get("username").and_then(Value::as_str))).map(str::to_owned);
        self.ready = true;
        for evt in ["VOICE_CHANNEL_SELECT", "VOICE_SETTINGS_UPDATE"] {
            let _ = self.call("SUBSCRIBE", json!({}), Some(evt));
        }
        Ok(())
    }

    /// Everything the screens show: servers, the channel you are in, voice settings and the
    /// recent channels, with avatars and server icons saved next to Sofa's other art.
    fn refresh(&mut self, interactive: bool) {
        let configured_now = configured(&self.cfg);
        if let Err(e) = self.connect(interactive) {
            let not_running = e == "Discord is not running";
            self.publish(|v| {
                v.configured = configured_now;
                v.ready = false;
                v.authorizing = false;
                v.current = None;
                v.not_running = not_running;
                v.error = configured_now.then_some(e);
            });
            return;
        }
        let guilds_raw = self.call("GET_GUILDS", json!({}), None).ok().and_then(|d| d.get("guilds").and_then(Value::as_array).cloned()).unwrap_or_default();
        let current_raw = self.call("GET_SELECTED_VOICE_CHANNEL", json!({}), None).ok().filter(|c| c.get("id").is_some());
        let settings = self.call("GET_VOICE_SETTINGS", json!({}), None).ok();
        let names: std::collections::HashMap<String, String> =
            guilds_raw.iter().filter_map(|g| Some((g.get("id")?.as_str()?.to_owned(), g.get("name")?.as_str()?.to_owned()))).collect();
        let guilds: Vec<Guild> = guilds_raw
            .iter()
            .filter_map(|g| {
                let id = g.get("id")?.as_str()?.to_owned();
                let icon = g.get("icon_url").and_then(Value::as_str).and_then(|u| self.image(u));
                Some(Guild { id, name: g.get("name")?.as_str()?.to_owned(), icon })
            })
            .collect();
        let current = current_raw.as_ref().map(|c| self.channel(c, &names));
        if let Some(c) = &current
            && self.watching.as_deref() != Some(c.id.as_str())
        {
            // Follow who joins and leaves the channel you are in.
            self.watching = Some(c.id.clone());
            for evt in ["VOICE_STATE_CREATE", "VOICE_STATE_DELETE", "VOICE_STATE_UPDATE"] {
                let _ = self.call("SUBSCRIBE", json!({ "channel_id": c.id }), Some(evt));
            }
        }
        let ids: Vec<String> = self.cfg.get("recent").and_then(Value::as_array).map(|l| l.iter().filter_map(|c| c.get("id")?.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let mut recent = Vec::new();
        for id in ids {
            if let Ok(ch) = self.call("GET_CHANNEL", json!({ "channel_id": id }), None) {
                recent.push(self.channel(&ch, &names));
            }
        }
        let mute = settings.as_ref().and_then(|s| s.get("mute")).and_then(Value::as_bool).unwrap_or(false);
        let deaf = settings.as_ref().and_then(|s| s.get("deaf")).and_then(Value::as_bool).unwrap_or(false);
        let user = self.user.clone();
        self.publish(|v| {
            v.configured = true;
            v.ready = true;
            v.error = None;
            v.not_running = false;
            v.authorizing = false;
            v.user = user;
            v.guilds = guilds;
            v.current = current;
            v.mute = mute;
            v.deaf = deaf;
            v.recent = recent;
        });
    }

    fn channel(&self, c: &Value, names: &std::collections::HashMap<String, String>) -> Channel {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let members = c
            .get("voice_states")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .map(|v| {
                        let u = v.get("user").cloned().unwrap_or(Value::Null);
                        let id = s(&u, "id");
                        let name = [v.get("nick"), u.get("global_name"), u.get("username")].into_iter().flatten().find_map(Value::as_str).unwrap_or("Someone").to_owned();
                        let avatar = u.get("avatar").and_then(Value::as_str).and_then(|a| self.image(&format!("https://cdn.discordapp.com/avatars/{id}/{a}.png?size=128")));
                        let vs = v.get("voice_state");
                        let muted = ["self_mute", "mute"].iter().any(|k| vs.and_then(|x| x.get(*k)).and_then(Value::as_bool).unwrap_or(false));
                        Member { id, name, avatar, muted }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let guild = c.get("guild_id").and_then(Value::as_str).and_then(|g| names.get(g)).cloned().unwrap_or_default();
        Channel { id: s(c, "id"), name: s(c, "name"), guild, members }
    }

    /// A server's voice channels with who is in each.
    fn server(&mut self, guild: &str) -> Vec<Channel> {
        if self.connect(false).is_err() {
            return Vec::new();
        }
        let list = self.call("GET_CHANNELS", json!({ "guild_id": guild }), None).ok().and_then(|d| d.get("channels").and_then(Value::as_array).cloned()).unwrap_or_default();
        let names = std::collections::HashMap::new();
        let mut out = Vec::new();
        for c in list.iter().filter(|c| matches!(c.get("type").and_then(Value::as_u64), Some(2 | 13))) {
            let id = c.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            let full = self.call("GET_CHANNEL", json!({ "channel_id": id }), None).unwrap_or_else(|_| c.clone());
            out.push(self.channel(&full, &names));
        }
        out
    }

    /// An avatar or server icon from Discord's CDN, saved once (`cache\discord`).
    fn image(&self, url: &str) -> Option<PathBuf> {
        let rest = url.strip_prefix("https://cdn.discordapp.com")?;
        let path = self.icons.join(format!("{}.png", crate::startmenu::hash(url)));
        if path.exists() {
            return Some(path);
        }
        let (status, bytes) = https("GET", "cdn.discordapp.com", rest, None).ok()?;
        if status != 200 || bytes.is_empty() {
            return None;
        }
        let _ = std::fs::create_dir_all(&self.icons);
        std::fs::write(&path, bytes).ok()?;
        Some(path)
    }
}

fn epoch_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

// ---------- The pipe ----------

/// Discord's RPC pipe (`\\.\pipe\discord-ipc-N`; on Linux the Unix socket
/// `$XDG_RUNTIME_DIR/discord-ipc-N`): frames of op (i32), length (i32), JSON.
#[cfg(windows)]
struct Pipe(HANDLE);

#[cfg(target_os = "linux")]
struct Pipe(std::os::unix::net::UnixStream);

#[cfg(windows)]
impl Drop for Pipe {
    fn drop(&mut self) {
        // SAFETY: closing the handle this pipe owns.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
impl Pipe {
    fn connect() -> Option<Pipe> {
        (0..10).find_map(|i| {
            let name = HSTRING::from(format!(r"\\.\pipe\discord-ipc-{i}"));
            // SAFETY: opening a named pipe as a client; the handle is owned by `Pipe`.
            unsafe { CreateFileW(&name, (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_MODE(0), None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) }.ok().map(Pipe)
        })
    }

    fn send(&self, op: i32, v: &Value) -> bool {
        let body = v.to_string().into_bytes();
        let mut buf = Vec::with_capacity(8 + body.len());
        buf.extend(op.to_le_bytes());
        buf.extend((body.len() as i32).to_le_bytes());
        buf.extend(body);
        let mut n = 0u32;
        // SAFETY: writing our buffer to the pipe.
        unsafe { WriteFile(self.0, Some(&buf), Some(&mut n), None) }.is_ok() && n as usize == buf.len()
    }

    /// Bytes waiting in the pipe; Err when Discord closed it.
    fn available(&self) -> std::result::Result<u32, ()> {
        let mut n = 0u32;
        // SAFETY: a query into a local.
        unsafe { windows::Win32::System::Pipes::PeekNamedPipe(self.0, None, 0, None, Some(&mut n), None) }.map(|_| n).map_err(|_| ())
    }

    fn read_exact(&self, n: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; n];
        let mut got = 0;
        while got < n {
            let mut r = 0u32;
            // SAFETY: reading into the rest of our buffer.
            if unsafe { ReadFile(self.0, Some(&mut out[got..]), Some(&mut r), None) }.is_err() || r == 0 {
                return None;
            }
            got += r as usize;
        }
        Some(out)
    }
}

#[cfg(target_os = "linux")]
impl Pipe {
    /// Discord's socket: in the runtime folder, or in its Flatpak's or Snap's.
    fn connect() -> Option<Pipe> {
        let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dirs = [run.clone(), run.join("app/com.discordapp.Discord"), run.join("snap.discord"), run.join(".flatpak/com.discordapp.Discord/xdg-run"), std::env::temp_dir()];
        dirs.iter().find_map(|d| (0..10).find_map(|i| std::os::unix::net::UnixStream::connect(d.join(format!("discord-ipc-{i}"))).ok().map(Pipe)))
    }

    fn send(&self, op: i32, v: &Value) -> bool {
        use std::io::Write;
        let body = v.to_string().into_bytes();
        let mut buf = Vec::with_capacity(8 + body.len());
        buf.extend(op.to_le_bytes());
        buf.extend((body.len() as i32).to_le_bytes());
        buf.extend(body);
        (&self.0).write_all(&buf).is_ok()
    }

    /// Bytes waiting in the socket; Err when Discord closed it.
    fn available(&self) -> std::result::Result<u32, ()> {
        use std::os::fd::AsRawFd;
        let mut p = libc::pollfd { fd: self.0.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd on the stack for the call, no wait.
        let r = unsafe { libc::poll(&mut p, 1, 0) };
        if r < 0 || p.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(());
        }
        if p.revents & (libc::POLLIN | libc::POLLHUP) == 0 {
            return Ok(0);
        }
        let mut n: libc::c_int = 0;
        // SAFETY: FIONREAD writes the byte count into our local.
        let ok = unsafe { libc::ioctl(self.0.as_raw_fd(), libc::FIONREAD, &mut n) } >= 0;
        // Readable with nothing to read: the other end closed.
        if !ok || n <= 0 { Err(()) } else { Ok(n as u32) }
    }

    fn read_exact(&self, n: usize) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut out = vec![0u8; n];
        (&self.0).read_exact(&mut out).ok().map(|_| out)
    }
}

impl Pipe {
    /// The next frame within `wait` (None when none came); Err when the pipe closed.
    fn next(&self, wait: Duration) -> std::result::Result<Option<(i32, Value)>, ()> {
        let until = Instant::now() + wait;
        loop {
            if self.available()? >= 8 {
                let head = self.read_exact(8).ok_or(())?;
                let op = i32::from_le_bytes([head[0], head[1], head[2], head[3]]);
                let len = i32::from_le_bytes([head[4], head[5], head[6], head[7]]).max(0) as usize;
                let body = self.read_exact(len).ok_or(())?;
                return Ok(Some((op, serde_json::from_slice(&body).unwrap_or(Value::Null))));
            }
            if Instant::now() >= until {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

// ---------- HTTPS ----------

/// One HTTPS request to Discord, a form body when given: (status, body).
fn https(method: &str, host: &str, path: &str, form: Option<&str>) -> std::result::Result<(u32, Vec<u8>), String> {
    let headers = if form.is_some() { "Content-Type: application/x-www-form-urlencoded\r\n" } else { "" };
    request(method, host, path, headers, form.unwrap_or("").as_bytes()).map_err(|_| "No connection to Discord".to_owned())
}

/// One HTTPS request (`headers`: lines ending in CRLF): (status, body). Spotify uses it too.
#[cfg(target_os = "linux")]
pub(crate) fn request(method: &str, host: &str, path: &str, headers: &str, body: &[u8]) -> std::result::Result<(u32, Vec<u8>), String> {
    crate::linux::net::request(method, host, path, headers, body)
}

/// One HTTPS request with WinHTTP (`headers`: lines ending in CRLF): (status, body). Spotify
/// uses it too.
#[cfg(windows)]
pub(crate) fn request(method: &str, host: &str, path: &str, headers: &str, body: &[u8]) -> std::result::Result<(u32, Vec<u8>), String> {
    use windows::Win32::Networking::WinHttp::*;
    let offline = || format!("No connection to {host}");
    // SAFETY: WinHTTP handles opened and closed here; buffers outlive the calls.
    unsafe {
        let session = WinHttpOpen(w!("Sofa"), WINHTTP_ACCESS_TYPE(4), PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() {
            return Err(offline());
        }
        let _ = WinHttpSetTimeouts(session, 5000, 5000, 10_000, 10_000);
        let result = (|| {
            let conn = WinHttpConnect(session, &HSTRING::from(host), 443, 0);
            if conn.is_null() {
                return Err(offline());
            }
            let req = WinHttpOpenRequest(conn, &HSTRING::from(method), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_OPEN_REQUEST_FLAGS(0x0080_0000));
            let out = (|| {
                if req.is_null() {
                    return Err(offline());
                }
                let headers: Vec<u16> = headers.encode_utf16().collect();
                let ptr = (!body.is_empty()).then(|| body.as_ptr().cast::<core::ffi::c_void>());
                WinHttpSendRequest(req, (!headers.is_empty()).then_some(&headers[..]), ptr, body.len() as u32, body.len() as u32, 0).map_err(|_| offline())?;
                WinHttpReceiveResponse(req, std::ptr::null_mut()).map_err(|_| offline())?;
                let mut status = 0u32;
                let mut size = 4u32;
                let mut index = 0u32;
                // WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER
                WinHttpQueryHeaders(req, 19 | 0x2000_0000, PCWSTR::null(), Some((&mut status as *mut u32).cast()), &mut size, &mut index).map_err(|_| offline())?;
                let mut data = Vec::new();
                loop {
                    let mut avail = 0u32;
                    if WinHttpQueryDataAvailable(req, &mut avail).is_err() || avail == 0 || data.len() > 4_000_000 {
                        break;
                    }
                    let mut chunk = vec![0u8; avail as usize];
                    let mut read = 0u32;
                    if WinHttpReadData(req, chunk.as_mut_ptr().cast(), avail, &mut read).is_err() || read == 0 {
                        break;
                    }
                    data.extend_from_slice(&chunk[..read as usize]);
                }
                Ok((status, data))
            })();
            if !req.is_null() {
                let _ = WinHttpCloseHandle(req);
            }
            let _ = WinHttpCloseHandle(conn);
            out
        })();
        let _ = WinHttpCloseHandle(session);
        result
    }
}

pub(crate) fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

// ---------- Secrets ----------

/// A secret sealed with Sofa's key, which only this account can read: `aes:` + base64.
#[cfg(target_os = "linux")]
pub fn seal(s: &str) -> String {
    crate::linux::secret::seal(s).unwrap_or_else(|| format!("raw:{s}"))
}

/// A secret saved by `seal`.
#[cfg(target_os = "linux")]
pub fn open(_data: &Path, s: &str) -> Option<String> {
    if let Some(b) = s.strip_prefix("aes:") {
        return crate::linux::secret::open(&b64_decode(b)?);
    }
    s.strip_prefix("raw:").map(str::to_owned)
}

/// A secret encrypted for this Windows account: `dpapi:` + base64.
#[cfg(windows)]
pub fn seal(s: &str) -> String {
    protect(s.as_bytes(), true).map_or_else(|| format!("raw:{s}"), |b| format!("dpapi:{}", b64_encode(&b)))
}

/// A secret saved by `seal`, or by Sofa 1.x (`enc:`: Electron's safeStorage, AES-GCM with a
/// key kept in Sofa's `Local State`, itself protected for this Windows account).
#[cfg(windows)]
pub fn open(data: &Path, s: &str) -> Option<String> {
    if let Some(b) = s.strip_prefix("dpapi:") {
        return String::from_utf8(protect(&b64_decode(b)?, false)?).ok();
    }
    if let Some(r) = s.strip_prefix("raw:") {
        return Some(r.to_owned());
    }
    let blob = b64_decode(s.strip_prefix("enc:")?)?;
    let rest = blob.strip_prefix(b"v10")?;
    if rest.len() < 12 + 16 {
        return None;
    }
    let key = electron_key(data)?;
    let (nonce, sealed) = rest.split_at(12);
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    String::from_utf8(aes_gcm_open(&key, nonce, ct, tag)?).ok()
}

#[cfg(windows)]
fn electron_key(data: &Path) -> Option<Vec<u8>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(data.join("Local State")).ok()?).ok()?;
    let k = b64_decode(v.pointer("/os_crypt/encrypted_key")?.as_str()?)?;
    protect(k.strip_prefix(b"DPAPI")?, false)
}

#[cfg(windows)]
fn protect(input: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::*;
    let blob = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr().cast_mut() };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: DPAPI reads `blob` and allocates `out`, which is copied and freed here.
    unsafe {
        let r = if encrypt {
            CryptProtectData(&blob, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&blob, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        r.ok()?;
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
        Some(v)
    }
}

#[cfg(windows)]
fn aes_gcm_open(key: &[u8], nonce: &[u8], ct: &[u8], tag: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::*;
    // SAFETY: CNG handles opened and closed here; every buffer outlives the calls using it.
    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).is_err() {
            return None;
        }
        let mode: Vec<u8> = "ChainingModeGCM\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut out = None;
        if BCryptSetProperty(BCRYPT_HANDLE(alg.0), BCRYPT_CHAINING_MODE, &mode, 0).is_ok() {
            let mut hkey = BCRYPT_KEY_HANDLE::default();
            if BCryptGenerateSymmetricKey(alg, &mut hkey, None, key, 0).is_ok() {
                let mut nonce = nonce.to_vec();
                let mut tag = tag.to_vec();
                let info = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
                    cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
                    dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
                    pbNonce: nonce.as_mut_ptr(),
                    cbNonce: nonce.len() as u32,
                    pbTag: tag.as_mut_ptr(),
                    cbTag: tag.len() as u32,
                    ..Default::default()
                };
                let mut plain = vec![0u8; ct.len()];
                let mut n = 0u32;
                if BCryptDecrypt(hkey, Some(ct), Some((&info as *const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO).cast()), None, Some(&mut plain), &mut n, BCRYPT_FLAGS(0)).is_ok() {
                    plain.truncate(n as usize);
                    out = Some(plain);
                }
                let _ = BCryptDestroyKey(hkey);
            }
        }
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        out
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn b64_encode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|&x| x == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            assert_eq!(b64_decode(&b64_encode(s.as_bytes())).as_deref(), Some(s.as_bytes()));
        }
        assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn secrets_open_what_seal_wrote() {
        let sealed = seal("client-secret-123");
        assert!(sealed.starts_with(if cfg!(windows) { "dpapi:" } else { "aes:" }));
        assert_eq!(open(Path::new("."), &sealed).as_deref(), Some("client-secret-123"));
        assert_eq!(open(Path::new("."), "raw:plain").as_deref(), Some("plain"));
        assert_eq!(open(Path::new("."), "nonsense"), None);
    }

    #[test]
    fn form_values_are_encoded() {
        assert_eq!(url_encode("http://127.0.0.1"), "http%3A%2F%2F127.0.0.1");
        assert_eq!(url_encode("a b+c"), "a%20b%2Bc");
    }
}
