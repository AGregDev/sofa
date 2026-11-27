//! What is playing: Spotify, a browser tab, any player that shows in Windows' media flyout.
//! Read from Windows' media session manager on the `media` thread, which sleeps until Windows
//! reports a change; the quick menu shows the track and its art, and the controller plays,
//! pauses and skips through the same session. Sofa never has to drive the player's own window,
//! so a player built for a mouse (Spotify) still works from the couch.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Storage::Streams::{DataReader, IInputStream};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::Interface;

/// WM_APP + 14: what is playing changed.
pub const WM_MEDIA: u32 = 0x8000 + 14;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// The app playing it ("Spotify", "Chrome").
    pub app: String,
    pub playing: bool,
    /// The album or video art, saved in Sofa's cache.
    pub art: Option<PathBuf>,
}

pub enum Cmd {
    Toggle,
    Next,
    Previous,
    /// Windows reported a change: read the session again.
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

/// A player's name from its app id: "Spotify.exe", "SpotifyAB.SpotifyMusic_…!Spotify",
/// "Chrome", "MSEdge", Firefox's hashed id.
pub(crate) fn app_name(id: &str) -> String {
    let l = id.to_lowercase();
    let known = [("spotify", "Spotify"), ("msedge", "Edge"), ("chrome", "Chrome"), ("firefox", "Firefox"), ("308046b0af4a39cb", "Firefox"), ("vlc", "VLC"), ("zunemusic", "Media Player")];
    if let Some((_, n)) = known.iter().find(|(k, _)| l.contains(k)) {
        return (*n).to_owned();
    }
    let tail = id.rsplit(['!', '\\']).next().unwrap_or(id);
    tail.strip_suffix(".exe").or_else(|| tail.strip_suffix(".EXE")).unwrap_or(tail).to_owned()
}

fn media_thread(rx: Receiver<Cmd>, tx: Sender<Cmd>, state: Arc<Mutex<Option<NowPlaying>>>, cache: PathBuf, hwnd: isize) {
    // SAFETY: COM for this thread; Windows calls the change handlers on its own threads.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let Ok(manager) = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.join()) else { return };
    let changed = |tx: &Sender<Cmd>| {
        let tx = tx.clone();
        move || {
            let _ = tx.send(Cmd::Refresh);
        }
    };
    let on = changed(&tx);
    let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        on();
        Ok(())
    }));
    // The session being watched, with its two change subscriptions.
    let mut watched: Option<(GlobalSystemMediaTransportControlsSession, i64, i64)> = None;
    let mut art_for = String::new();
    let _ = tx.send(Cmd::Refresh);
    for cmd in rx {
        let session = manager.GetCurrentSession().ok();
        if let Some(s) = &session {
            let _ = match cmd {
                Cmd::Toggle => s.TryTogglePlayPauseAsync().and_then(|o| o.join()),
                Cmd::Next => s.TrySkipNextAsync().and_then(|o| o.join()),
                Cmd::Previous => s.TrySkipPreviousAsync().and_then(|o| o.join()),
                Cmd::Refresh => Ok(true),
            };
        }
        if watched.as_ref().map(|w| &w.0) != session.as_ref() {
            if let Some((w, a, b)) = watched.take() {
                let _ = w.RemoveMediaPropertiesChanged(a);
                let _ = w.RemovePlaybackInfoChanged(b);
            }
            if let Some(s) = &session {
                let (on_props, on_playback) = (changed(&tx), changed(&tx));
                let a = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                    on_props();
                    Ok(())
                }));
                let b = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                    on_playback();
                    Ok(())
                }));
                watched = Some((s.clone(), a.unwrap_or(0), b.unwrap_or(0)));
            }
        }
        let now = session.as_ref().and_then(|s| read(s, &cache, &mut art_for));
        let differs = state.lock().map(|mut g| {
            let d = *g != now;
            *g = now;
            d
        });
        if differs.unwrap_or(false) {
            // SAFETY: posting to our window; a closed window drops it.
            unsafe {
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_MEDIA, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// The session's track, status and art (art saved once per track, the previous file removed).
fn read(s: &GlobalSystemMediaTransportControlsSession, cache: &Path, art_for: &mut String) -> Option<NowPlaying> {
    let props = s.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = props.Title().ok()?.to_string().trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let artist = props.Artist().map(|h| h.to_string().trim().to_owned()).unwrap_or_default();
    let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).ok() == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
    let app = app_name(&s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default());
    let key = format!("{app}\n{title}\n{artist}");
    let file = cache.join(format!("nowplaying-{:016x}.img", fnv(&key)));
    if *art_for != key {
        if !art_for.is_empty() {
            let _ = std::fs::remove_file(cache.join(format!("nowplaying-{:016x}.img", fnv(art_for))));
        }
        *art_for = key;
        let _ = save_art(&props, &file);
    }
    let art = file.exists().then_some(file);
    Some(NowPlaying { title, artist, app, playing, art })
}

fn save_art(props: &GlobalSystemMediaTransportControlsSessionMediaProperties, file: &Path) -> Option<()> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok().filter(|&n| n > 0 && n <= 8 << 20)?;
    let input: IInputStream = stream.cast().ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    let got = reader.LoadAsync(size).ok()?.join().ok()?;
    let mut buf = vec![0u8; got as usize];
    reader.ReadBytes(&mut buf).ok()?;
    let _ = std::fs::create_dir_all(file.parent()?);
    std::fs::write(file, &buf).ok()
}

/// FNV-1a, for art file names.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn players_are_named_from_their_app_ids() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(app_name("MSEdge"), "Edge");
        assert_eq!(app_name("Chrome"), "Chrome");
        assert_eq!(app_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(app_name("foobar2000.exe"), "foobar2000");
    }
}
