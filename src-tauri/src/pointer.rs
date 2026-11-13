//! Raw input from all keyboards and mice.
//!
//! Two things are taken from it:
//! - movement of the pointing device built into the tracked keyboard
//!   (keyboards such as the Svalboard switch to a mouse layer when it moves,
//!   which never shows up in the switch matrix);
//! - which keyboard was typed on last, plus the keys currently held on other
//!   keyboards, so the HUD can show a standard layout for them.
//!
//! Only the set of keys that are down right now is kept; nothing is recorded.

use std::sync::{Arc, Mutex};

/// USB vendor / product id of the tracked keyboard and whether its pointer
/// movements should be reported.
#[derive(Debug, Clone, Copy)]
pub struct TargetInfo {
    pub vendor_id: u16,
    pub product_id: u16,
    pub pointer: bool,
}

pub type Target = Arc<Mutex<Option<TargetInfo>>>;

#[derive(Debug, Clone)]
pub enum RawEvent {
    /// The tracked keyboard's pointing device moved by (x, y) counts and
    /// scrolled by this many wheel steps (both wheels, absolute) since the
    /// previous event.
    PointerMoved(i32, i32, i32),
    /// The keyboard typed on last changed (`true` = the tracked keyboard).
    TypingOn(bool),
    /// A key or mouse button of the tracked keyboard went down (no key codes
    /// are passed on; it only tells the worker to read the matrix now).
    TrackedKey,
    /// Scan codes held on other keyboards (0xE0 prefix folded into the high byte).
    OtherKeys(Vec<u16>),
    /// Esc was pressed on any keyboard (closes the HUD's temporary modes even
    /// when the HUD window does not have keyboard focus).
    Escape,
}

#[cfg(windows)]
pub fn spawn(target: Target, sink: Box<dyn Fn(RawEvent) + Send>) {
    std::thread::Builder::new()
        .name("raw-input".into())
        .spawn(move || {
            if let Err(e) = imp::run(target, sink) {
                eprintln!("raw input disabled: {e}");
            }
        })
        .ok();
}

#[cfg(not(windows))]
pub fn spawn(_target: Target, _sink: Box<dyn Fn(RawEvent) + Send>) {}

#[cfg(windows)]
mod imp {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use windows::core::w;
    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::{
        GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
        RAWINPUTHEADER, RIDEV_INPUTSINK, RIDI_DEVICENAME, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, HWND_MESSAGE,
        MSG, RI_KEY_BREAK, RI_KEY_E0, RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT,
        WNDCLASSW,
    };

    use super::{RawEvent, Target};

    struct Ctx {
        target: Target,
        sink: Box<dyn Fn(RawEvent) + Send>,
        names: HashMap<isize, String>,
        last_move: Option<Instant>,
        /// Movement not reported yet (events are sent at most every few ms):
        /// x, y and wheel steps.
        pending_move: (i32, i32, i32),
        typing_on_target: Option<bool>,
        held: BTreeSet<u16>,
    }

    static CTX: OnceLock<Mutex<Ctx>> = OnceLock::new();

    pub fn run(target: Target, sink: Box<dyn Fn(RawEvent) + Send>) -> windows::core::Result<()> {
        let _ = CTX.set(Mutex::new(Ctx {
            target,
            sink,
            names: HashMap::new(),
            last_move: None,
            pending_move: (0, 0, 0),
            typing_on_target: None,
            held: BTreeSet::new(),
        }));
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: hinstance.into(),
                lpszClassName: w!("CoCoKeyRawInput"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("CoCoKeyRawInput"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )?;
            let devices = [
                RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x02, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd },
                RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x06, dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd },
            ];
            RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32)?;
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        Ok(())
    }

    unsafe fn device_name(handle: HANDLE) -> String {
        let mut len = 0u32;
        GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, None, &mut len);
        if len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize];
        let got = GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, Some(buf.as_mut_ptr() as *mut _), &mut len);
        if got == u32::MAX {
            return String::new();
        }
        String::from_utf16_lossy(&buf).trim_end_matches('\0').to_uppercase()
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_INPUT {
            handle_input(lparam);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    impl Ctx {
        /// Is this raw input device part of the tracked keyboard?
        unsafe fn is_target(&mut self, device: HANDLE) -> Option<bool> {
            let target = (*self.target.lock().unwrap())?;
            let key = device.0 as isize;
            let name = self.names.entry(key).or_insert_with(|| device_name(device));
            let wanted = format!("VID_{:04X}&PID_{:04X}", target.vendor_id, target.product_id);
            Some(name.contains(&wanted))
        }
    }

    unsafe fn handle_input(lparam: LPARAM) {
        let header = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        let hraw = HRAWINPUT(lparam.0 as *mut _);
        let mut size = 0u32;
        GetRawInputData(hraw, RID_INPUT, None, &mut size, header);
        if size == 0 {
            return;
        }
        let mut buf = vec![0u8; size as usize];
        if GetRawInputData(hraw, RID_INPUT, Some(buf.as_mut_ptr() as *mut _), &mut size, header) != size {
            return;
        }
        let raw = &*(buf.as_ptr() as *const RAWINPUT);
        let Some(ctx) = CTX.get() else { return };
        let Ok(mut ctx) = ctx.lock() else { return };

        if raw.header.dwType == RIM_TYPEMOUSE.0 {
            let mouse = raw.data.mouse;
            let flags = mouse.Anonymous.Anonymous.usButtonFlags as u32;
            let moved = mouse.lLastX != 0 || mouse.lLastY != 0 || flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0;
            // Left, right, middle, 4th and 5th button down.
            const BUTTON_DOWN: u32 = 0x0001 | 0x0004 | 0x0010 | 0x0040 | 0x0100;
            if !moved && flags & BUTTON_DOWN == 0 {
                return;
            }
            if ctx.is_target(raw.header.hDevice) != Some(true) {
                return;
            }
            if flags & BUTTON_DOWN != 0 {
                (ctx.sink)(RawEvent::TrackedKey);
            }
            let pointer_enabled = ctx.target.lock().unwrap().is_some_and(|t| t.pointer);
            if !moved || !pointer_enabled {
                return;
            }
            // Relative counts, as the firmware sees them (auto mouse thresholds).
            ctx.pending_move.0 += mouse.lLastX;
            ctx.pending_move.1 += mouse.lLastY;
            if flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 {
                // 120 per detent; firmware wheel reports are one step each.
                let delta = (mouse.Anonymous.Anonymous.usButtonData as i16 as i32).abs();
                ctx.pending_move.2 += (delta / 120).max(1);
            }
            let now = Instant::now();
            if ctx.last_move.is_some_and(|t| now.duration_since(t) < Duration::from_millis(8)) {
                return;
            }
            ctx.last_move = Some(now);
            let (dx, dy, dw) = std::mem::take(&mut ctx.pending_move);
            (ctx.sink)(RawEvent::PointerMoved(dx, dy, dw));
            return;
        }

        if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
            let kb = raw.data.keyboard;
            // Windows sends fake shift events (VKey 0xFF) around some keys.
            if kb.VKey == 0xFF || kb.MakeCode == 0 {
                return;
            }
            let flags = kb.Flags as u32;
            if kb.MakeCode == 0x01 && flags & (RI_KEY_BREAK | RI_KEY_E0) == 0 {
                (ctx.sink)(RawEvent::Escape);
            }
            let on_target = ctx.is_target(raw.header.hDevice).unwrap_or(false);
            if ctx.typing_on_target != Some(on_target) {
                ctx.typing_on_target = Some(on_target);
                (ctx.sink)(RawEvent::TypingOn(on_target));
            }
            if on_target {
                if flags & RI_KEY_BREAK == 0 {
                    (ctx.sink)(RawEvent::TrackedKey);
                }
                return;
            }
            // Pause arrives as an E1-prefixed sequence; give it its own code.
            let code = if kb.VKey == 0x13 {
                0xE11D
            } else {
                kb.MakeCode | if flags & RI_KEY_E0 != 0 { 0xE000 } else { 0 }
            };
            let changed = if flags & RI_KEY_BREAK != 0 { ctx.held.remove(&code) } else { ctx.held.insert(code) };
            if changed {
                let held: Vec<u16> = ctx.held.iter().copied().collect();
                (ctx.sink)(RawEvent::OtherKeys(held));
            }
        }
    }
}
