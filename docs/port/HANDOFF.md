# Handoff

Where Sofa stands, for the next session. Read [MAP.md](MAP.md) first, then the ADRs it
points to, then `../../CLAUDE.md`.

## State (2026-11-27, 2.1.0 released)

- Sofa 2.1.0 is the native app only (ADR-008), released from `main` as v2.1.0. It passes the automated QA run (53 checks),
  `cargo test --workspace` and `cargo clippy --workspace --all-targets` with no warnings.
- It installs itself (`Sofa-Setup-<version>.exe`), replaces Sofa 1.x keeping
  `%APPDATA%\sofa`, and uninstalls from Installed apps (`Sofa.exe --uninstall`).
- The 1.x features still missing are listed in `../../PRODUCT.md`, "Not yet in 2.x".

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
2. DualSense output reports: lightbar in the focused game's colour, player light, mute LED.
   Send only on change, never on focus moves (1.x lag lesson).
3. Audio: the sound output switcher and controller headphones (IMMDeviceEnumerator
   notifications plus the policy config interface for the default endpoint).
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
