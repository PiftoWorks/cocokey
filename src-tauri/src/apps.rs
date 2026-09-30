//! Detects other programs that use the keyboard's raw HID interface.
//!
//! Replies on that interface go to every program that has it open, so the
//! HUD's polling would confuse a configurator while it is running. The worker
//! leaves the keyboards such a program can edit alone while it is open.

/// Configurator apps that are running right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Running {
    /// Vial desktop app: edits Vial keyboards only.
    pub vial: bool,
    /// VIA desktop app: edits every VIA-compatible keyboard.
    pub via: bool,
}

impl Running {
    /// Should the HUD keep away from a keyboard of this kind?
    pub fn blocks(&self, is_vial: bool) -> bool {
        self.via || (self.vial && is_vial)
    }
}

pub fn configurators() -> Running {
    let names = process_names();
    Running { vial: names.iter().any(|n| n == "vial.exe"), via: names.iter().any(|n| n == "via.exe") }
}

#[cfg(windows)]
fn process_names() -> Vec<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };

    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return vec![];
        };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut names = vec![];
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            names.push(String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase());
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        names
    }
}

#[cfg(not(windows))]
fn process_names() -> Vec<String> {
    vec![]
}
