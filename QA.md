# Sofa QA

A release ships only when every automated check passes and the manual checks
have been walked on real hardware. Results for each release are recorded at the
bottom.

## 1. Automated UI run: `qa\qa.ps1`

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File qa\qa.ps1
```

Starts the release build with `--qa --size 1600x900`: its own data folder in
`%TEMP%\sofa-qa`, nothing launched, closed, installed or changed outside that folder,
no foreground hook, no topmost window. Drives it with posted keys, reads the state
line the app logs after every action, saves a screenshot per screen to `shots\` and
writes `shots\qa.txt`. Exit code 0 means every check passed.

| ID | Check |
| --- | --- |
| S1 | Starts and opens its window |
| S2 | First frame in under 1.5 s |
| W1 | First run shows the welcome screen, focus on Continue |
| W2 | The welcome screen's button layout cycles (and is saved) |
| W3 | Continue opens Home with the "Sofa is ready" notice |
| W4 | Setup is saved |
| L1 | The Start menu scan runs in the background and writes `startmenu.json` |
| L2 | The scan helper process exits when done |
| H1 | Home has rows and the library |
| H2 | Right moves along the row |
| H3 | Down moves to the next row |
| H4 | Home goes back to the first tile |
| T1 | R1 walks the tabs and wraps (Games, Media, Apps, Social, System, Home) |
| T2 | L1 goes back a tab |
| T3 | Back returns to Home |
| C1 | Options opens Settings |
| C2 | Focus moves down the list |
| C3, C4 | Reduce motion switches on and off (saved) |
| C5 | Show hidden apps switches on |
| C6, C7 | Home button: Create + Options, then PS again (saved) |
| C8 | Back closes Settings |
| K1 | Square opens search |
| K2 | Typing finds games and apps |
| K3 | Backspace deletes |
| K4 | On-screen keys type |
| K5 | Back closes the keyboard |
| I1 | Triangle opens the item menu |
| I2, I3 | Add to favorites, remove from favorites (saved) |
| I4 | Rename opens the keyboard with the name |
| I5 | Rename saves the name |
| I6 | An empty name brings the original back |
| G1 | Select shows the launch screen (nothing starts in QA) |
| G2 | Back leaves the launch screen |
| G3 | Launching records Jump back in |
| R1 | PS from an app adds a Running card, first and focused |
| R2 | Triangle on Running opens its menu (Resume, Close, Force close) |
| R3 | A closed app loses its card |
| O1 | A second start hands over to the first and exits |
| P1 | Memory on Home under 150 MB |
| P2 | Idle CPU under 1 % of a core |
| P3 | Threads stay few (24 at most) |
| P4 | Memory is given back in the background |
| P5 | Draws again after giving memory back |
| Q1 | Quit Sofa is the last setting |
| Q2 | Quit Sofa closes it |
| Q3 | No welcome once set up |
| Q4 | Settings were kept |
| Q5 | Closing the window quits |
| X1 | QA touched nothing outside its folder (real settings, start with Windows, Steam's setting, shortcut, Installed apps) |
| X2 | No errors in the log |

## 2. Unit tests: `cargo test --workspace`

| Crate | What it checks |
| --- | --- |
| `crates/vdf` | The Rust VDF parser against 172 cases recorded from the original JavaScript (`tests/cases.jsonl`) |
| `crates/steam` | G1 to G15: the Steam settings editor, on a made-up `localconfig.vdf` in a temp folder, never a real Steam install |

G1 to G15: a file without the key reads as on; turning it off adds the key with 0 and
changes nothing else; a file with the setting on reads 1; turning it off replaces only
the value; Windows line endings are kept; a file it cannot verify is refused; status
finds the account with the setting on; with Steam open and no permission to restart
it, nothing changes; with Steam closed the fix writes the file; the original is kept as
a backup; no temp file is left behind; running it again changes nothing; status then
reports it off; a missing Steam is reported, not guessed.

## 3. Build, install, uninstall

| ID | Check |
| --- | --- |
| B1 | `cargo build --release` builds `sofa.exe` with its icon and version |
| B2 | Run as `Sofa-Setup-<version>.exe`, it installs per user without admin, replaces Sofa 1.x (keeping `%APPDATA%\sofa`) and starts Sofa |
| B3 | Installed apps lists Sofa with its version; the Start menu has Sofa; start with Windows points at the installed file |
| B4 | `Sofa.exe --uninstall --quiet` removes the program, shortcut, start with Windows and the Installed apps entry, and keeps the settings |

## 4. On the desktop, with the installed app

Needs an unlocked desktop. A stand-in window plays the game.

| ID | Check |
| --- | --- |
| D1 | PS (from the controller thread) while another app is in front brings Sofa forward with that app first |
| D2 | Resume puts the app back in front and Sofa no longer covers it |
| D3 | Showing the desktop brings Sofa back within a second |
| D4 | Go to desktop lets the desktop stay; PS brings Sofa back |

## 5. Manual, on real hardware

These need a person, a TV and a controller.

| ID | Check |
| --- | --- |
| M1 | PS button from a running game opens Sofa with the game first; ✕ resumes it |
| M2 | With Steam's "Guide button focuses Steam" on, PS shows Sofa's one-time Steam notice and Steam does not stay in front |
| M3 | With that setting off, PS opens Sofa with no flicker |
| M4 | Go to desktop shows Windows; PS brings Sofa back |
| M5 | Battery level and charging show in the top bar; a low battery shows a notice |
| M6 | Xbox controller: A selects, B goes back, Guide goes home; Nintendo layout swaps A and B |
| M7 | Netflix opens full screen in the chosen browser; YouTube opens in its TV interface |
| M8 | Moving quickly across a long row stays smooth on the TV |
| M9 | With Steam's setting on and Steam open: Fix it now restarts Steam in the background, and the PS button no longer brings Steam forward |
| M10 | With that setting on and Steam closed: starting Sofa turns it off and says so |
| M11 | A game that exits on its own brings Sofa back with nothing in Running |
| M12 | Sleep from System, wake the PC: Sofa is in front and draws |
| M13 | YouTube: Sofa's launch screen, then YouTube's own screen, no browser window or "press Esc" bubble; D-pad, Cross and Circle move, select and go back, Square plays and pauses, with Steam running and without |
| M14 | Discord: Home shows your channel with Mute, Deafen, Leave and recent channels; Mute flips at once and Discord agrees; a server's sheet joins a channel |
| M15 | A Running card for a browser shows the browser's name and icon; the hero shows the page |

## Results

### 2.1.0 (2026-11-27)

- Automated UI run: 53 of 53 PASS (T1 now walks Social). Home: 66 MB working set, 86 MB private,
  17 threads, 0.94 % idle CPU (an XInput pad was connected: 60 Hz polling); 17.5 MB after giving
  memory back; first frame in 87 ms.
- Unit tests and `cargo clippy --workspace --all-targets`: all PASS, no warnings.
- D4 reproduced and fixed on the development PC with a scripted run (real app, not QA mode):
  before, PS made Sofa the foreground window but left it minimized; after, it is restored and
  in front. Cause: while busy, every window message was answered 0, WM_QUERYOPEN included.
- D1 with Firefox in front: the card reads "Firefox" with Firefox's icon (was the page title
  and a blank file icon).
- YouTube, full screen at 3440 x 1440 with Chrome: launch screen, then YouTube's splash and
  sign-in; PS brings Sofa back with a YouTube Running card. Chrome's kiosk mode (no "press Esc"
  bubble) is not yet seen on screen: M13.
- The controller in YouTube (first report: Cross did nothing): Steam's desktop layout sent arrows
  and Space for the pad, and the first remote gave way to it entirely. Now Sofa holds back other
  programs' synthetic input while YouTube's TV window is in front; the opt-in test
  (`cargo test --bin sofa -- --ignored`) PASS: another program's key is stopped, Sofa's passes,
  and keys pass again after.
- Build, install: B1 to B3 PASS. `dist\Sofa-Setup-2.1.0.exe` (1.9 MB, version resource 2.1.0)
  replaced the installed build: Installed apps shows 2.1.0, the Start menu shortcut and the
  start-with-Windows entry point at the installed file, and Sofa started from it. B4 not rerun.
- Manual checks M1 to M15: not walked for this release. An earlier build of it was tried on the
  TV (YouTube, where Cross did nothing); the fix below was not tried on the TV before release.
- Discord: with the sign-in Sofa 1.x saved, the Social tab lists the account's servers with
  their icons and Home lists the recent channels. Joining, muting and the Connect flow: M14.

### 2.0.0

- Automated UI run: 53 of 53 PASS. Home: 62 MB working set, 84 MB private, 17 threads, 0.3 % idle CPU; 17 MB working set after giving memory back; first frame in 80 ms.
- Unit tests: VDF 172 of 172 recorded cases, Steam G1 to G15, all PASS. `cargo clippy --workspace --all-targets`: no warnings.
- Build, install, uninstall: B1 to B4 PASS on the development PC. The setup replaced the installed Electron 1.3.0 in under 7 seconds: its processes, its Installed apps entry and its start-with-Windows entry were gone, `sofa.json` was unchanged, and Sofa 2.0.0 was running at 54 MB. `--uninstall --quiet` removed the program folder, shortcut, start-with-Windows and Installed apps entries and kept the settings; Sofa was then installed again.
- Desktop checks D1 to D4: not run. The PC was on its screen saver during the release run (input desktop "Screen-saver"), where no window can take the focus. Found and fixed on the way: with no window in front, Sofa now takes the focus back itself (with a 5 s back-off).
- Manual: not yet walked for 2.0.0.

### 1.3.0 (Electron)

- Automated UI run: 34 of 34 PASS. Steam settings editor: 15 of 15 PASS. Helper: 5 of 5 PASS.
- Build and install: B1, B2 and B3 PASS.

### 1.2.0 and 1.1.0 (Electron)

- Automated UI runs: 32 of 32 and 28 of 28 PASS. See the git history of this file for details.
