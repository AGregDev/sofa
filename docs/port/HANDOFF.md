# Handoff

Where Sofa stands, for the next session. Read [MAP.md](MAP.md) first, then the ADRs it
points to, then `../../CLAUDE.md`.

## State (2026-11-27, 2.2.2 released)

- Sofa 2.2.2 is the native app only (ADR-008), released from `main` as v2.2.2. It passes the
  automated QA run (73 checks), `cargo test --workspace` and
  `cargo clippy --workspace --all-targets` with no warnings. Of the manual checks only M21
  (Spotify connect, for 2.2.2) and M25 (on the TV, for 2.2.1) were walked; the others were not
  walked for 2.1.0 to 2.2.2.
- It installs itself (`Sofa-Setup-<version>.exe`), replaces Sofa 1.x keeping
  `%APPDATA%\sofa`, and uninstalls from Installed apps (`Sofa.exe --uninstall`).
- The 1.x features still missing are listed in `../../PRODUCT.md`, "Not yet in 2.x".

## New in 2.2.2

- **Spotify's redirect has a port** (`spotify::REDIRECT`, `PORT`). The dashboard refuses
  `http://127.0.0.1/callback` ("This redirect URI is not secure"), with or without the path,
  although Spotify's documentation says to register a loopback redirect without a port. The
  setup now asks for `http://127.0.0.1:7632/callback`, and the sign-in listens on 7632 (any
  free port if it is taken). Spotify itself does not compare a loopback redirect's port: 2.2.1
  signed in on a random port against that registration.
- Spotify connected with a real account for the first time (M21). Playback from Music (M22) is
  still untried.

## New in 2.2.1

- **Resuming hands the screen over on release** (`App::resume`, `resume_tick`,
  `input::let_go`). Switching to a game on the ✕ press let it read ✕ as a fresh press
  (Assassin's Creed IV jumped; Steam Input also changes from its desktop layout to the game's
  as the game comes forward). The window now takes the focus 80 ms after no controller button
  and no key Sofa reads is down (`Input::anything_down`; keys tracked from WM_KEYDOWN and
  WM_KEYUP), or after 3 s if something stays down. Sofa losing the focus first cancels it.
- ✕ on a library tile whose program is open already resumes it (`App::window_of`: an app's
  own program, or a window whose program is in a game's folder), so Steam no longer does the
  switch with ✕ still down.
- QA: `Key` posts a key-up with a real key-up's flags (with lParam 0, TranslateMessage types
  its letter); R4 and R5 cover the wait; a QA run no longer drops its own keys while someone
  plays with a controller (`Input::key`'s pad filter is skipped in QA mode).

## New in 2.2.0

- PS on Home opens the **quick menu** (`Layer::Quick`, `ui.rs` "The quick menu" block,
  `view.rs` `draw_quick`): Now playing, Sound output, Volume, Microphone, Sleep timer,
  Controller light, Power. Anywhere else PS still goes Home; ○ on Home returns to the first
  tile. QA H4 changed and U1 to U13 were added.
- **DualSense output** (`input.rs`): lightbar, player light and mute LED, written by the
  `dualsense` thread between reads, only when `Lights` changed; the lightbar only while Sofa
  is in front and only after focus rested 0.7 s. The mute button (`WM_PAD_MUTE`) mutes the
  microphone from any app: in Discord while in voice, else Windows' default microphone.
- **Sound** (`audio.rs`): outputs, default output (policy config interface), volume (L2 and
  R2 now use the endpoint volume and show Sofa's own level in the top bar), microphone mute.
- **Now playing** (`media.rs`): Windows' media sessions on the `media` thread; the quick
  menu shows the track and art and controls it (Spotify has no TV interface for a browser).
- **Spotify** (`spotify.rs`, Music tab, Home's Music row): Liked Songs, playlists and saved
  albums from the Web API, played on this PC's Spotify app (`/me/player/play` with this PC's
  Connect device, Spotify started when closed). The person makes a Spotify app (Development
  Mode: the owner needs Premium since February 2026, five users) with the redirect
  `http://127.0.0.1:7632/callback` (since 2.2.2: the dashboard refuses a loopback redirect
  without a port, although Spotify's documentation says to register one that way); sign-in is
  PKCE in the browser answered on 127.0.0.1 at that port (any free port if it is taken); the
  refresh token is sealed with DPAPI in `music.spotify`. Playlists report `items` since
  February 2026 (`tracks` before): both are read. Connected with a real account on
  2026-11-27 (M21); playback (M22) not yet tried.
- **○ acting as ✕ fixed**: Steam's desktop layout turned ○ into Enter. While Sofa is in front
  with a pad it reads, other programs' synthetic keys that Sofa reads as actions are held back
  (`Hold::SofaKeys`, the same hook as YouTube). Holding every key there made Spotify show
  "not responding" on a second start; picking an app that is already open now switches to it.
- **Sleep timer**: epoch-based (`sleep_at`), a countdown dialog a minute before, then
  `platform::sleep`. A QA run never sleeps; `WM_QA_SLEEP` (0x8000 + 13) tests the warning.
- Colour from art (`gfx::tone`, computed when art is decoded) feeds the lightbar; the hero
  shows Steam's total play time and a running game's session length (process start time).
- Fixes: Sofa no longer lists itself; row counts of 0 are hidden; dialogs show keycaps;
  13 settings fit at 16:9, and longer sheets scroll by whole options.
- For anyone: text names the button on the controller in use (PS or Xbox button); with an
  Xbox pad, the first-run screen and Settings offer "Keep Game Bar off the Xbox button"
  (`UseNexusForGameBarEnabled` under HKCU Software Microsoft GameBar, setting `gameBarOff`);
  the Music tab shows only with Spotify installed or connected.

## New in 2.1.0

- Go to desktop, then PS: Sofa came to the front but stayed minimized. Fixed in `main.rs`
  (`ours()`: only Sofa's own messages wait while the app is busy).
- Running cards name the program from its file description and match Start menu apps by that
  name (Firefox and Chrome have app IDs, not paths); icons come from the program file.
- Streaming (ADR-009): Edge, Chrome, then Firefox; the launch screen covers the start; page
  zoom per screen; Chrome kiosk; brand tiles and hero; the controller as YouTube's remote,
  with other programs' synthetic input (Steam's desktop layout) held back meanwhile.
- Discord voice is back (`social.rs`, Social tab, Home row, Connect walkthrough), reading the
  1.x sign-in (ADR-003 met).
- Crunchyroll joins the streaming tiles.
- QA mode ignores the real controller.

## Next, in order

1. Walk the manual checks in QA.md on the TV with a DualSense (M1 to M15) and record them
   (2.1.0 was released before they were walked).
2. Try the new output reports on a real DualSense over USB and Bluetooth (M16, M17), the
   quick menu's sound output and Now playing (M18, M19), Connect Spotify and play from Music
   (M21, M22), and ○ with Steam open (M23).
3. Controller headphones taking the sound by themselves: endpoint notifications
   (IMMNotificationClient) on top of `audio.rs`.
4. Custom covers and uninstalling from Sofa (the `cover` override is already read).
5. Automate D1 to D4: a real Sofa in a window, PS posted as WM_PAD_HOME, then the foreground
   and minimized state checked (how this round's desktop bug was reproduced).

## Known issues

- The installer is unsigned: SmartScreen warns on first run.
- The demo animation in the README is recorded from the QA build (`shots\demo`, encoded
  with ffmpeg to `docs\demo.webp`).

## How to build and run

See CLAUDE.md. Short form, in PowerShell:

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\tools\winlibs\mingw64\bin;$env:Path"
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\sofa-build\target"
cargo build --release
& "$env:LOCALAPPDATA\sofa-build\target\release\sofa.exe" --windowed
```
