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

#[cfg(windows)]
mod reg {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, REG_SZ,
        RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
    };

    fn read(key: &str, name: &str, flags: REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
        let (key, name) = (HSTRING::from(key), HSTRING::from(name));
        let mut size = 0u32;
        let err = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, None, Some(&mut size)) };
        if err != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let err = unsafe {
            RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, Some(buf.as_mut_ptr().cast()), Some(&mut size))
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        buf.truncate(size as usize);
        Some(buf)
    }

    pub fn read_string(key: &str, name: &str) -> Option<String> {
        let bytes = read(key, name, RRF_RT_REG_SZ)?;
        let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        Some(String::from_utf16_lossy(&wide[..end]))
    }

    pub fn read_binary(key: &str, name: &str) -> Option<Vec<u8>> {
        read(key, name, RRF_RT_REG_BINARY)
    }

    pub fn write_string(key: &str, name: &str, value: &str) -> Result<(), WIN32_ERROR> {
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let err = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(name),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(err)
        }
    }

    /// Deleting a value that is not there counts as done.
    pub fn delete(key: &str, name: &str) -> Result<(), WIN32_ERROR> {
        let err = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name)) };
        if err == ERROR_SUCCESS || err == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(err)
        }
    }
}

/// Whether CoCoKey starts at sign-in now.
pub fn enabled(keys: &Keys) -> bool {
    #[cfg(windows)]
    {
        is_on(reg::read_string(keys.run, keys.name).as_deref(), reg::read_binary(keys.approved, keys.name).as_deref())
    }
    #[cfg(not(windows))]
    {
        let _ = keys;
        false
    }
}

/// Turn starting at sign-in on or off and return the state read back.
/// Turning it on also clears an "off" left in Task Manager, so the switch means what it shows.
pub fn set(keys: &Keys, on: bool, exe: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let result = if on {
            reg::write_string(keys.run, keys.name, &command_for(exe)).and_then(|_| reg::delete(keys.approved, keys.name))
        } else {
            reg::delete(keys.run, keys.name).and_then(|_| reg::delete(keys.approved, keys.name))
        };
        result.map_err(|e| format!("registry error {}", e.0))?;
        Ok(enabled(keys))
    }
    #[cfg(not(windows))]
    {
        let _ = (keys, on, exe);
        Err("not supported".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_quoted_for_folders_with_spaces() {
        let exe = Path::new(r"C:\Program Files\CoCoKey\cocokey.exe");
        assert_eq!(command_for(exe), "\"C:\\Program Files\\CoCoKey\\cocokey.exe\"");
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

    /// Writes only under a scratch key of the current user and removes it again.
    #[cfg(windows)]
    #[test]
    fn switching_on_and_off_round_trips_in_the_registry() {
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
        let exe = Path::new(r"C:\Program Files\CoCoKey\cocokey.exe");

        assert!(!enabled(&keys));
        // An "off" from Task Manager is cleared when the switch is turned on.
        reg::write_string(&approved, "CoCoKey", "x").unwrap();
        assert_eq!(set(&keys, true, exe), Ok(true));
        assert_eq!(reg::read_string(&run, "CoCoKey").as_deref(), Some(command_for(exe).as_str()));
        assert_eq!(reg::read_string(&approved, "CoCoKey"), None);
        // Turning it on twice is the same as once.
        assert_eq!(set(&keys, true, exe), Ok(true));
        assert_eq!(set(&keys, false, exe), Ok(false));
        assert_eq!(reg::read_string(&run, "CoCoKey"), None);
        // Off when already off is fine.
        assert_eq!(set(&keys, false, exe), Ok(false));
    }
}
