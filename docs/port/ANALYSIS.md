# Rust migration plan: scratch-2026-11-26-cd3351 (TypeScript/JavaScript)

1 package, 15 source files (3314 lines), 1 test file, 0 UI files, grouped into 10 units.

| Ready | Caution | Blocked | Keep in source language |
| ---: | ---: | ---: | ---: |
| 8 | 2 | 0 | 0 |

## Start here

Best first ports: safe to port (readiness) and worth porting (loops and string work, tests, size).

| # | Unit | Lines | Priority | Readiness | Tests | Bridge | Main risks |
| ---: | --- | ---: | ---: | ---: | ---: | --- | --- |
| 1 | `src/vdf.js` | 52 | 85 | 91 | 0 | napi-rs | String offsets and slicing (1); Dynamic typing (1); Insertion-ordered maps and object keys (1) |
| 2 | `src/steamguide.js` | 83 | 78 | 77 | 1 | napi-rs | String offsets and slicing (3); Regex features the `regex` crate lacks (1); Async and promises (12) |
| 3 | `src/library.js` | 264 | 56 | 56 | 0 | napi-rs | Regex features the `regex` crate lacks (7); String offsets and slicing (1); Regexes built at runtime (1) |
| 4 | `src/social.js` | 232 | 55 | 66 | 0 | napi-rs | String offsets and slicing (3); Async and promises (42); Insertion-ordered maps and object keys (2) |

### `src/vdf.js`

52 lines in 1 file, wave 0, priority 85, readiness 91, bridge **napi-rs**.

- Leaf: imports no other unit.
- String offsets and slicing x1: `src/vdf.js:21`
- Dynamic typing x1: `src/vdf.js:55`
- Insertion-ordered maps and object keys x1: `src/vdf.js:57`

### `src/steamguide.js`

83 lines in 1 file, wave 1, priority 78, readiness 77, bridge **napi-rs**.

- Imports `src/vdf.js`: port it first, or port them together.
- 1 test file imports it directly.
- String offsets and slicing x3: `src/steamguide.js:36`, `src/steamguide.js:39`, `src/steamguide.js:40`
- Regex features the `regex` crate lacks x1: `src/steamguide.js:66`
- Async and promises x12: `src/steamguide.js:65`, `src/steamguide.js:69`, `src/steamguide.js:72`
- Insertion-ordered maps and object keys x3: `src/steamguide.js:43`, `src/steamguide.js:47`, `src/steamguide.js:48`
- Dynamic typing x1: `src/steamguide.js:48`
- Exceptions x6: `src/steamguide.js:20`, `src/steamguide.js:37`, `src/steamguide.js:44`
- Clock and random numbers x2: `src/steamguide.js:70`, `src/steamguide.js:71`

### `src/library.js`

264 lines in 1 file, wave 1, priority 56, readiness 56, bridge **napi-rs**.

- Imports `src/vdf.js`: port it first, or port them together.
- Regex features the `regex` crate lacks x7: `src/library.js:20`, `src/library.js:173`, `src/library.js:174`
- String offsets and slicing x1: `src/library.js:182`
- Regexes built at runtime x1: `src/library.js:160`
- Insertion-ordered maps and object keys x8: `src/library.js:21`, `src/library.js:36`, `src/library.js:61`
- Async and promises x3: `src/library.js:151`, `src/library.js:253`, `src/library.js:255`
- Dynamic typing x1: `src/library.js:296`
- Exceptions x10: `src/library.js:11`, `src/library.js:24`, `src/library.js:31`
- JSON round-trips x4: `src/library.js:116`, `src/library.js:155`, `src/library.js:291`
- Number semantics x2: `src/library.js:247`
- Clock and random numbers x1: `src/library.js:301`

### `src/social.js`

232 lines in 1 file, wave 0, priority 55, readiness 66, bridge **napi-rs**.

- 1 npm dependencies not in the crate table: check them by hand.
- Leaf: imports no other unit.
- String offsets and slicing x3: `src/social.js:14`, `src/social.js:15`, `src/social.js:227`
- Async and promises x42: `src/social.js:63`, `src/social.js:102`, `src/social.js:104`
- Insertion-ordered maps and object keys x2: `src/social.js:29`, `src/social.js:237`
- Exceptions x5: `src/social.js:14`, `src/social.js:116`, `src/social.js:136`
- Clock and random numbers x3: `src/social.js:103`, `src/social.js:118`, `src/social.js:151`
- JSON round-trips x2: `src/social.js:76`, `src/social.js:88`

## Order by package

Packages come after the workspace packages they use. Inside a package, port by wave: a unit's imports are in earlier waves.

### 1. sofa (`.`, node, 1 test file)

| Wave | Unit | Status | Lines | Readiness | Bridge | Main risks |
| ---: | --- | --- | ---: | ---: | --- | --- |
| 0 | `src/store.js` | ready | 42 | 100 | napi-rs | JSON round-trips (2); Exceptions (1) |
| 0 | `preload.js` | ready | 46 | 99 | napi-rs | - |
| 0 | `preload-web.js` | ready | 26 | 95 | napi-rs | Insertion-ordered maps and object keys (3) |
| 0 | `src/vdf.js` | ready | 52 | 91 | napi-rs | String offsets and slicing (1); Dynamic typing (1) |
| 0 | `src/social.js` | ready | 232 | 66 | napi-rs | String offsets and slicing (3); Async and promises (42) |
| 0 | `src/shots.js` | ready | 240 | 64 | napi-rs | String offsets and slicing (5); Async and promises (153) |
| 0 | `renderer/` | caution | 1695 | 7 | napi-rs | String offsets and slicing (12); Regex features the `regex` crate lacks (4) |
| 1 | `src/steamguide.js` | ready | 83 | 77 | napi-rs | String offsets and slicing (3); Regex features the `regex` crate lacks (1) |
| 1 | `src/library.js` | ready | 264 | 56 | napi-rs | Regex features the `regex` crate lacks (7); String offsets and slicing (1) |
| 2 | `main.js` | caution | 634 | 37 | napi-rs | String offsets and slicing (6); Regex features the `regex` crate lacks (2) |

## npm dependencies

| Package | Units | Rust | Fit | Note |
| --- | ---: | --- | --- | --- |
| `electron` | 5 | ? | unknown |  |

## Risk guide

- **String offsets and slicing** (High): Rust slices strings at UTF-8 byte offsets that fall on character boundaries. Convert offsets from the source's indexing units as needed and test with non-ASCII text.
- **Regex features the `regex` crate lacks** (High): Lookaround, backreferences and the `y` flag are not in the `regex` crate. Use `fancy-regex` for those patterns, or rewrite them, and test them on non-ASCII text: `\w`, `\b` and case-insensitive matching differ between engines even where a pattern compiles. JS `/i` without `u` never matches a non-ASCII letter to an ASCII one, Rust `(?i)` does (`ſ` matches `s`, the Kelvin sign `K` matches `k`): write `(?i-u)` or compare with `eq_ignore_ascii_case`.
- **Regexes built at runtime** (High): Patterns come from data or config in JS syntax. Rust regex syntax differs, so user-visible behavior can change. Pin it with tests on real patterns.
- **Dynamic typing** (Medium): Values whose shape is checked at runtime need an explicit Rust enum or `serde_json::Value`.
- **Insertion-ordered maps and object keys** (Medium): JS Map, Set and object keys keep insertion order. `HashMap` does not: use `indexmap` wherever order reaches output. With serde_json's `preserve_order`, `Map::remove` swaps the last key into the hole: use `shift_remove` for JS `delete`. Sort with `js::sort` when a comparator can return NaN: the order then depends on V8's algorithm.
- **Exceptions** (Low): Represent source exceptions with `Result`. Preserve the original exception type and message when errors cross the bridge.
- **Async and promises** (Medium): Promises become futures. Bridge with napi-rs `async fn` or `wasm-bindgen-futures`, or keep the async shell in JS and port the sync core.
- **Number semantics** (Low): JS numbers are f64; bitwise ops truncate to i32; parseInt is lenient. Pick Rust integer types deliberately. Printing differs: `${n}` gives `1e+21`, `1e-7` and `0` for -0 where Rust's `{}` does not, so use `js::number_to_string`. `Math.round` rounds halves up (-2.5 gives -2), Rust's `round` away from zero: use `js::math_round`. `toFixed` rounds exact halves up ((2.5).toFixed(0) is "3"), Rust's `{:.0}` to even.
- **JSON round-trips** (Low): JSON.stringify key order and number formatting must match `serde_json` output if the result is user-visible.
- **Clock and random numbers** (Low): Results that depend on the time or on random numbers differ on every run, so a plain recording cannot pin them down. Record with `record --pin-clock` (TS/JS with the napi bridge, Python): `Date.now()`, `new Date()` and `Math.random` (`time.time`, `datetime.now` and `random`) then return fixed values during record, diff and gate, and the port reads the same ones from `src/port/pinned.rs` (`now_ms()`, `random()`, a documented mulberry32). Or pass the time and the seed in as parameters.

## Whole app

Shape: **library**.

| Entry point | Kind | Found from |
| --- | --- | --- |
| sofa: `.` | library | no main and no server: the package's public API |

### Every dependency

| Dependency | Declared | Used by | Rust | Fit | Note |
| --- | --- | --- | --- | --- | --- |
| `electron` | no | 5 | ? | unknown |  |
| `@fontsource-variable/figtree` | yes | 0 | ? | unknown |  |

### Pieces, in port order

- wave 0: `preload-web.js` (26 lines)
- wave 0: `preload.js` (46 lines)
- wave 0: `renderer/` (1695 lines)
- wave 0: `src/shots.js` (240 lines)
- wave 0: `src/social.js` (232 lines)
- wave 0: `src/store.js` (42 lines)
- wave 0: `src/vdf.js` (52 lines)
- wave 1: `src/library.js` (264 lines)
- wave 1: `src/steamguide.js` (83 lines)
- wave 2: `main.js` (634 lines)

### Decisions for the user

**D1** (shape): The app is a library, entered through sofa (.). Port every entry point, in that shape?

> Yes: the Rust crate's public API becomes the library's front door

**D2** (desktop shell): This is an Electron app. Its main process is Node code that can move to Rust; its windows are web pages. Port the main-process logic and keep the web UI?

> Yes: port the main process piece by piece and keep the renderer as it is. Whether the UI later moves to Tauri (the same web UI on a Rust backend) is a separate choice, made once the logic is Rust

**D3** (dependency electron): Which crate replaces electron (used by 5 units)?

> Research it against primary sources into rust/research/electron.md before porting what uses it

**D4** (dependency @fontsource-variable/figtree): Which crate replaces @fontsource-variable/figtree (declared, no unit imports it)?

> Research it against primary sources into rust/research/@fontsource-variable-figtree.md before porting what uses it

