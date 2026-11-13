//! The license texts that come with the app (THIRD-PARTY-NOTICES.txt, made by
//! tools/third-party-notices.mjs and installed next to the program). The tray
//! menu opens it in the text editor.

use tauri::{AppHandle, Manager};

pub const FILE: &str = "THIRD-PARTY-NOTICES.txt";

pub fn open(app: &AppHandle) {
    let path = match app.path().resolve(FILE, tauri::path::BaseDirectory::Resource) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("notices: {e}");
            return;
        }
    };
    #[cfg(windows)]
    unsafe {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let result = ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        // Values of 32 or less are errors.
        if result.0 as usize <= 32 {
            eprintln!("notices: could not open {} ({})", path.display(), result.0 as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FILE;

    fn lock_packages() -> Vec<String> {
        let lock = include_str!("../Cargo.lock");
        lock.split("[[package]]")
            .skip(1)
            .filter_map(|block| {
                let field = |key: &str| {
                    block.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim().trim_matches('"').to_string()))
                };
                Some((field("name = ")?, field("version = ")?))
            })
            .filter(|(name, _)| name != "cocokey")
            .map(|(name, version)| format!("{name} {version}"))
            .collect()
    }

    #[test]
    fn every_package_in_cargo_lock_is_in_the_notices() {
        // Fails after a dependency change until `node tools/third-party-notices.mjs` is run again.
        let notices = include_str!("../../THIRD-PARTY-NOTICES.txt");
        let lines: std::collections::HashSet<&str> = notices.lines().map(str::trim).collect();
        let missing: Vec<String> = lock_packages().into_iter().filter(|p| !lines.contains(p.as_str())).collect();
        assert!(missing.is_empty(), "not in {FILE} (run node tools/third-party-notices.mjs): {missing:?}");
    }

    #[test]
    fn the_notices_are_installed_and_carry_the_licenses() {
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let target = &conf["bundle"]["resources"][format!("../{FILE}")];
        assert_eq!(target, FILE, "{FILE} is not installed next to the program");
        let notices = include_str!("../../THIRD-PARTY-NOTICES.txt");
        assert!(notices.contains("GNU GENERAL PUBLIC LICENSE"), "CoCoKey's own license is missing");
        assert!(notices.contains("@tauri-apps/api"), "the window's JavaScript is missing");
        for name in ["tauri 2.", "hidapi ", "windows 0.62"] {
            assert!(notices.lines().any(|l| l.starts_with(name)), "{name} is missing");
        }
    }

    #[test]
    fn the_tray_menu_opens_the_notices() {
        let lib: String = include_str!("lib.rs").split_whitespace().collect();
        assert!(lib.contains("\"notices\"=>notices::open(app),"), "the tray item does not open the notices");
    }
}
