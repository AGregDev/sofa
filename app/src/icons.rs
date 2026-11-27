//! Icons and brand marks, read from the reference UI's own files at build time
//! (`assets/icons.js`: Lucide shapes, `assets/brand.js`: Simple Icons marks), kept as Sofa 1.x
//! draw the same shapes.

use std::collections::HashMap;
use std::sync::LazyLock;

const ICONS_JS: &str = include_str!("../assets/icons.js");
const BRAND_JS: &str = include_str!("../assets/brand.js");

/// `name: '<svg markup or path data>',` lines of a JS object literal.
fn entries(js: &'static str) -> HashMap<&'static str, &'static str> {
    let mut out = HashMap::new();
    for line in js.lines() {
        let t = line.trim();
        let Some((name, rest)) = t.split_once(": '") else { continue };
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let Some(value) = rest.strip_suffix("',").or_else(|| rest.strip_suffix('\'')) else { continue };
        out.insert(name, value);
    }
    out
}

static ICONS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| entries(ICONS_JS));
static BRANDS: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| entries(BRAND_JS));

/// A Lucide icon's SVG elements (stroked).
pub fn icon(name: &str) -> Option<(&'static str, &'static str)> {
    ICONS.get_key_value(name).map(|(k, v)| (*k, *v))
}

/// A brand mark's path data (filled).
pub fn brand(name: &str) -> Option<(&'static str, &'static str)> {
    BRANDS.get_key_value(name).map(|(k, v)| (*k, *v))
}
