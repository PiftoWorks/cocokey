//! "Start with Windows": one value in the current user's Run key.
//!
//! The value is named after the product (`productName` in tauri.conf.json):
//! the uninstaller made by `tauri build` deletes the Run value of that name, so
//! uninstalling does not leave CoCoKey starting at sign-in.

use std::path::Path;

/// Where the switch lives. The app uses [`SYSTEM`]; the tests use a scratch key.
pub struct Keys<'a> {
    /// Under HKEY_CURRENT_USER.
    pub run: &'a str,
    /// Task Manager and Settings > Startup apps keep their own on/off record here.
    pub approved: &'a str,
    pub name: &'a str,
}

pub const SYSTEM: Keys<'static> = Keys {
    run: r"Software\Microsoft\Windows\CurrentVersion\Run",
    approved: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
    // Must stay the same as productName (see the module comment).
    name: "CoCoKey",
};

/// The command Windows runs at sign-in. Quoted so a folder with spaces works.
pub fn command_for(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Task Manager writes a 12-byte record; an odd first byte means it was
/// switched off there. No record means on.
pub fn approved_allows(record: Option<&[u8]>) -> bool {
    match record.and_then(|r| r.first()) {
        Some(b) => b & 1 == 0,
        None => true,
    }
}

/// On when the Run value is there and Task Manager has not switched it off.
pub fn is_on(run_value: Option<&str>, approved: Option<&[u8]>) -> bool {
    run_value.is_some_and(|v| !v.trim().is_empty()) && approved_allows(approved)
}

/// Values under HKEY_CURRENT_USER. Reads tell "not there" (`Ok(None)`) apart
/// from "could not be read" (`Err`, a Windows error code), so a failed read is
/// never shown as "off".
pub trait Store {
    fn read_string(&self, key: &str, name: &str) -> Result<Option<String>, u32>;
    fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>, u32>;
    fn write_string(&self, key: &str, name: &str, value: &str) -> Result<(), u32>;
    /// Deleting a value that is not there counts as done.
    fn delete(&self, key: &str, name: &str) -> Result<(), u32>;
}

/// The real registry.
pub struct Registry;

#[cfg(windows)]
mod reg {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, REG_VALUE_TYPE,
    };

    fn missing(err: WIN32_ERROR) -> bool {
        err == ERROR_FILE_NOT_FOUND || err == ERROR_PATH_NOT_FOUND
    }

    pub fn read(key: &str, name: &str, flags: REG_ROUTINE_FLAGS) -> Result<Option<Vec<u8>>, u32> {
        let (key, name) = (HSTRING::from(key), HSTRING::from(name));
        let mut size = 0u32;
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, None, Some(&mut size)) };
        if missing(err) {
            return Ok(None);
        }
        if err != ERROR_SUCCESS {
            return Err(err.0);
        }
        let mut buf = vec![0u8; size as usize];
        let err = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
        };
        if missing(err) {
            return Ok(None);
        }
        if err != ERROR_SUCCESS {
            return Err(err.0);
        }
        buf.truncate(size as usize);
        Ok(Some(buf))
    }

    pub fn write(key: &str, name: &str, kind: REG_VALUE_TYPE, data: &[u8]) -> Result<(), u32> {
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(name),
                kind.0,
                Some(data.as_ptr().cast()),
                data.len() as u32,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(err.0)
        }
    }

    pub fn delete(key: &str, name: &str) -> Result<(), u32> {
        let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) };
        if err == ERROR_SUCCESS || missing(err) {
            Ok(())
        } else {
            Err(err.0)
        }
    }
}

#[cfg(windows)]
impl Store for Registry {
    fn read_string(&self, key: &str, name: &str) -> Result<Option<String>, u32> {
        use windows::Win32::System::Registry::RRF_RT_REG_SZ;
        Ok(reg::read(key, name, RRF_RT_REG_SZ)?.map(|bytes| {
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
            String::from_utf16_lossy(&wide[..end])
        }))
    }

    fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>, u32> {
        use windows::Win32::System::Registry::RRF_RT_REG_BINARY;
        reg::read(key, name, RRF_RT_REG_BINARY)
    }

    fn write_string(&self, key: &str, name: &str, value: &str) -> Result<(), u32> {
        use windows::Win32::System::Registry::REG_SZ;
        let bytes: Vec<u8> = value.encode_utf16().chain(std::iter::once(0)).flat_map(|c| c.to_le_bytes()).collect();
        reg::write(key, name, REG_SZ, &bytes)
    }

    fn delete(&self, key: &str, name: &str) -> Result<(), u32> {
        reg::delete(key, name)
    }
}

#[cfg(not(windows))]
impl Store for Registry {
    fn read_string(&self, _: &str, _: &str) -> Result<Option<String>, u32> {
        Err(1)
    }
    fn read_binary(&self, _: &str, _: &str) -> Result<Option<Vec<u8>>, u32> {
        Err(1)
    }
    fn write_string(&self, _: &str, _: &str, _: &str) -> Result<(), u32> {
        Err(1)
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), u32> {
        Err(1)
    }
}

/// Whether CoCoKey starts at sign-in now. `Err` when the registry could not
/// be read: then the switch is left alone rather than shown as off.
pub fn enabled(store: &impl Store, keys: &Keys) -> Result<bool, String> {
    let run = store.read_string(keys.run, keys.name).map_err(|e| format!("read Run: error {e}"))?;
    let approved = store.read_binary(keys.approved, keys.name).map_err(|e| format!("read StartupApproved: error {e}"))?;
    Ok(is_on(run.as_deref(), approved.as_deref()))
}

/// Turn starting at sign-in on or off. Turning it on also clears an "off" left
/// in Task Manager, so the switch means what it shows.
///
/// This takes two steps, and the first can succeed while the second fails, so
/// whatever happened the result is the state read back from the registry
/// (`Err` only when it cannot be read back).
pub fn set(store: &impl Store, keys: &Keys, on: bool, exe: &Path) -> Result<bool, String> {
    let wrote = if on {
        store.write_string(keys.run, keys.name, &command_for(exe)).and_then(|_| store.delete(keys.approved, keys.name))
    } else {
        store.delete(keys.run, keys.name).and_then(|_| store.delete(keys.approved, keys.name))
    };
    if let Err(e) = wrote {
        eprintln!("start with Windows: registry error {e}");
    }
    enabled(store, keys)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    const KEYS: Keys<'static> = Keys { run: "Run", approved: "Approved", name: "CoCoKey" };
    const EXE: &str = r"C:\Program Files\CoCoKey\cocokey.exe";

    #[derive(Clone, Debug, PartialEq)]
    enum Value {
        Text(String),
        Bytes(Vec<u8>),
    }

    /// A registry in memory; each operation can be told to fail.
    #[derive(Default)]
    struct FakeStore {
        values: RefCell<HashMap<(String, String), Value>>,
        fail_read: RefCell<Vec<&'static str>>,
        fail_write: RefCell<Vec<&'static str>>,
        fail_delete: RefCell<Vec<&'static str>>,
    }

    impl FakeStore {
        fn get(&self, key: &str) -> Option<Value> {
            self.values.borrow().get(&(key.to_string(), "CoCoKey".to_string())).cloned()
        }
        fn put(&self, key: &str, v: Value) {
            self.values.borrow_mut().insert((key.to_string(), "CoCoKey".to_string()), v);
        }
    }

    impl Store for FakeStore {
        fn read_string(&self, key: &str, name: &str) -> Result<Option<String>, u32> {
            if self.fail_read.borrow().contains(&key) {
                return Err(5);
            }
            match self.values.borrow().get(&(key.into(), name.into())) {
                Some(Value::Text(s)) => Ok(Some(s.clone())),
                Some(Value::Bytes(_)) => Err(1630), // wrong type
                None => Ok(None),
            }
        }
        fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>, u32> {
            if self.fail_read.borrow().contains(&key) {
                return Err(5);
            }
            match self.values.borrow().get(&(key.into(), name.into())) {
                Some(Value::Bytes(b)) => Ok(Some(b.clone())),
                Some(Value::Text(_)) => Err(1630),
                None => Ok(None),
            }
        }
        fn write_string(&self, key: &str, name: &str, value: &str) -> Result<(), u32> {
            if self.fail_write.borrow().contains(&key) {
                return Err(5);
            }
            self.values.borrow_mut().insert((key.into(), name.into()), Value::Text(value.into()));
            Ok(())
        }
        fn delete(&self, key: &str, name: &str) -> Result<(), u32> {
            if self.fail_delete.borrow().contains(&key) {
                return Err(5);
            }
            self.values.borrow_mut().remove(&(key.into(), name.into()));
            Ok(())
        }
    }

    #[test]
    fn the_command_is_quoted_for_folders_with_spaces() {
        assert_eq!(command_for(Path::new(EXE)), "\"C:\\Program Files\\CoCoKey\\cocokey.exe\"");
    }

    #[test]
    fn task_manager_can_switch_it_off() {
        assert!(approved_allows(None));
        assert!(approved_allows(Some(&[0x02, 0, 0, 0])));
        assert!(approved_allows(Some(&[0x06, 0, 0, 0])));
        assert!(!approved_allows(Some(&[0x03, 0, 0, 0])));
        assert!(!approved_allows(Some(&[0x07])));
        assert!(approved_allows(Some(&[])));
    }

    #[test]
    fn on_needs_the_run_value() {
        assert!(!is_on(None, None));
        assert!(!is_on(Some(""), None));
        assert!(is_on(Some("\"C:\\a\\cocokey.exe\""), None));
        assert!(!is_on(Some("\"C:\\a\\cocokey.exe\""), Some(&[0x03])));
    }

    #[test]
    fn the_value_name_is_the_product_name_the_uninstaller_removes() {
        let conf = include_str!("../tauri.conf.json");
        assert!(conf.contains(&format!("\"productName\": \"{}\"", SYSTEM.name)), "productName and the Run value name differ");
    }

    #[test]
    fn a_value_that_cannot_be_read_is_not_shown_as_off() {
        let store = FakeStore::default();
        assert_eq!(enabled(&store, &KEYS), Ok(false), "nothing there is off");
        store.fail_read.borrow_mut().push("Run");
        assert!(enabled(&store, &KEYS).is_err(), "a failed read of Run must not read as off");
        let store = FakeStore::default();
        store.put("Run", Value::Text(command_for(Path::new(EXE))));
        store.fail_read.borrow_mut().push("Approved");
        assert!(enabled(&store, &KEYS).is_err(), "a failed read of the Task Manager record must not read as on or off");
    }

    #[test]
    fn switching_on_reports_the_state_actually_left_when_the_second_step_fails() {
        // Run is written, but the Task Manager "off" record cannot be removed:
        // CoCoKey still does not start, so the switch must show off.
        let store = FakeStore::default();
        store.put("Approved", Value::Bytes(vec![0x03, 0, 0, 0]));
        store.fail_delete.borrow_mut().push("Approved");
        assert_eq!(set(&store, &KEYS, true, Path::new(EXE)), Ok(false));
        assert!(store.get("Run").is_some(), "Run was written");
        // Without the Task Manager record the same failure leaves it on.
        let store = FakeStore::default();
        store.fail_delete.borrow_mut().push("Approved");
        assert_eq!(set(&store, &KEYS, true, Path::new(EXE)), Ok(true));
    }

    #[test]
    fn a_failed_write_reports_the_unchanged_state() {
        let store = FakeStore::default();
        store.fail_write.borrow_mut().push("Run");
        assert_eq!(set(&store, &KEYS, true, Path::new(EXE)), Ok(false));
        let store = FakeStore::default();
        store.put("Run", Value::Text(command_for(Path::new(EXE))));
        store.fail_delete.borrow_mut().push("Run");
        assert_eq!(set(&store, &KEYS, false, Path::new(EXE)), Ok(true), "still on: the switch must say so");
    }

    #[test]
    fn a_state_that_cannot_be_read_back_is_an_error() {
        let store = FakeStore::default();
        store.fail_read.borrow_mut().push("Run");
        assert!(set(&store, &KEYS, true, Path::new(EXE)).is_err());
    }

    /// Writes only under a scratch key of the current user and removes it again.
    #[cfg(windows)]
    #[test]
    fn switching_on_and_off_round_trips_in_the_registry() {
        use windows::Win32::System::Registry::REG_BINARY;
        let base = format!(r"Software\com.piftoworks.cocokey.test\{}", std::process::id());
        let run = format!(r"{base}\Run");
        let approved = format!(r"{base}\Approved");
        let keys = Keys { run: &run, approved: &approved, name: "CoCoKey" };
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                use windows::core::HSTRING;
                use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};
                unsafe {
                    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(self.0.as_str()));
                    let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(r"Software\com.piftoworks.cocokey.test"));
                }
            }
        }
        let _cleanup = Cleanup(base.clone());
        let exe = Path::new(EXE);
        let store = Registry;

        // Nothing there (not even the keys) reads as off, not as an error.
        assert_eq!(enabled(&store, &keys), Ok(false));
        // An "off" from Task Manager is cleared when the switch is turned on.
        reg::write(&approved, "CoCoKey", REG_BINARY, &[0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        store.write_string(&run, "CoCoKey", &command_for(exe)).unwrap();
        assert_eq!(enabled(&store, &keys), Ok(false), "Task Manager's off counts");
        assert_eq!(set(&store, &keys, true, exe), Ok(true));
        assert_eq!(store.read_string(&run, "CoCoKey"), Ok(Some(command_for(exe))));
        assert_eq!(store.read_binary(&approved, "CoCoKey"), Ok(None));
        // Turning it on twice is the same as once.
        assert_eq!(set(&store, &keys, true, exe), Ok(true));
        assert_eq!(set(&store, &keys, false, exe), Ok(false));
        assert_eq!(store.read_string(&run, "CoCoKey"), Ok(None));
        // Off when already off is fine.
        assert_eq!(set(&store, &keys, false, exe), Ok(false));
        // A value of the wrong type is "cannot be read", not "off".
        reg::write(&run, "CoCoKey", REG_BINARY, &[1, 2]).unwrap();
        assert!(enabled(&store, &keys).is_err());
    }
}
