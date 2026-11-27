# Port map

**Goal:** Sofa as one native Rust executable with no web engine and the lowest RAM it can
reach (ADR-001), proven against the Electron original: logic units by rust-anything's recorded
cases (ADR-002), the app by the native QA run (`../../qa/qa.ps1`, listed in `../../QA.md`).
Since ADR-008 the native app is Sofa (2.0) and the Electron sources are gone from the tree.

Decisions: [DECISIONS.md](DECISIONS.md) (ADR-001 native UI, ADR-002 how each part is proven,
ADR-003 keep settings and data, ADR-004 YouTube TV in the browser, ADR-005 superseded,
ADR-006 no checkpoints, ADR-007 toolchain, ADR-008 native only). Terms:
[GLOSSARY.md](GLOSSARY.md). Next: [HANDOFF.md](HANDOFF.md).

## Logic units

| Original | Rust | State | Proof |
| --- | --- | --- | --- |
| `src/vdf.js` | `crates/vdf` | done | rust-anything gate passed; 172/172 recorded cases replayed by `cargo test` |
| `src/steamguide.js` | `crates/steam` | done (by hand, ADR-008) | G1 to G15 in `crates/steam/tests/guide.rs` |
| `src/library.js` | `app/src/library.rs` | done (by hand) | QA H1, L1; same `cache\` files |
| `src/store.js` | `app/src/store.rs` | done (by hand), same keys and defaults | QA W4, C3 to C7, Q4 |
| `src/social.js` | | not yet (Discord) | |

## App

| Part | Original | State |
| --- | --- | --- |
| Window, message loop, single instance, installer, uninstaller | `main.js`, electron-builder | done |
| Renderer: Direct2D, DirectWrite (Figtree), WIC images, Lucide icons | Chromium + `style.css` | done |
| Library: Steam, RecompLauncher, Start menu (child process scan), icons | `library.js`, `apps.ps1`, `icon.ps1` | done |
| Home rows, section grids, hero, tabs, hints, notices, sounds, start-up | `app.js`, `sound.js` | done |
| Item options (favorite, move, rename, hide), settings, welcome | `app.js` | done |
| Search with the on-screen keyboard, paste, physical typing | `app.js` | done |
| Launching (Steam, Start apps, programs, browsers, YouTube TV) | `main.js` | done |
| Keyboard, XInput (Guide), DualSense over HID, PS button from games | `input.js`, `dualsense.js`, `padwatch.ps1` | done |
| Running card, resume, close, force close, switch prompt | `main.js`, `padwatch.ps1` | done |
| Staying in front, launcher guard, Steam push-down and fix | `main.js`, `padwatch.ps1`, `steamguide.js` | done |
| Giving memory back in the background | (new) | done |
| Native QA run and screenshots | `src/shots.js` | done (`qa/qa.ps1`) |
| Discord voice (RPC pipe, OAuth, DPAPI secrets) | `src/social.js` | not yet |
| Sound output, mic mute, controller headphones | `audio.ps1`, `app.js` | not yet |
| DualSense output: lightbar, player light, mute LED | `dualsense.js` | not yet |
| Custom covers, uninstalling games and apps | `main.js`, `uninstall.ps1` | not yet |

## Measured

| | Electron 1.3.0 | Sofa 2.0.0 |
| --- | ---: | ---: |
| Working set, Home | 564 MB (5 processes + helper) | about 62 MB (1 process) |
| Private memory, Home | 333 MB | about 85 MB |
| Working set behind a game | same | about 17 MB |
| Idle CPU | helper polling every 30 ms | 0 % |
| On disk | 358 MB installed | 1.8 MB, one file |

Measured on the same PC and library by `qa\qa.ps1` (1600 x 900 window).

## Out of scope

- The in-app YouTube window (ADR-004: YouTube TV opens in the browser).
- Steam friends and the phone link (removed in 1.1, PRODUCT.md).
