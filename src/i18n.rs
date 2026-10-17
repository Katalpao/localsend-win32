//! External, file-based localization.
//!
//! UI strings are **not** compiled into the binary. Every language is a JSON
//! key→value map loaded at runtime from `<executable_dir>/lang/<code>.json`.
//! Switching the UI language is simply a matter of (re)loading the matching
//! file — see `Config::language` and the `WM_LANG_CHANGED` broadcast.
//!
//! Missing keys fall back to the raw key so the UI never breaks; production
//! packs are expected to be complete.

use std::collections::HashMap;
use std::path::PathBuf;

/// Directory that holds the language packs (`*.json`), resolved relative to the
/// running executable so packs can be edited or added without recompiling.
pub fn lang_dir() -> PathBuf {
    let mut dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.push("lang");
    dir
}

/// A loaded language pack.
#[derive(Clone)]
pub struct Lang {
    pub code: String,
    pub name: String,
    map: HashMap<String, String>,
}

impl Lang {
    /// Load a language pack by code (the JSON file name stem, e.g. `en`).
    ///
    /// Missing file or malformed JSON leaves the pack empty (every lookup then
    /// returns the key itself). The pack's own display name comes from the
    /// reserved `lang.name` key.
    pub fn load(code: &str) -> Lang {
        let path = lang_dir().join(format!("{}.json", code));
        let mut name = code.to_string();
        let mut map = HashMap::new();

        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<HashMap<String, String>>(&text) {
                Ok(v) => {
                    if let Some(n) = v.get("lang.name") {
                        name = n.clone();
                    }
                    map = v;
                }
                Err(e) => log::warn!("Language pack {} is not a valid map: {}", path.display(), e),
            },
            Err(e) => log::warn!("Language pack {} not found: {}", path.display(), e),
        }

        Lang {
            code: code.to_string(),
            name,
            map,
        }
    }

    /// Translate a key, falling back to the key itself when absent.
    pub fn tr(&self, key: &str) -> String {
        self.map.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    /// Translate with simple `{name}` placeholder substitution.
    pub fn trf(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.tr(key);
        for (k, v) in args {
            s = s.replace(&format!("{{{}}}", k), v);
        }
        s
    }

    /// List available language packs as `(code, display_name)` sorted by code.
    pub fn available() -> Vec<(String, String)> {
        let dir = lang_dir();
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let code = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if code.is_empty() || code == "README" {
                    continue;
                }
                let name = peek_name(&p).unwrap_or_else(|| code.clone());
                out.push((code, name));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

/// Read just the `lang.name` of a pack file.
fn peek_name(path: &PathBuf) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("lang.name").and_then(|n| n.as_str()).map(|s| s.to_string())
}

/// Pick a sensible default language from the user's Windows UI language so a
/// first run feels native. English-family locales → `en`, everything else →
/// `zh-CN` (Simplified Chinese).
pub fn default_language() -> String {
    unsafe {
        let lang = windows::Win32::Globalization::GetUserDefaultUILanguage();
        // Primary language id lives in the low 10 bits.
        let primary = lang & 0x3FF;
        if primary == 0x09 {
            "en".to_string()
        } else {
            "zh-CN".to_string()
        }
    }
}
