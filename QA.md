# Sofa QA

A release ships only when every automated check passes and the manual checks
have been walked on real hardware. Results for each release are recorded at the
bottom.

## 1. Automated UI run: `npm run qa`

Drives the real app with a simulated DualSense and keyboard from a clean profile.
Writes `shots/qa.txt` and a screenshot per screen. Never launches, opens or
uninstalls anything, and never talks to Discord.

| ID | Check |
| --- | --- |
| S1 | First run opens setup |
| S2 | On the controller check, tapping ✕ and ○ only lights them |
| S3 | Holding ✕ moves on |
| S4 | Setup recommends the Steam "Guide button focuses Steam" fix |
| S5 | Finishing setup shows Home |
| H1 | Tabs are Home, Games, Media, Apps, Social, System (no Phone, no Steam friends) |
| H2 | Home rows stay short (15 tiles at most) |
| H3 | A tile has focus and the hero shows its name |
| H4 | The "All ..." tile opens its section |
| P1 | Each focus move is handled in under 16 ms |
| P2 | No main-thread task over 100 ms while browsing |
| R1 | PS from a game puts it first on Home, focused |
| R2 | △ on it offers Resume, Close, Force close |
| R3 | Starting another game asks about the running one |
| T1 | Settings has Go to desktop, Quit Sofa and Button layout |
| D1 | Social offers Connect Discord and nothing from Steam |
| D2 | Discord setup shows five numbered steps |
| D3 | The Client ID is picked up from the clipboard |
| Y1 | System has Go to desktop and Quit Sofa |
| L1 | Nintendo layout labels confirm as A |
| L2 | Xbox layout labels confirm as A |
| I1 | △ opens the item menu |
| I2 | Change cover opens the picture picker |
| K1 | The on-screen Paste key types the clipboard |
| K2 | Ctrl+V pastes into the keyboard |
| K3 | Typing in search finds library items |
| N1 | A press that arrives as both pad and key (Steam desktop layout) moves once |
| E1 | No errors in the console |

## 2. Helper on the desktop: `scripts\qa-helper.ps1`

Close Sofa first. Opens two test windows (one stands in for Sofa, one for a game)
and drives `padwatch.ps1` like Sofa does.

| ID | Check |
| --- | --- |
| W1 | The helper starts and says ready |
| W2 | Home from a game brings Sofa forward and reports the game |
| W3 | Alive reports the open game window |
| W4 | Close closes the game, and alive stops reporting it |
| W5 | Focus brings the game back to the front (Resume) |

W2 and W5 need an active, unlocked desktop. With the screen off Windows gives no
window the focus, so they report SKIP rather than a result.

## 3. Build and install

| ID | Check |
| --- | --- |
| B1 | `setup.ps1 -BuildOnly` produces `dist\Sofa Setup <version>.exe` |
| B2 | The installer installs per-user without admin and starts Sofa |
| B3 | The installed Sofa and its helper are running 20 seconds later |

## 4. Manual, on real hardware

These need a person, a TV and a controller.

| ID | Check |
| --- | --- |
| M1 | PS button from a running game opens Sofa with the game first; ✕ resumes it |
| M2 | With Steam's "Guide button focuses Steam" on, PS shows Sofa's one-time Steam tip and Steam does not stay in front |
| M3 | With that setting off, PS opens Sofa with no flicker |
| M4 | Go to desktop shows Windows; PS brings Sofa back |
| M5 | Discord: Connect once, restart Sofa; voice channels appear with no new Authorize prompt |
| M6 | Discord: join a recent channel from Home; mute with the controller's mute button and its LED follows |
| M7 | Battery level and charging show in the top bar; lightbar follows the selected game |
| M8 | Xbox controller: A selects, B goes back; Nintendo layout swaps them |
| M9 | Netflix opens full screen in the chosen browser; YouTube opens in its TV interface; hold ○ leaves |
| M10 | Uninstall a Start menu app from its options; it disappears after the uninstaller finishes |
| M11 | Moving quickly across a long row stays smooth on the TV |

## Results

### 1.1.0

- Automated UI run: 28 of 28 PASS.
- Helper: W1, W3, W4 PASS; W2 and W5 SKIP (run with the display off: no window
  could take focus). They passed in daily use before this release (helper log).
- Build and install: see the release notes.
- Manual: not yet walked for 1.1.0.
