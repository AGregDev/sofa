# Sofa: notes for coding agents

Read PRODUCT.md first: it says what Sofa promises, what is not in 2.0 yet, and what was
removed on purpose. QA.md lists every check a release must pass.

Sofa is one native executable written in Rust, with no web engine: Direct2D on Windows, and
since 2.3 an X11 window drawn on the CPU on Linux (ADR-010). It replaced the Electron app in
2.0 (docs/port/DECISIONS.md, ADR-008); the Electron sources are in git history and the v1.3.0
release. The port's map and next steps are in `docs/port/MAP.md` and `docs/port/HANDOFF.md`.

## Commands

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\tools\winlibs\mingw64\bin;$env:Path"
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\sofa-build\target"
cargo build --release                                   # target\release\sofa.exe
& "$env:CARGO_TARGET_DIR\release\sofa.exe" --windowed   # 1600x900 window; full screen without the flag
cargo test --workspace                                  # VDF recorded cases, Steam editor G1-G15
cargo clippy --workspace --all-targets                  # must stay at zero warnings
powershell -NoProfile -ExecutionPolicy Bypass -File qa\qa.ps1   # automated QA, exit 0 = all pass
```

`sofa.exe` flags: `--windowed`, `--size WxH` (borderless, for screenshots), `--log`
(`%TEMP%\sofa-native.log`), `--qa` (see below), `--uninstall [--quiet]`, and the internal
`--scan <cache> [--jobs <file>] [--no-startmenu]` and `--relaunch`. Named
`Sofa-Setup-*.exe` (anything containing "setup"), the same file installs itself.

Linux (in CI: `.github/workflows/linux.yml`, Ubuntu 20.04 container, every push):

```bash
cargo build --release                          # target/release/sofa
target/release/sofa --windowed
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
xvfb-run -a -s "-screen 0 1920x1080x24" bash qa/qa.sh target/release/sofa shots/linux
bash packaging/linux/build-appimage.sh target/release/sofa dist   # Sofa-Setup-<v>-x86_64.AppImage
```

From Windows, type-check the Linux build (ring, rustls' cryptography, cannot be compiled
there, so HTTPS is left out):

```powershell
cargo clippy --target x86_64-unknown-linux-gnu --no-default-features -p sofa --all-targets
```

On Linux `--install` installs any build (as a file named `*setup*` does); `--qa` adds a control
pipe (`$TMPDIR/sofa-qa/control`) that `qa/qa.sh` drives.

## Layout

| Path | What it does |
| --- | --- |
| `app/src/main.rs` | window, message loop, single instance, PS button routing, deferred messages, setup/uninstall/scan modes, log |
| `app/src/ui.rs` | state and behaviour: rows, focus, tabs, layers (sheets, dialogs, keyboard, welcome, launch, quick menu), settings, Running cards, Steam fix, notices, memory trim, sleep timer, DualSense lights |
| `app/src/view.rs` | all drawing: top bar, hero, shelf, tiles, hints, layers, quick menu, toast, start-up |
| `app/src/gfx.rs` | Direct2D on a DXGI flip swap chain, DirectWrite (Figtree from the WOFF2), WIC decode thread, SVG icon paths, image budget |
| `app/src/input.rs` | keyboard, XInput (Guide via ordinal 100), DualSense over HID on its own thread (buttons, mute button, output reports for its lights), layouts, repeat, duplicate-press filter |
| `app/src/audio.rs` | sound outputs and the default one (policy config interface), volume, microphone mute |
| `app/src/media.rs` | Now playing: Windows' media sessions on the `media` thread, play, pause, skip, art |
| `app/src/spotify.rs` | Spotify on the `spotify` thread: PKCE sign-in answered on 127.0.0.1, library (Liked Songs, playlists, albums, covers), playback on this PC |
| `app/src/shell.rs` | foreground events: topmost while home, the guard (desktop, taskbar, Start, Search), Steam push-down, full-screen sessions, Running windows |
| `app/src/library.rs` | Steam (through `crates/vdf`), RecompLauncher, Start menu (from `cache\startmenu.json`), web and system tiles, `identify` |
| `app/src/social.rs` | Discord voice: RPC pipe on the `discord` thread, OAuth over WinHTTP, secrets (DPAPI, and 1.x's safeStorage values) |
| `app/src/startmenu.rs` | Start menu enumeration and 256 px icons; runs only in the `--scan` child process |
| `app/src/platform.rs` | bring to front, launch, browsers and their TV profiles, power, processes, clipboard, working set |
| `app/src/install.rs` | install, start with Windows, Start menu shortcut, Installed apps entry, uninstall, removing Sofa 1.x |
| `app/src/store.rs` | `%APPDATA%\sofa\sofa.json`, same keys as Sofa 1.x |
| `app/src/sound.rs` | interface sounds, synthesized once into in-memory WAVs |
| `app/src/perf.rs` | the Performance panel and overlay: memory, Sofa's own, each card's video memory (DXGI and PDH's GPU Adapter Memory counters; NVML and amdgpu on Linux, opened once on the short-lived `perf` thread), the frame rate |
| `app/src/icons.rs` | reads `app/assets/icons.js` (Lucide) and `brand.js` at build time |
| `app/src/draw.rs` | what both renderers share: Rect, Color, Fit, the art's tone, SVG path data walked into either renderer's paths |
| `app/build.rs` | icon and version resource through windres (Windows targets only) |
| `app/src/linux/` | Linux. Picked in `main.rs` in place of the Windows file: `gfx.rs` (tiny-skia into softbuffer, same API), `platform.rs`, `audio.rs` (`pactl`), `media.rs` (MPRIS over zbus), `install.rs` (AppImage, desktop entry, autostart), `startmenu.rs` (desktop entries, `--scan` only). Shared by those: `app.rs` (winit X11 window, loop, single instance, QA pipe, signals), `text.rs` (Figtree with swash, DirectWrite-like layout), `x11.rs` (EWMH, XTest, clipboard), `wm.rs` (what `shell.rs` asks of the window manager), `pads.rs` (evdev, hidraw), `desktop.rs` (desktop entries, icon themes), `proc.rs` (`/proc`, Wine paths), `net.rs` (ureq + rustls), `secret.rs` (AES-GCM, key file), `pulse.rs` (sounds), `clock.rs` |
| `qa/qa.sh` | the Linux QA run (the same 76 checks) |
| `packaging/linux/` | the AppImage's desktop entry and `build-appimage.sh` |
| `crates/vdf` | Steam's VDF parser, ported with rust-anything (172 recorded cases in `tests/`) |
| `crates/steam` | turns off "Guide button focuses Steam" in each account's `localconfig.vdf` |
| `qa/qa.ps1` | the automated QA run |
| `assets/` | icon (`icon.png`, `icon.ico`), welcome scene, Blender script |

## Rules that keep Sofa working

- **Two systems, one program.** Windows-only code is behind `#[cfg(windows)]`, Linux-only
  behind `#[cfg(target_os = "linux")]`, and the shared logic (ui, view, shell's `Shell`, input's
  `Input`, library, social, spotify) calls small functions both sides provide. A change for one
  system must leave the other's build, tests and QA run as they were: run clippy for both
  targets. The Windows build stays exactly as it was before Linux (ADR-010).
- **On Linux Sofa is always an X11 window** (winit with X11 forced; XWayland under Wayland),
  because only X11 lets a program bring itself and others forward (EWMH, as a pager). Under
  Wayland only X11 windows can be seen; never treat "no window in front" as the desktop there
  (`shell::NOTHING_IS_SHELL`). Browsers Sofa starts get `--ozone-platform=x11` (or
  `MOZ_ENABLE_WAYLAND=0`) and the class `SofaWeb` so Sofa can see and drive them.
- **QA mode (`--qa`) must never touch the real world.** It uses `%TEMP%\sofa-qa`, its own
  window class and mutex, no foreground hook, no topmost, no hotkey, and launching,
  closing, the Steam fix and start with Windows return early. Add the same guard to
  anything new with side effects. QA check X1 compares the real settings, Run key, Steam
  setting, shortcut and Installed apps entry before and after. On Linux: `$TMPDIR/sofa-qa`,
  the class `SofaQA` and its own socket, no window-manager watch, no autostart or desktop
  entry written, no key file; X1 compares `~/.config/sofa`, the autostart and desktop entries,
  `~/.local/bin/sofa` and Steam's setting.
- **Draw only when something changed.** `App::animating` decides; the loop otherwise
  sleeps in `MsgWaitForMultipleObjects`. Never redraw on a timer, never move focus under
  the person (rebuilds keep focus by tile id).
- **Layers hold item indices.** A new library waits while one is open
  (`pending_rescan`); Running cards are keyed by window handle, never by position.
- **Win32 calls inside an action can send messages back** (focus, minimize). `handle()`
  defers them while the app is borrowed; keep using `with_app` and never hold the app
  across a call that pumps messages without that guard. Only messages listed in `ours()`
  are deferred; everything else must reach DefWindowProc (WM_QUERYOPEN answered 0 keeps a
  minimized window minimized).
- **Synthetic input** (SendInput, keybd_event) must carry `platform::SOFA_MARK` in
  `dwExtraInfo`: while a streaming window Sofa opened is in front, all unmarked synthetic
  input is held back (`platform::Hold::All`); while Sofa itself is in front with a DualSense or
  XInput pad it reads, only unmarked keys Sofa reads as actions are (`Hold::SofaKeys`: Steam's
  desktop layout sent Enter for ○, which acted as ✕). Holding everything there stopped
  Chromium apps (Spotify) from faking the key press that brings their window forward. The PS
  button's Alt tap would be lost without the mark.
  `cargo test --bin sofa -- --ignored` checks the hold (it sends F24 system-wide).
- **Discord** runs only outside QA mode and only on its own thread; the UI reads `Voice` and
  saves `social.discord` when the thread hands it over (same keys as 1.x). **Spotify** follows
  the same pattern (`Music`, `music.spotify`); a QA run shows a made-up library and plays
  nothing.
- **Memory is the point.** Decode art at the size it is drawn (`image_sized` with the
  focused size, so zooming never decodes twice), stay under the image budget, give memory
  back 15 s after a game takes the screen, and keep Windows' shell code (icons, Start menu)
  in the `--scan` child. Measure with the QA run (P1 to P4).
- **No rumble or output reports on focus moves.** Bluetooth output reports on every move
  caused lag in 1.x. The `dualsense` thread writes `Lights` only when they changed, between
  reads; the UI changes the lightbar only after focus rested 0.7 s, and only while Sofa is in
  front (behind a game, only the mute LED, after the mute button).
- **Sound, players and the sleep timer touch the real world.** In QA mode `audio.rs` setters
  are never called (made-up devices in `AudioState::qa`), the `media` thread does not run (a
  made-up track), the lights are never armed and the sleep timer never calls
  `platform::sleep`.
- **Hand the screen over on release.** A game given the focus while ✕ is still down reads a fresh
  press (Assassin's Creed IV jumped), and so does Steam Input switching from its desktop layout
  to the game's. `App::resume` waits until no controller button and no key Sofa reads is down
  (80 ms after, `input::let_go`; after 3 s it goes anyway) before `platform::focus`. Bring other
  windows forward from a press only through it. ✕ on a library tile whose program is open
  already (`App::window_of`) resumes it too, instead of asking Steam to switch.
- **The PS button.** The DualSense thread posts `WM_PAD_HOME` (wparam 0 PS, 1 Create +
  Options) even while a game has focus; XInput's Guide comes from `poll`. Both go through
  `home_button`, which drops duplicates within 350 ms. With "Home button: Create +
  Options", PS is ignored.
- **Steam and the PS button:** Steam's "Guide button focuses Steam" is
  `Controller_CheckGuideButton` in `userdata/<id>/config/localconfig.vdf`. Steam rewrites
  the file on exit, so `crates/steam` edits it only while Steam is closed: silently at
  start, or after the person agrees to a restart (`steam.exe -shutdown`, then `-silent`).
  It keeps a `.sofa-backup`, verifies by parsing back, and never restarts Steam under a
  running game. `shell.rs` still pushes Steam down if it grabs the screen.
- **Same data as 1.x** (ADR-003): never rename keys in `sofa.json` or move caches. An
  unreadable `sofa.json` is copied to `sofa.json.unreadable` before defaults are written.
- **Unsafe code** only for system calls (Win32, libc, dlopen'd C libraries), each block with a
  `// SAFETY:` comment.

## Design rules

Warm graphite surfaces, one amber accent (`ACCENT`) for state only, colour from game art.
No decorative gradients (scrims over art are fine), no glows, no emoji, no uppercase
tracked labels. Lucide-style 1.5 px line icons. Every hint is a real keycap (`keycap()`),
and the hint bar shows only actions that work on the focused tile. Copy is plain: say what
happens, no "please", no exclamation marks. Lay out in rem (screen height / 67.5); colours
and the motion curve are defined once at the top of `ui.rs`.

## Toolchain (Linux)

- Rust stable (1.97 or later) and a C compiler (rustls' `ring`). No `-dev` packages: X11,
  xkbcommon and libpulse-simple are loaded at run time, so the binary links only glibc.
- The release build in CI runs on Ubuntu 20.04 (glibc 2.31): a newer build host makes an
  AppImage that older distributions refuse.

## Toolchain (Windows, no Visual Studio)

- Rust stable, GNU host (`stable-x86_64-pc-windows-gnu`), installed with rustup.
- WinLibs MinGW-w64 (GCC + binutils, `windres`) in `C:\tools\winlibs\mingw64\bin`, on PATH.
  The folder must have **no spaces** anywhere in its path: GCC derives internal paths from
  its own location and breaks on spaces.
- **Builds go to `CARGO_TARGET_DIR=%LOCALAPPDATA%\sofa-build\target`.** Inside the project
  folder, build paths pass Windows' 260-character limit.
- rust-anything 5.0.1 for logic units ported from the original (see docs/port).

## Releasing

1. Bump `version` in `app/Cargo.toml`.
2. `cargo clippy --workspace --all-targets` (no warnings), `cargo test --workspace`, and
   `qa\qa.ps1` (all PASS). Record the results in QA.md.
3. Walk the manual checks in QA.md on real hardware.
4. `cargo build --release`, copy `sofa.exe` to `dist\Sofa-Setup-<version>.exe`, run it once
   to check the install (B2, B3), then
   `gh release create v<version> "dist\Sofa-Setup-<version>.exe"`.
5. Publishing the release runs the Linux workflow, which attaches
   `Sofa-Setup-<version>-x86_64.AppImage` to it. Its QA run must pass; walk LB2 to LB6 and the
   LM checks on a Linux PC.

## README media

Screenshots in `docs/screenshots/` come from a QA run (`shots/`), converted to JPEG. The
animation `docs/demo.webp` is recorded from the QA build into `shots/demo/` (PrintWindow
frames with their times in `list.txt`) and encoded with ffmpeg (`libwebp_anim`, 20 fps).
