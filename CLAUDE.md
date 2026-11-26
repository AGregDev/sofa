# Sofa: notes for coding agents

Read PRODUCT.md first: it says what Sofa promises and what was removed on purpose.
QA.md lists every check a release must pass.

## Commands

```powershell
npm install
npm start                      # run from source (uses your real profile)
npm run qa                     # automated QA: drives every screen, writes shots/ and shots/qa.txt, exit 0 = all pass
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\qa-helper.ps1   # desktop QA for the PS-button helper (close Sofa first)
powershell -NoProfile -ExecutionPolicy Bypass -File setup.ps1 -BuildOnly    # build dist\Sofa Setup <version>.exe
```

`$env:SOFA_DEMO=1; npm run qa` also records `shots/demo/` frames for the README
animation (see "README media" below).

## Layout

| Path | What it does |
| --- | --- |
| `main.js` | Electron main: window, `sofa://` protocol, library cache, launching, running-app tracking, staying on top, all IPC |
| `preload.js` | `window.sofa` bridge (the only API the UI can call) |
| `preload-web.js` | YouTube TV window: controller to remote keys |
| `src/library.js` | Steam, RecompLauncher and Start menu scan; categories; web and system tiles |
| `src/social.js` | Discord local RPC (named pipe), OAuth with the user's own application, encrypted tokens |
| `src/store.js` | `sofa.json` settings, overrides, recent |
| `src/shots.js` | the QA run (`--shots`): fake gamepad, checks, screenshots |
| `scripts/padwatch.ps1` | background helper: joystick PS button, focus, launcher guard, Steam push-down |
| `scripts/apps.ps1`, `audio.ps1`, `uninstall.ps1` | Start menu apps and icons, audio endpoints and mic mute, uninstallers |
| `renderer/app.js` | the whole UI: tabs, shelf, hero, layers (sheets, dialogs, keyboard), setup, Discord, settings |
| `renderer/input.js` | gamepad and keyboard to actions; button layouts; duplicate-press filter |
| `renderer/dualsense.js` | WebHID DualSense: battery, lightbar, mute LED, and the pad itself in full-report mode |

## Rules that keep Sofa working

- **The UI talks to Windows only through `preload.js` IPC.** Every IPC handler
  validates its input (ids, URLs on an allowlist, hwnds as digits).
- **QA mode (`--shots`) must never touch the real world.** It wipes and uses
  `%TEMP%\sofa-shots`, and launch, open, uninstall and Discord all return early.
  Add the same guard to any new handler with side effects.
- **Never redraw the shelf on a timer unless the data changed** (see
  `refreshSocial`'s signature check). Redraws move focus and read as glitches.
- **No rumble on focus moves.** Bluetooth output reports on every move caused lag.
- **Discord's Authorize prompt only on an explicit Connect** (`state(true)`).
  Background refreshes use the saved token and report an error instead.
- **DualSense full-report mode** (entered when Sofa reads it over Bluetooth) hides
  its buttons from the browser Gamepad API and winmm until the controller is
  power-cycled. Sofa reads buttons from HID itself (`DS.pad`) and forwards the PS
  button to the helper (`pad:home`).
- **Steam and the PS button:** the helper minimizes Steam when it grabs focus and
  prints `steamgrab`; the UI then offers the one-time Steam settings fix. Do not
  try to block Steam in other ways.
- **Building:** NSIS fails on paths over 260 characters, so `setup.ps1` mirrors the
  project to `%TEMP%\sofa-build` and builds there.

## Design rules

Warm graphite surfaces, one amber accent (`--accent`) for state only, colour from
game art. No decorative gradients (scrims over art are fine), no glows, no emoji,
no uppercase tracked labels. Lucide-style 1.5px line icons. Every hint is a real
keycap (`key()`), and the hint bar shows only actions that work on the focused
tile. Copy is plain: say what happens, no "please", no exclamation marks.

## Releasing

1. Bump `version` in `package.json`.
2. `npm run qa` (all PASS) and `scripts\qa-helper.ps1` (no FAIL).
3. Walk the manual checks in QA.md on real hardware.
4. `setup.ps1 -BuildOnly`, then `gh release create v<version> "dist\Sofa Setup <version>.exe"`.

## README media

Screenshots in `docs/screenshots/` come from a QA run (`shots/`), converted to
JPEG. The animation `docs/demo.webp` is built from `shots/demo/` with ffmpeg; the
frame timings are in `shots/demo/times.json`.
