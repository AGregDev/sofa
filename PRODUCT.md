# Sofa: product

## What it is

Sofa turns a Windows gaming PC on the TV into something that feels like a console.
It is a full-screen home screen for the couch: every Steam game, RecompLauncher
port, streaming service and Start menu app on one screen, driven entirely from a
controller. Windows stays out of sight until you choose to leave.

## Who it is for

Someone who plays on a PC connected to a TV, from the couch, with a DualSense or
any other controller, and who does not want to reach for a mouse and keyboard
to start a game, switch apps or join friends in voice.

## The experience we promise

1. **Console, not app.** Sofa starts with Windows and stays in front like a home
   screen. The desktop, taskbar and Start menu do not appear unless you choose
   Go to desktop. The PS button always comes back to Sofa.
2. **PS button = Home.** From any game or app, PS opens Sofa with that app first,
   ready to Resume, Close or Force close. Picking another game while one runs asks
   first, like a PS5.
3. **Everything from the controller.** Navigation, search with an on-screen
   keyboard (with Paste), item options, covers, uninstalling, sound output,
   browser choice and setup all work with the pad. The keyboard works too.
4. **Your library, sorted.** Games, Media, Apps and System sections fill
   themselves from Steam, RecompLauncher and the Start menu. Anything can be moved,
   renamed, hidden, favorited or given a custom cover.
5. **Calm and fast.** Warm graphite surfaces, one amber accent, and colour that
   comes from the games' own art. No lag while moving, no flicker, no surprises.

## Features

- Home rows: Running, Discord voice, Jump back in, Favorites, Games, Media, Apps
  (each long row ends with an "All ..." tile).
- PS button home with the running app first; resume, close, force close; switch
  prompt.
- DualSense without drivers: battery and charging in the top bar, lightbar in the
  selected game's colour, player light, mute button (mutes Discord voice, else the
  Windows microphone) with its LED.
- Button layouts: Match controller, PlayStation, Xbox (A B X Y) or Nintendo
  (confirm and back swapped).
- Discord voice: join recent channels from Home, browse servers' voice channels,
  mute, deafen, leave. One-time guided setup; the sign-in is kept.
- Streaming: YouTube in its TV interface inside Sofa; Netflix, Disney+, Prime
  Video and Twitch full screen in Firefox (kiosk profile), Chrome or Edge.
- Uninstall games and apps from their options.
- Sound output switcher, including the DualSense headphone jack over USB.
- Go to desktop (the PS button brings Sofa back) and Quit Sofa.

## Deliberately not in Sofa

- **Steam friends.** Steam's Web API needs a personal key and a public friends
  list, and failed for real users. Removed in 1.1.
- **Phone link.** The TwinDesk bridge needed a custom phone build and never linked
  reliably. Removed in 1.1.
- **Virtual controller drivers (ViGEm, HidHide).** Sofa reads the DualSense
  directly instead, so nothing extra is installed.

## Constraints we live with

- **Discord** only lets an application control voice for the people who created
  it, so each person makes a small Discord application once (two minutes, guided
  in Sofa). There is no shared "Sofa" application that could work for everyone.
- **Steam** also listens to the PS button. Sofa pushes Steam back down when it
  grabs the screen, and asks the user once to turn off Steam's "Guide button
  focuses Steam" setting, which removes the conflict for good.
- **The DualSense headphone jack** only works over USB on Windows.
- **Windows only.** Sofa uses PowerShell helpers for the joystick API, window
  focus and audio endpoints.

## Quality bar

- Every screen reachable and usable from a controller; the hint bar only offers
  what works on the focused item.
- Moving focus is handled in well under a frame; no main-thread task over 100 ms
  while browsing (checked by `npm run qa`).
- No console errors in a full QA run.
- Copy is plain and specific: controls name their action; errors say what to do.

## Visual identity

Living room at night. Warm graphite (`#161513` base), cream text, one lamp-amber
accent (`#f0a24b`) used for state only: the current tab, switches, progress,
focus inside the keyboard. Game art supplies the colour. No gradients as
decoration (scrims over art only, for legibility), no glows, no emoji. Figtree,
tabular numbers. Real controller keycaps for every hint.
