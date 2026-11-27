//! Port of `src/vdf.js` (Valve KeyValues). JS semantics kept so every recorded call replays: `text` is
//! indexed as JS does, objects keep JS key order and drop `__proto__`. Where JS would loop forever
//! (whitespace `skip` leaves but that ends a token, e.g. U+00A0) the port returns what it parsed.
#![forbid(unsafe_code)]

use crate::js;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// `String(v)` as UTF-16.
fn js_string(v: &Value) -> Vec<u16> {
    let text = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    match (js::string_units(v), js::as_number(v), v) {
        _ if js::is_undefined(v) => text("undefined"),
        (Some(units), ..) => units,
        (_, Some(n), _) => text(&js::number_to_string(n)),
        (.., Value::Null) => text("null"),
        (.., Value::Bool(b)) => text(&b.to_string()),
        (.., Value::Array(a)) => {
            let parts: Vec<_> = a.iter().map(|i| if i.is_null() || js::is_undefined(i) { vec![] } else { js_string(i) }).collect();
            parts.join(&(b',' as u16))
        }
        _ => text("[object Object]"),
    }
}

/// JS own-key order: array-index keys ("0".."4294967294") ascending, then insertion order; a
/// repeated key keeps its place; `o.__proto__ = v` never makes an own property.
fn js_object(entries: impl IntoIterator<Item = (String, Value)>) -> Map<String, Value> {
    let (mut indexed, mut named) = (BTreeMap::new(), Map::new());
    for (k, v) in entries.into_iter().filter(|(k, _)| k != "__proto__") {
        let canonical = k.len() <= 10 && !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) && (k == "0" || !k.starts_with('0'));
        match k.parse::<u64>().ok().filter(|&n| canonical && n < u64::from(u32::MAX)) {
            Some(n) => indexed.insert(n, (k, v)).map(drop).unwrap_or_default(),
            None => named.insert(k, v).map(drop).unwrap_or_default(),
        }
    }
    indexed.into_values().chain(named).collect()
}

/// One `text[i]`: a code unit of a string, or an element / numbered property.
enum Item<'a> {
    Unit(u16),
    Value(&'a Value),
}

struct Parser<'a> {
    units: Option<Vec<u16>>,
    values: Option<&'a [Value]>,
    props: Option<&'a Map<String, Value>>,
    i: usize,
}

impl Parser<'_> {
    /// `length`: undefined (None) for anything but strings and arrays.
    fn len(&self) -> Option<usize> {
        self.units.as_ref().map(Vec::len).or(self.values.map(<[Value]>::len))
    }
    fn before_end(&self, i: usize) -> bool {
        self.len().is_some_and(|n| i < n)
    }
    fn item(&self, i: usize) -> Option<Item<'_>> {
        match (&self.units, self.values, self.props) {
            (Some(u), ..) => u.get(i).map(|&c| Item::Unit(c)),
            (_, Some(a), _) => a.get(i).map(Item::Value),
            (.., Some(o)) => o.get(&i.to_string()).map(Item::Value),
            _ => None,
        }
    }
    /// `text[i] === c` (strict: only a one-character string).
    fn is(&self, i: usize, c: u8) -> bool {
        match self.item(i) {
            Some(Item::Unit(u)) => u == u16::from(c),
            Some(Item::Value(v)) => js::string_units(v).is_some_and(|s| s == [u16::from(c)]),
            None => false,
        }
    }
    /// `String(text[i])`.
    fn string_at(&self, i: usize) -> Vec<u16> {
        match self.item(i) {
            Some(Item::Unit(u)) => vec![u],
            Some(Item::Value(v)) => js_string(v),
            None => "undefined".encode_utf16().collect(),
        }
    }
    fn skip(&mut self) {
        while self.before_end(self.i) {
            if b" \t\r\n".iter().any(|&c| self.is(self.i, c)) {
                self.i += 1;
            } else if self.is(self.i, b'/') && self.is(self.i + 1, b'/') {
                while self.before_end(self.i) && !self.is(self.i, b'\n') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }
    fn str(&mut self) -> Result<Value, String> {
        if !self.is(self.i, b'"') {
            let (start, space) = (
                self.i,
                |c: &u16| matches!(c, 9..=13 | 32 | 34 | 123 | 125 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff),
            );
            while self.before_end(self.i) && !self.string_at(self.i).iter().any(space) {
                self.i += 1;
            }
            let range = |n: usize| start.min(n)..self.i.min(n);
            return match (&self.units, self.values) {
                (Some(u), _) => Ok(js::utf16_string(&u[range(u.len())])),
                (_, Some(a)) => Ok(Value::Array(a[range(a.len())].to_vec())),
                _ => Err("text.slice is not a function".into()), // recorded-literal: V8 TypeError
            };
        }
        self.i += 1;
        let mut out = Vec::new();
        while self.before_end(self.i) && !self.is(self.i, b'"') {
            let escaped = self.is(self.i, b'\\') && self.before_end(self.i + 1);
            let at = self.i + usize::from(escaped);
            out.extend(match () {
                _ if escaped && self.is(at, b'n') => vec![u16::from(b'\n')],
                _ if escaped && self.is(at, b't') => vec![u16::from(b'\t')],
                _ => self.string_at(at),
            });
            self.i = at + 1;
        }
        self.i += 1;
        Ok(js::utf16_string(&out))
    }
    fn obj(&mut self) -> Result<Value, String> {
        let mut entries = Vec::new();
        loop {
            self.skip();
            if self.len().is_some_and(|n| self.i >= n) || self.is(self.i, b'}') {
                self.i += usize::from(self.is(self.i, b'}'));
                return Ok(Value::Object(js_object(entries)));
            }
            let before = self.i;
            let key = String::from_utf16_lossy(&js_string(&self.str()?));
            self.skip();
            let value = if self.is(self.i, b'{') {
                self.i += 1;
                self.obj()?
            } else {
                self.str()?
            };
            entries.push((key, value));
            if self.i == before {
                return Ok(Value::Object(js_object(entries))); // JS would loop forever here
            }
        }
    }
}

/// Port of `parseVdf` (`src/vdf.js:2`).
pub fn parse_vdf(text: serde_json::Value) -> Result<serde_json::Value, String> {
    if text.is_null() || js::is_undefined(&text) {
        let what = if text.is_null() { "null" } else { "undefined" };
        return Err(format!("Cannot read properties of {what} (reading 'length')")); // recorded-literal: V8 TypeError
    }
    let units = js::string_units(&text);
    let object = units.is_none() && js::as_number(&text).is_none();
    let values = text.as_array().filter(|_| object).map(Vec::as_slice);
    let props = text.as_object().filter(|_| object);
    Parser { units, values, props, i: 0 }.obj()
}

/// Port of `get` (`src/vdf.js:53`): case-insensitive property lookup.
pub fn get(o: serde_json::Value, key: serde_json::Value) -> Result<serde_json::Value, String> {
    let plain = js::string_units(&o).is_none() && js::as_number(&o).is_none() && !js::is_undefined(&o);
    let entries: Vec<(String, Value)> = match &o {
        Value::Array(a) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
        Value::Object(m) if plain => js_object(m.clone()).into_iter().collect(),
        _ => return Ok(js::undefined()), // falsy, or typeof o !== 'object'
    };
    let prop = String::from_utf16_lossy(&js_string(&key));
    if let (Value::Array(a), "length") = (&o, prop.as_str()) {
        return Ok(js::number(a.len() as f64));
    }
    if let Some((_, v)) = entries.iter().find(|(k, _)| *k == prop) {
        return Ok(v.clone());
    }
    let Some(units) = js::string_units(&key) else {
        return Err(match () {
            _ if key.is_null() => "Cannot read properties of null (reading 'toLowerCase')".into(), // recorded-literal: V8 TypeError
            _ if js::is_undefined(&key) => "Cannot read properties of undefined (reading 'toLowerCase')".into(), // recorded-literal: V8 TypeError
            _ => "key.toLowerCase is not a function".into(), // recorded-literal: V8 TypeError
        });
    };
    // Lone surrogates stay as they are (they never occur in Steam's files).
    let lower = |s: &[u16]| String::from_utf16(s).map_or_else(|_| s.to_vec(), |s| s.to_lowercase().encode_utf16().collect());
    let lk = lower(&units);
    let found = entries.into_iter().find(|(k, _)| lower(&k.encode_utf16().collect::<Vec<_>>()) == lk);
    Ok(found.map_or_else(js::undefined, |(_, v)| v))
}
