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
    use std::path::Path;

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
    fn each_built_crates_license_text_starts_in_the_notices() {
        let output = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--offline", "--filter-platform", "x86_64-pc-windows-msvc"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo metadata must run to check local license files");
        assert!(output.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let notices = include_str!("../../THIRD-PARTY-NOTICES.txt");
        let headings: std::collections::HashSet<&str> = notices.lines().collect();
        let normalized_notices = notices.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut checked = 0;
        for package in metadata["packages"].as_array().unwrap() {
            let title = format!("{} {}", package["name"].as_str().unwrap(), package["version"].as_str().unwrap());
            if !headings.contains(title.as_str()) {
                continue; // Includes crates listed in Cargo.lock but not built for Windows.
            }
            let dir = Path::new(package["manifest_path"].as_str().unwrap()).parent().unwrap();
            let mut files = Vec::new();
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_file() && is_license_file(&entry.file_name().to_string_lossy()) {
                    files.push(entry.path());
                } else if entry.file_type().unwrap().is_dir() && entry.file_name().to_string_lossy().eq_ignore_ascii_case("LICENSES") {
                    for nested in std::fs::read_dir(entry.path()).unwrap() {
                        let nested = nested.unwrap();
                        if nested.file_type().unwrap().is_file() && ["txt", "md"].contains(&nested.path().extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str()) {
                            files.push(nested.path());
                        }
                    }
                }
            }
            for file in files {
                let text = std::fs::read_to_string(&file).unwrap();
                let beginning = text.split_whitespace().take(24).collect::<Vec<_>>().join(" ");
                assert!(!beginning.is_empty(), "empty license file: {}", file.display());
                assert!(normalized_notices.contains(&beginning), "{title}: the beginning of {} is missing from {FILE}", file.display());
                checked += 1;
            }
        }
        assert!(checked > 250, "only {checked} local crate license texts were checked");
    }

    fn is_license_file(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        ["license", "licence", "copying", "notice", "copyright", "unlicense"]
            .iter()
            .any(|prefix| lower == *prefix || ["-", ".", "_"].iter().any(|separator| lower.starts_with(&format!("{prefix}{separator}"))))
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
