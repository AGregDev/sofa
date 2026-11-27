# Handoff

Where Sofa stands, for the next session. Read [MAP.md](MAP.md) first, then the ADRs it
points to, then `../../CLAUDE.md`.

## State (2026-11-27)

- Sofa 2.0.0 is the native app only (ADR-008). It passes the automated QA run (53 checks),
  `cargo test --workspace` and `cargo clippy --workspace --all-targets` with no warnings.
- It installs itself (`Sofa-Setup-<version>.exe`), replaces Sofa 1.x keeping
  `%APPDATA%\sofa`, and uninstalls from Installed apps (`Sofa.exe --uninstall`).
- The 1.x features not in 2.0 are listed in `../../PRODUCT.md`, "Not yet in 2.0".

## Next, in order

1. Walk the manual checks in QA.md on the TV with a DualSense (M1 to M12), and record them.
2. DualSense output reports: lightbar in the focused game's colour, player light, mute LED.
   Send only on change, never on focus moves (1.x lag lesson).
3. Discord voice: named pipe RPC (`\\.\pipe\discord-ipc-N`), OAuth with the person's own
   application (WinHTTP), secrets with DPAPI. Electron's `safeStorage` secrets are not
   readable outside Electron: ask once to reconnect (ADR-003).
4. Audio: the sound output switcher and controller headphones (IMMDeviceEnumerator
   notifications plus the policy config interface for the default endpoint).
5. Custom covers and uninstalling from Sofa.

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
