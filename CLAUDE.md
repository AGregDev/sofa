# Sofa: notes for coding agents

Read PRODUCT.md first: it says what Sofa promises, what is not in 2.0 yet, and what was
removed on purpose. QA.md lists every check a release must pass.

Sofa is one native Windows executable written in Rust (Direct2D, no web engine). It
replaced the Electron app in 2.0 (docs/port/DECISIONS.md, ADR-008); the Electron sources
are in git history and the v1.3.0 release. The port's map and next steps are in
`docs/port/MAP.md` and `docs/port/HANDOFF.md`.

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

## Layout

| Path | What it does |
| --- | --- |
| `app/src/main.rs` | window, message loop, single instance, PS button routing, deferred messages, setup/uninstall/scan modes, log |
| `app/src/ui.rs` | state and behaviour: rows, focus, tabs, layers (sheets, dialogs, keyboard, welcome, launch), settings, Running cards, Steam fix, notices, memory trim |
| `app/src/view.rs` | all drawing: top bar, hero, shelf, tiles, hints, layers, toast, start-up |
| `app/src/gfx.rs` | Direct2D on a DXGI flip swap chain, DirectWrite (Figtree from the WOFF2), WIC decode thread, SVG icon paths, image budget |
| `app/src/input.rs` | keyboard, XInput (Guide via ordinal 100), DualSense over HID on its own thread, layouts, repeat, duplicate-press filter |
| `app/src/shell.rs` | foreground events: topmost while home, the guard (desktop, taskbar, Start, Search), Steam push-down, full-screen sessions, Running windows |
| `app/src/library.rs` | Steam (through `crates/vdf`), RecompLauncher, Start menu (from `cache\startmenu.json`), web and system tiles, `identify` |
| `app/src/startmenu.rs` | Start menu enumeration and 256 px icons; runs only in the `--scan` child process |
| `app/src/platform.rs` | bring to front, launch, browsers and their TV profiles, power, processes, clipboard, working set |
| `app/src/install.rs` | install, start with Windows, Start menu shortcut, Installed apps entry, uninstall, removing Sofa 1.x |
| `app/src/store.rs` | `%APPDATA%\sofa\sofa.json`, same keys as Sofa 1.x |
| `app/src/sound.rs` | interface sounds, synthesized once into in-memory WAVs |
| `app/src/icons.rs` | reads `app/assets/icons.js` (Lucide) and `brand.js` at build time |
| `app/build.rs` | icon and version resource through windres |
| `crates/vdf` | Steam's VDF parser, ported with rust-anything (172 recorded cases in `tests/`) |
| `crates/steam` | turns off "Guide button focuses Steam" in each account's `localconfig.vdf` |
| `qa/qa.ps1` | the automated QA run |
| `assets/` | icon (`icon.png`, `icon.ico`), welcome scene, Blender script |

## Rules that keep Sofa working

- **QA mode (`--qa`) must never touch the real world.** It uses `%TEMP%\sofa-qa`, its own
  window class and mutex, no foreground hook, no topmost, no hotkey, and launching,
  closing, the Steam fix and start with Windows return early. Add the same guard to
  anything new with side effects. QA check X1 compares the real settings, Run key, Steam
  setting, shortcut and Installed apps entry before and after.
- **Draw only when something changed.** `App::animating` decides; the loop otherwise
  sleeps in `MsgWaitForMultipleObjects`. Never redraw on a timer, never move focus under
  the person (rebuilds keep focus by tile id).
- **Layers hold item indices.** A new library waits while one is open
  (`pending_rescan`); Running cards are keyed by window handle, never by position.
- **Win32 calls inside an action can send messages back** (focus, minimize). `handle()`
  defers them while the app is borrowed; keep using `with_app` and never hold the app
  across a call that pumps messages without that guard.
- **Memory is the point.** Decode art at the size it is drawn (`image_sized` with the
  focused size, so zooming never decodes twice), stay under the image budget, give memory
  back 15 s after a game takes the screen, and keep Windows' shell code (icons, Start menu)
  in the `--scan` child. Measure with the QA run (P1 to P4).
- **No rumble or output reports on focus moves.** Bluetooth output reports on every move
  caused lag in 1.x.
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
- **Unsafe code** only for Windows calls, each block with a `// SAFETY:` comment.

## Design rules

Warm graphite surfaces, one amber accent (`ACCENT`) for state only, colour from game art.
No decorative gradients (scrims over art are fine), no glows, no emoji, no uppercase
tracked labels. Lucide-style 1.5 px line icons. Every hint is a real keycap (`keycap()`),
and the hint bar shows only actions that work on the focused tile. Copy is plain: say what
happens, no "please", no exclamation marks. Lay out in rem (screen height / 67.5); colours
and the motion curve are defined once at the top of `ui.rs`.

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

## README media

Screenshots in `docs/screenshots/` come from a QA run (`shots/`), converted to JPEG. The
animation `docs/demo.webp` is recorded from the QA build into `shots/demo/` (PrintWindow
frames with their times in `list.txt`) and encoded with ffmpeg (`libwebp_anim`, 20 fps).
