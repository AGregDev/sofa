//! `%APPDATA%\sofa\sofa.json`, shared with the Electron app (ADR-003): the same keys and
//! defaults as `src/store.js`. Unknown keys are kept as they are.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub struct Store {
    file: PathBuf,
    pub data: Map<String, Value>,
    dirty: bool,
}

fn defaults() -> Map<String, Value> {
    let v = json!({
        "setupDone": false,
        "settings": {
            "launchAtLogin": true,
            "fullscreen": true,
            "keepInFront": true,
            "homeButton": "ps",
            "controllerLayout": "auto",
            "steamGuide": true,
            "gameBarOff": true,
            "padAudio": true,
            "controllerLight": "follow",
            "browser": "auto",
            "sounds": true,
            "motion": "full",
            "showHidden": false
        },
        "overrides": {},
        "recent": {}
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

impl Store {
    pub fn open(dir: &Path) -> Store {
        let file = dir.join("sofa.json");
        let mut data = defaults();
        let text = std::fs::read_to_string(&file).ok();
        // Editors may add a byte order mark; Sofa writes plain UTF-8.
        let parsed = text.as_deref().and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok());
        if text.as_deref().is_some_and(|t| !t.trim().is_empty()) && parsed.is_none() {
            // Unreadable: keep a copy before Sofa writes its defaults over it.
            let _ = std::fs::copy(&file, dir.join("sofa.json.unreadable"));
        }
        if let Some(Value::Object(saved)) = parsed {
            for (k, v) in saved {
                if k == "settings" {
                    if let (Some(Value::Object(base)), Value::Object(s)) = (data.get_mut("settings"), v) {
                        base.extend(s);
                    }
                } else {
                    data.insert(k, v);
                }
            }
        }
        Store { file, data, dirty: false }
    }

    pub fn setting(&self, key: &str) -> Option<&Value> {
        self.data.get("settings")?.get(key)
    }
    pub fn flag(&self, key: &str) -> bool {
        self.setting(key).and_then(Value::as_bool).unwrap_or(false)
    }
    pub fn text(&self, key: &str) -> &str {
        self.setting(key).and_then(Value::as_str).unwrap_or("")
    }
    pub fn set(&mut self, key: &str, value: Value) {
        if let Some(Value::Object(s)) = self.data.get_mut("settings") {
            s.insert(key.into(), value);
            self.dirty = true;
        }
    }
    /// A top-level flag (`setupDone` lives beside `settings`, not in it).
    pub fn top_flag(&self, key: &str) -> bool {
        self.data.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
    pub fn set_top(&mut self, key: &str, value: Value) {
        self.data.insert(key.into(), value);
        self.dirty = true;
    }
    pub fn map(&self, key: &str) -> Map<String, Value> {
        self.data.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
    }
    pub fn map_mut(&mut self, key: &str) -> &mut Map<String, Value> {
        self.dirty = true;
        let entry = self.data.entry(key.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        match entry {
            Value::Object(m) => m,
            _ => unreachable!("INFALLIBLE: replaced with an object just above"),
        }
    }

    /// Writes the file if anything changed (atomically, through a temp file).
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("json.tmp");
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(self.data.clone()))
            && std::fs::write(&tmp, text).is_ok()
            && std::fs::rename(&tmp, &self.file).is_ok()
        {
            self.dirty = false;
        }
    }
}

/// `%APPDATA%\sofa`, the folder Electron's `app.getPath('userData')` uses for Sofa.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("sofa")
}
