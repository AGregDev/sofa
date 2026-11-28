# Sofa: product

## What it is

Sofa turns a gaming PC on the TV into something that feels like a console, on Windows
and, since 2.3, on Linux.
It is a full-screen home screen for the couch: every Steam game, RecompLauncher
port, streaming service and Start menu app on one screen, driven entirely from a
controller. Windows stays out of sight until you choose to leave.

Since 2.0 Sofa is one native executable (Rust; Direct2D on Windows, an X11 window drawn
on the CPU on Linux): no browser engine and no helper processes, so the home screen itself
costs almost nothing while you play.

## Who it is for

Someone who plays on a PC connected to a TV, from the couch, with a DualSense or
any other controller, and who does not want to reach for a mouse and keyboard
to start a game or switch apps.

## The experience we promise

1. **Console, not app.** Sofa starts with Windows and stays in front like a home
   screen. The desktop, taskbar and Start menu do not appear unless you choose
   Go to desktop. The PS button always comes back to Sofa.
2. **PS button = Home.** From any game or app, PS opens Sofa with that app first,
   ready to Resume, Close or Force close. Picking another game while one runs asks
   first, like a PS5. On Home, PS opens the quick menu: what is playing, sound,
   microphone, the sleep timer and power, without leaving the couch.
3. **Everything from the controller.** Navigation, search with an on-screen
   keyboard (with Paste), item options, settings and the first-run screen all work
   with the pad. The keyboard works too.
4. **Your library, sorted.** Games, Media, Apps and System sections fill
   themselves from Steam, RecompLauncher and the Start menu. Anything can be moved,
   renamed, hidden or favorited.
5. **Calm and light.** Warm graphite surfaces, one amber accent, and colour that
   comes from the games' own art. No lag while moving, no flicker, no surprises,
   and no memory taken from the game you are playing.

## Features (2.3)

- Home rows: Running, Discord voice, Jump back in, Favorites, Games, Media, Apps
  (each long row ends with an "All ..." tile). ○ on Home returns to the first tile.
- The hero says what matters before you press ✕: how long a running game has been
  going ("Running for 1 h 12 min"), and for Steam games when you last played and
  your total play time.
- **Quick menu** (PS on Home; PS or ○ closes it): a large clock with the date, when
  the PC will sleep and the controller's charge, then
  - **Now playing**: the track and its art from Spotify, a browser or any player in
    Windows' media flyout; ✕ plays or pauses, L1 and R1 skip. Players built for a
    mouse (Spotify) are controlled without touching their window.
  - **Sound output**: TV, headphones, speakers or the controller, by name.
  - **Volume** and mute (L2 and R2 change the volume anywhere in Sofa, shown in
    the top bar).
  - **Microphone** mute (in Discord while you are in a voice channel).
  - **Sleep timer**: 30 min, 1 h, 1 h 30 min, 2 h. A minute before the end Sofa comes
    forward with a countdown (Stay awake, Sleep now); games and videos wait in sleep.
  - **Controller light** and **Power** (sleep, restart, shut down, desktop, quit).
  - **Performance**: CPU and GPU use, memory and each graphics card's video memory.
    **Show on screen** keeps them, and the frame rate of the game in front, in a one-line
    overlay in the top left corner, a window of its own over every game and app (not over
    exclusive full screen). Read only while they show. The frame rate needs Windows'
    Performance Log Users group, as Game Bar's does: Sofa offers to ask Windows once, and it
    counts from the next sign-in. DirectX 9 to 12 games; Vulkan and OpenGL show none yet.
  - ≡ opens Settings from here.
- Running cards name the program itself (Firefox, not the page it shows) with its
  own icon; the page or document shows beside "Running" in the hero.
- PS button home with the running app first; resume, close, force close; switch
  prompt when starting a second game. A game comes forward once ✕ is let go, so it
  never reads the press; one already open resumes from its tile too.
- Stays in front: the desktop, taskbar, Start menu and Search showing up bring
  Sofa back; a full-screen game closing brings Sofa back. Go to desktop rests this
  until the PS button.
- Steam: turns off "Guide button focuses Steam" (silently while Steam is closed,
  after asking while it is open) and pushes Steam back down if it grabs the screen.
- DualSense without drivers over USB or Bluetooth: buttons (also while a game has
  focus), battery and charging. XInput pads with the Guide button.
- DualSense lights: the lightbar takes the focused game's colour once focus rests
  (or amber, or off), player 1's light shows, and the **mute button** mutes the
  microphone from any game with its LED showing it, like a PS5. Sofa writes only
  on change, never on focus moves, and the lightbar only while Sofa is in front.
- While Sofa is in front it reads the controller itself, so keys other programs make
  from the same buttons (Steam's desktop layout) are held back: ○ is always Back.
- Picking an app that is already open switches to it instead of starting a second copy.
- Button layouts: Match controller, PlayStation, Xbox (A B X Y) or Nintendo
  (confirm and back swapped). Home button: PS, or Create + Options.
- Search with an on-screen keyboard (Shift, Space, Delete, Paste, Clear, Done) or a
  physical keyboard; rename with the same keyboard.
- Streaming: YouTube in its TV interface, Netflix, Disney+, Prime Video, Crunchyroll and Twitch
  full screen in Edge, Chrome or Firefox (Automatic picks them in that order), each
  with Sofa's own browser profile. Sofa's launch screen stays up until the service is
  ready (no browser window opening), the page is sized for the screen (sharp on 1440p
  and 4K), and the controller works YouTube's TV interface as its remote (other
  programs' synthetic keys, such as Steam's desktop layout, are held back meanwhile). Tiles show
  each service's own colour and mark.
- Discord voice: your channel with Mute, Deafen and Leave, channels you joined
  recently (on Home), your servers and their voice channels (Social tab), and a
  one-time Connect walkthrough. A sign-in made in Sofa 1.x carries over.
- **Spotify from the controller** (Music tab, and a Music row on Home): Liked Songs,
  your playlists and saved albums with their covers; ✕ plays one on this PC's Spotify
  app (started when needed), △ offers Shuffle play and Open in Spotify. A one-time
  Connect walkthrough: you make a Spotify app (Spotify allows apps in development
  only for accounts with Premium) and approve Sofa once in the browser; Sofa never
  sees the password and keeps the sign-in encrypted for this Windows account.
- First-run screen: start with Windows, keep Steam off the PS button, button layout; with an
  Xbox controller also "Keep Game Bar off the Xbox button" (Windows' Game Bar otherwise opens
  on the same button). Text names the button on the controller in use (PS or Xbox button).
- Interface sounds, reduced motion, notices (controller connected, battery low,
  Steam fixed).
- Go to desktop and Quit Sofa; Sleep, Restart, Shut down; Steam Big Picture;
  display, sound and Bluetooth settings.
- Installs itself (one file, per user, no admin), replaces Sofa 1.x keeping its
  settings, and uninstalls from Windows' Installed apps.

## Not yet in 2.x (were in 1.x)

These came with the Electron app and are planned for the native one. Each needs a
native replacement for something the browser engine gave for free.

- **Headphones in the controller take the sound** by themselves: needs endpoint
  notifications (the quick menu already switches to the controller by hand).
- **Custom covers** from Pictures, Downloads or the Desktop, and **uninstalling** games
  and apps from their options.
- The longer 1.x setup (controller check, Windows settings worth changing).

## Linux (2.3)

The same program for Linux: one AppImage, `Sofa-Setup-<version>-x86_64.AppImage`, that
installs itself for the person (`~/.local/bin/sofa`, a desktop entry with an Uninstall
action, start with the session) with no root, or a build from source. Aimed first at Ubuntu
and Debian with GNOME, and at Arch-based systems, KDE and other window managers; X11 sessions
and Wayland sessions with XWayland (every GNOME and KDE session has it). Built on Ubuntu 20.04,
so glibc 2.31 or later.

**Works as on Windows**
- Home, sections, search with the on-screen keyboard and Paste, item options, settings, the
  first-run screen, the quick menu, the sleep timer, Sleep, Restart, Shut down (systemd).
- Performance and its overlay (override-redirect, over every window); GPU use and video memory
  from NVIDIA's driver (NVML) and from amdgpu, none for other cards (Intel's, nouveau).
- Every pad through evdev, read whichever window has the focus: DualSense, Xbox and other
  xpad pads, Switch Pro; the PS (Guide) button brings Sofa forward from inside a game with that
  game first, and ✕ hands it back once the button is up. The keyboard works too.
- Steam games from a native, Flatpak or Snap Steam, with art, last played and play time;
  desktop apps from their desktop entries (Flatpak and Snap apps included) with icons from the
  icon theme; streaming in Edge, Chrome, Chromium or Firefox with Sofa's own profile (also when
  the browser is a Snap or a Flatpak), and the controller as YouTube's remote.
- Running cards with the program's name and icon, close and force close, picking an open app
  switches to it; Steam pushed back down when it grabs the screen after PS; Steam's "Guide
  button focuses Steam" turned off as on Windows.
- Now playing from the desktop's media players (MPRIS); sound outputs (the TV by its own name),
  volume and the microphone through PulseAudio or PipeWire (`pactl`); Discord voice through
  Discord's socket (package, Flatpak or Snap); Spotify, its sign-in sealed with a key only the
  person's account can read (`~/.config/sofa/key`).
- The DualSense's battery; its lightbar, player light, mute LED and mute button when its
  hidraw device may be opened (Steam's udev rules, the `steam-devices` package).
- Settings in `~/.config/sofa/sofa.json`, with the same keys as on Windows.

**Not on Linux yet**
- **Wayland apps.** Sofa is an X11 window, and under Wayland X11 sees only X11 windows (Steam,
  Proton and most SDL games, and the browsers Sofa starts, which it starts on X11). An app
  that draws through Wayland itself (GNOME's own apps, a browser started outside Sofa) gets no
  Running card, and PS over it opens Sofa without it first. GNOME offers no other way without a
  Shell extension; a small extension of Sofa's own is the planned fix.
- **The guard.** A desktop or panel window coming forward brings Sofa back; GNOME's Activities
  overview and other Wayland surfaces cannot be seen, so they do not.
- **Ctrl+Alt+H** from anywhere; the PS button and the Home key work.
- **Holding back other programs' synthetic keys** (Steam's desktop layout turning ○ into a
  key). X11 cannot drop them, so a key that arrives while a pad button is down counts as that
  press instead.
- **The DualSense without Steam's udev rules**: no lights and no mute button (the kernel keeps
  the mute button to itself); everything else works.
- **The frame rate in the performance overlay.** Windows tells Sofa the frames a game presents;
  nothing on Linux tells one program another's (MangoHud draws inside the game instead).
- RecompLauncher (a Windows launcher) and the Game Bar setting (Windows only).
- **gamescope and the Steam Deck's Game Mode**: gamescope decides which app is in front; Sofa
  runs in desktop sessions.
- **Fractional scaling under Wayland** (125 %, 150 %): XWayland windows are scaled up and look
  soft. 100 % or 200 % keeps Sofa sharp.
- Sofa draws on the CPU here: smooth at 1080p; on a 4K screen at 100 % scaling its animations
  cost more CPU than on Windows.

**Needs** X11 or XWayland, FUSE to run the AppImage (or `--appimage-extract-and-run`),
`pactl` for the sound controls, fontconfig for characters Figtree lacks (CJK titles), and
`zenity` or `kdialog` for the uninstall question.

## Deliberately not in Sofa

- **Steam friends.** Steam's Web API needs a personal key and a public friends
  list, and failed for real users. Removed in 1.1.
- **Phone link.** The TwinDesk bridge needed a custom phone build and never linked
  reliably. Removed in 1.1.
- **Virtual controller drivers (ViGEm, HidHide).** Sofa reads the DualSense
  directly instead, so nothing extra is installed.
- **A browser engine.** YouTube TV and streaming open in the browser you already
  have, with its own TV profile (ADR-004), instead of inside Sofa.
- **Spotify's TV interface.** Unlike YouTube, Spotify has no TV interface a PC
  browser can open; its web player is built for a mouse. Sofa shows your Spotify
  library itself (Music tab) and controls playback (quick menu) instead.

## Constraints we live with

- **Steam** also listens to the PS button while its "Guide button focuses Steam"
  setting is on. Sofa turns that setting off itself: silently while Steam is closed,
  or after asking while Steam is open, because Steam must restart to pick it up.
  Until then Sofa pushes Steam back down whenever it grabs the screen.
- **Windows and Linux only.** Sofa is built on Win32, Direct2D and DirectWrite on Windows, and
  on X11 (EWMH), evdev and freedesktop.org's interfaces on Linux (see "Linux (2.3)").
- **Unsigned.** SmartScreen warns on first run until the file has a reputation.

## Quality bar

- Every screen reachable and usable from a controller; the hint bar only offers
  what works on the focused item.
- About 60 MB of memory on Home, under 20 MB while a game runs, 0 % CPU when idle,
  first frame in well under a second (checked by `qa\qa.ps1`).
- Never redraw unless something changed; never move focus under the person.
- Copy is plain and specific: controls name their action; errors say what to do.

## Visual identity

Living room at night. Warm graphite (`#161513` base), cream text, one lamp-amber
accent (`#f0a24b`) used for state only: the current tab, switches, progress, the
running dot, the text caret. Game art supplies the colour. No gradients as
decoration (scrims over art only, for legibility), no glows, no emoji. Figtree.
Real controller keycaps for every hint.
