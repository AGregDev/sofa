//! Valve KeyValues (.vdf / .acf) parsing, as Sofa reads Steam's files.
//!
//! Ported from Sofa 1.x's `src/vdf.js` with rust-anything, which recorded 172 calls of the
//! original (generated inputs and Steam-shaped files) and checked every one against this code;
//! `tests/replay.rs` keeps replaying them now that the original is gone. `port` keeps the
//! original's JS semantics; `js` carries JS values (undefined, odd numbers, lone surrogates)
//! in JSON the way the recording does.

pub mod js;
pub mod port;
pub mod utf16;

pub use port::{get, parse_vdf};

/// Parses a file's text.
pub fn parse(text: &str) -> serde_json::Value {
    // A string never reaches the parser's error paths (they need null, undefined or a value
    // without `slice`), so the fallback is unreachable in practice.
    parse_vdf(serde_json::Value::String(text.to_owned())).unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
}

/// `get(o, key)` for Rust callers: a case-insensitive lookup that answers None for JS undefined.
pub fn field(o: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    let v = get(o.clone(), serde_json::Value::String(key.into())).ok()?;
    (!js::is_undefined(&v) && !v.is_null()).then_some(v)
}
