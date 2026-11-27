# Sofa: product

## What it is

Sofa turns a Windows gaming PC on the TV into something that feels like a console.
It is a full-screen home screen for the couch: every Steam game, RecompLauncher
port, streaming service and Start menu app on one screen, driven entirely from a
controller. Windows stays out of sight until you choose to leave.

Since 2.0 Sofa is one native executable (Rust, Direct2D): no browser engine and no
helper processes, so the home screen itself costs almost nothing while you play.

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
   first, like a PS5.
3. **Everything from the controller.** Navigation, search with an on-screen
   keyboard (with Paste), item options, settings and the first-run screen all work
   with the pad. The keyboard works too.
4. **Your library, sorted.** Games, Media, Apps and System sections fill
   themselves from Steam, RecompLauncher and the Start menu. Anything can be moved,
   renamed, hidden or favorited.
5. **Calm and light.** Warm graphite surfaces, one amber accent, and colour that
   comes from the games' own art. No lag while moving, no flicker, no surprises,
   and no memory taken from the game you are playing.

## Features (2.1)

- Home rows: Running, Discord voice, Jump back in, Favorites, Games, Media, Apps
  (each long row ends with an "All ..." tile).
- Running cards name the program itself (Firefox, not the page it shows) with its
  own icon; the page or document shows beside "Running" in the hero.
- PS button home with the running app first; resume, close, force close; switch
  prompt when starting a second game.
- Stays in front: the desktop, taskbar, Start menu and Search showing up bring
  Sofa back; a full-screen game closing brings Sofa back. Go to desktop rests this
  until the PS button.
- Steam: turns off "Guide button focuses Steam" (silently while Steam is closed,
  after asking while it is open) and pushes Steam back down if it grabs the screen.
- DualSense without drivers over USB or Bluetooth: buttons (also while a game has
  focus), battery and charging. XInput pads with the Guide button.
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
- First-run screen: start with Windows, keep Steam off the PS button, button layout.
- Interface sounds, reduced motion, notices (controller connected, battery low,
  Steam fixed).
- Go to desktop and Quit Sofa; Sleep, Restart, Shut down; Steam Big Picture;
  display, sound and Bluetooth settings.
- Installs itself (one file, per user, no admin), replaces Sofa 1.x keeping its
  settings, and uninstalls from Windows' Installed apps.

## Not yet in 2.x (were in 1.x)

These came with the Electron app and are planned for the native one. Each needs a
native replacement for something the browser engine gave for free.

- **Headphones in the controller take the sound**, and the sound output switcher:
  needs Windows' audio endpoint API (the policy config interface).
- **DualSense lightbar, player light and mute LED**: needs HID output reports
  (careful: output reports on every move caused lag over Bluetooth in 1.x).
- **Custom covers** from Pictures, Downloads or the Desktop, and **uninstalling** games
  and apps from their options.
- The longer 1.x setup (controller check, Windows settings worth changing).

## Deliberately not in Sofa

- **Steam friends.** Steam's Web API needs a personal key and a public friends
  list, and failed for real users. Removed in 1.1.
- **Phone link.** The TwinDesk bridge needed a custom phone build and never linked
  reliably. Removed in 1.1.
- **Virtual controller drivers (ViGEm, HidHide).** Sofa reads the DualSense
  directly instead, so nothing extra is installed.
- **A browser engine.** YouTube TV and streaming open in the browser you already
  have, with its own TV profile (ADR-004), instead of inside Sofa.

## Constraints we live with

- **Steam** also listens to the PS button while its "Guide button focuses Steam"
  setting is on. Sofa turns that setting off itself: silently while Steam is closed,
  or after asking while Steam is open, because Steam must restart to pick it up.
  Until then Sofa pushes Steam back down whenever it grabs the screen.
- **Windows only.** Sofa is built on Win32, Direct2D and DirectWrite.
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
