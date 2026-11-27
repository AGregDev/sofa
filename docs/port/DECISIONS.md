# Decisions for the Rust port of Sofa

Each decision is an ADR: numbered in order, never renumbered. A changed decision gets a new ADR
that supersedes the old one. This file is the source of truth for the port.

## ADR-001: A native Rust app, no web engine
Status: accepted (2026-11-27)
Context: Sofa 1.3.0 is Electron. Measured on the target PC with Sofa idle on Home: 483 MB working
set across Electron's 5 processes plus 81 MB for the PowerShell helper (564 MB in all, 333 MB
private), and 358 MB installed. rust-anything's analysis proposed keeping the web UI (its D2); the
user chose the lowest-RAM option instead.
Decision: Sofa becomes one native Rust executable. Windows draws it through Direct2D, DirectWrite
and WIC (the `windows` crate), with no web engine and no PowerShell. The renderer's HTML, CSS and
JS stay in the repo only as the reference for how every screen looks and behaves.
Consequences: every screen is rebuilt in Rust and checked against the Electron app's screenshots
and the QA list. Expected 60 to 120 MB of RAM and about 10 MB on disk, to be measured, not assumed.

## ADR-002: Logic units go through rust-anything; the UI is checked by QA
Status: accepted (2026-11-27)
Context: rust-anything records and replays functions, CLIs and servers. Sofa's logic (Steam's
`.vdf` files, the Steam settings editor, library sorting, Discord framing, the settings store) is
made of functions it can record. Its UI is not.
Decision: logic units follow rust-anything's loop (bridge, record, port, build, diff, gate) and
their Rust is used by the native app. The UI and the Windows integration (input, windows, audio,
focus) are checked by a native QA run modelled on `src/shots.js` and `QA.md`.
Consequences: `rust/<unit>-rs` crates hold the verified logic; `rust/app` is the native app and
depends on them.

## ADR-003: Keep Sofa's settings and data
Status: accepted (2026-11-27)
Context: people upgrading must keep their setup.
Decision: the Rust app reads and writes the same `%APPDATA%\sofa\sofa.json` (same keys and
defaults), the same cache folders (`cache\covers`, `cache\icons`, `cache\library.json`) and the
same encrypted Discord sign-in. Secrets stay encrypted with Windows' data protection (DPAPI),
which is what Electron's `safeStorage` uses on Windows.
Consequences: the JSON keys in `src/store.js` and the cover and icon paths are a compatibility
contract. Electron's `safeStorage` wraps DPAPI with its own key file, so ADR-003 is met when the
Rust app can read existing secrets; if it cannot, it asks the person to reconnect Discord once.

## ADR-004: YouTube opens in TV mode in the browser
Status: accepted (2026-11-27)
Context: Electron hosted YouTube's TV interface in its own window. A native app has no web engine.
Decision: YouTube opens `https://www.youtube.com/tv` full screen in the chosen browser with a TV
profile, and Sofa turns the controller into the remote keys (arrows, Enter, Escape) while that
window is in front.
Consequences: no in-app web window; the browser's TV profile keeps its own sign-in.

## ADR-005: Build beside 1.3.0, replace it when QA passes
Status: superseded by ADR-008 (2026-11-27)
Context: 1.3.0 is the working release.
Decision: the Rust app lives in `rust/` next to the Electron app. It replaces 1.3.0 as Sofa 2.0
only when it passes the same QA list (QA.md). Until then the Electron app stays the release.
Consequences: main keeps building both. No release ships a Rust Sofa that fails QA.

## ADR-006: No checkpoints; report with results
Status: accepted (2026-11-27)
Decision: waves run without stopping for review. Each report gives gate and diff results, the
native QA results, measured RAM and what is left.

## ADR-007: Toolchain and dependencies
Status: accepted (2026-11-27)
Context: the PC has no Visual Studio or C compiler. The GNU Rust toolchain links pure Rust code
on its own.
Decision: Rust stable on `x86_64-pc-windows-gnu`. Dependencies stay pure Rust: `windows` (Win32,
COM, Direct2D, DirectWrite, WIC, XInput, HID, WinHTTP, DPAPI), `serde` and `serde_json`, `regex`.
No crate that needs a C compiler. HTTPS goes through WinHTTP instead of a TLS crate.
Consequences: a small static binary; no OpenSSL or ring.

## ADR-008: The native app is Sofa; the Electron app is removed
Status: accepted (2026-11-27). Supersedes ADR-005.
Context: the owner chose the native app as the version to go with ("keep the rust native app
only") and asked for it to work as a full-screen launcher opened with the PS button. Keeping
both apps building in one repository doubled every change.
Decision: the Electron sources (`main.js`, `preload*.js`, `renderer/`, `src/*.js`, `scripts/`,
`setup.ps1`, `package.json`) are removed from the tree; they stay in git history and in the
v1.3.0 release. The native app moves from `rust/app` to `app/`, the lifted crates to `crates/`
(`vdf`, `steam`), and this folder from `rust/` to `docs/port/`. Sofa 2.0 ships the native app
with the 1.x features it does not have yet listed in PRODUCT.md. The Electron app's other
behaviour that the native app needed was ported by hand and is proven by the native QA run
(`qa/qa.ps1`) instead of rust-anything recordings: `steamguide.js` became `crates/steam` with
the same G1 to G15 tests, the Start menu scan became `app/src/startmenu.rs`, the helper's
window logic became `app/src/shell.rs`.
Consequences: one build, one QA run. Installing 2.0 removes 1.x and keeps `%APPDATA%\sofa`
(ADR-003 still holds). `PORT_MAP.md` and `ANALYSIS.md` describe the port as it was while the
JavaScript existed, and are no longer regenerated.
