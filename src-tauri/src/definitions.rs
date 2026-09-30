//! Keyboard definitions (matrix size and physical layout) for keyboards whose
//! firmware does not carry one, i.e. VIA keyboards configured with VIA or Remap.
//!
//! Lookup order: a file the user imported, a cached download, then VIA's public
//! definition catalog (by USB vendor / product id).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

const CATALOG: &[&str] = &[
    "https://usevia.app/definitions/v3/{id}.json",
    "https://usevia.app/definitions/v2/{id}.json",
];

pub struct Store {
    imported: PathBuf,
    cache: PathBuf,
    /// Remembers which keyboards run Vial, so the HUD can leave them alone
    /// while the Vial app is open without having to ask them first.
    kinds: PathBuf,
}

fn file_name(vendor_id: u16, product_id: u16) -> String {
    format!("{vendor_id:04x}_{product_id:04x}.json")
}

/// A definition is usable if it tells us the matrix size and has a layout.
pub fn validate(def: &Value) -> Result<(), String> {
    let m = def.get("matrix").ok_or("no \"matrix\"")?;
    let rows = m.get("rows").and_then(Value::as_u64).ok_or("no matrix rows")?;
    let cols = m.get("cols").and_then(Value::as_u64).ok_or("no matrix cols")?;
    if rows == 0 || cols == 0 || cols > 32 {
        return Err(format!("unsupported matrix {rows}x{cols}"));
    }
    let layouts = def.get("layouts").ok_or("no \"layouts\"")?;
    if layouts.get("keymap").is_none() && layouts.get("keys").is_none() {
        return Err("no layout".into());
    }
    Ok(())
}

impl Store {
    pub fn new(config_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            imported: config_dir.join("definitions"),
            cache: cache_dir.join("definitions"),
            kinds: config_dir.join("keyboards.json"),
        }
    }

    fn read_kinds(&self) -> serde_json::Map<String, Value> {
        std::fs::read_to_string(&self.kinds)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    /// `Some(true)` = Vial firmware, `Some(false)` = plain VIA, `None` = not seen yet.
    pub fn is_vial(&self, vendor_id: u16, product_id: u16) -> Option<bool> {
        if let Some(v) = self.read_kinds().get(&format!("{vendor_id:04x}:{product_id:04x}")).and_then(Value::as_bool) {
            return Some(v);
        }
        // A keyboard we hold a VIA definition for is a plain VIA keyboard
        // (Vial keyboards carry their own definition).
        let name = file_name(vendor_id, product_id);
        (self.imported.join(&name).exists() || self.cache.join(&name).exists()).then_some(false)
    }

    pub fn remember_kind(&self, vendor_id: u16, product_id: u16, vial: bool) {
        if self.is_vial(vendor_id, product_id) == Some(vial) {
            return;
        }
        let mut kinds = self.read_kinds();
        kinds.insert(format!("{vendor_id:04x}:{product_id:04x}"), Value::Bool(vial));
        if let Some(dir) = self.kinds.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.kinds, Value::Object(kinds).to_string());
    }

    fn read(path: &Path) -> Option<Value> {
        let text = std::fs::read_to_string(path).ok()?;
        let def: Value = serde_json::from_str(&text).ok()?;
        validate(&def).ok()?;
        Some(def)
    }

    /// Definition for a keyboard, downloading it if needed.
    pub fn lookup(&self, vendor_id: u16, product_id: u16) -> Option<Value> {
        let name = file_name(vendor_id, product_id);
        if let Some(def) = Self::read(&self.imported.join(&name)) {
            return Some(def);
        }
        if let Some(def) = Self::read(&self.cache.join(&name)) {
            return Some(def);
        }
        let def = download(vendor_id, product_id)?;
        let _ = std::fs::create_dir_all(&self.cache);
        let _ = std::fs::write(self.cache.join(&name), def.to_string());
        Some(def)
    }

    /// Save a definition file the user dropped on the window. The vendor and
    /// product id are taken from the file when present, otherwise from the
    /// keyboard that is waiting for a definition.
    pub fn import(&self, path: &Path, fallback: Option<(u16, u16)>) -> Result<(u16, u16), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let def: Value = serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
        validate(&def)?;
        let ids = ids_in(&def).or(fallback).ok_or("the file has no vendorId / productId")?;
        std::fs::create_dir_all(&self.imported).map_err(|e| e.to_string())?;
        std::fs::write(self.imported.join(file_name(ids.0, ids.1)), def.to_string()).map_err(|e| e.to_string())?;
        Ok(ids)
    }
}

/// Vendor / product id written in a VIA definition ("0x1234" strings, or the
/// combined `vendorProductId` number of the v3 catalog).
fn ids_in(def: &Value) -> Option<(u16, u16)> {
    let parse = |v: &Value| -> Option<u16> {
        match v {
            Value::String(s) => u16::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok(),
            Value::Number(n) => n.as_u64().map(|n| n as u16),
            _ => None,
        }
    };
    if let (Some(v), Some(p)) = (def.get("vendorId").and_then(parse), def.get("productId").and_then(parse)) {
        return Some((v, p));
    }
    let combined = def.get("vendorProductId")?.as_u64()?;
    Some(((combined >> 16) as u16, (combined & 0xFFFF) as u16))
}

fn download(vendor_id: u16, product_id: u16) -> Option<Value> {
    let id = ((vendor_id as u32) << 16) | product_id as u32;
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(6)).build();
    for url in CATALOG {
        let url = url.replace("{id}", &id.to_string());
        let Ok(resp) = agent.get(&url).call() else { continue };
        let Ok(def) = resp.into_json::<Value>() else { continue };
        if validate(&def).is_ok() {
            return Some(def);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ids_from_definitions() {
        let def = serde_json::json!({ "vendorId": "0x303A", "productId": "0x4044" });
        assert_eq!(ids_in(&def), Some((0x303A, 0x4044)));
        let def = serde_json::json!({ "vendorProductId": 0x303A4044u32 });
        assert_eq!(ids_in(&def), Some((0x303A, 0x4044)));
    }

    /// Needs network access: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_from_catalog() {
        let id: u32 = 1077979750;
        let def = download((id >> 16) as u16, id as u16).expect("definition");
        assert!(validate(&def).is_ok());
    }
}
