mod apps;
mod definitions;
mod device;
mod engine;
mod keycode;
mod pointer;
mod protocol;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

use device::{Control, KeyboardInfo, LiveState, Shared};
use protocol::DeviceInfo;

struct AppState {
    shared: Arc<Mutex<Shared>>,
    control: Mutex<Sender<Control>>,
    click_through: AtomicBool,
    /// "Highlight pressed keys". The window saves the choice; this copy only
    /// keeps the tray's check mark right.
    highlight: AtomicBool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    keyboard: Option<KeyboardInfo>,
    state: LiveState,
    devices: Vec<DeviceInfo>,
    click_through: bool,
}

#[tauri::command]
fn snapshot(app: State<AppState>) -> Snapshot {
    let shared = app.shared.lock().unwrap();
    Snapshot {
        keyboard: shared.keyboard.clone(),
        state: shared.state.clone(),
        devices: shared.devices.clone(),
        click_through: app.click_through.load(Ordering::Relaxed),
    }
}

fn send(app: &AppHandle, msg: Control) {
    let state = app.state::<AppState>();
    let _ = state.control.lock().unwrap().send(msg);
}

#[tauri::command]
fn reload(app: AppHandle) {
    send(&app, Control::Reload);
}

#[tauri::command]
fn reset_layers(app: AppHandle) {
    send(&app, Control::Reset);
}

#[tauri::command]
fn set_paused(app: AppHandle, paused: bool) {
    send(&app, Control::SetPaused(paused));
}

#[tauri::command]
fn select_device(app: AppHandle, path: Option<String>) {
    send(&app, Control::Select(path));
}

#[tauri::command]
fn set_ignore_configurators(app: AppHandle, ignore: bool) {
    send(&app, Control::IgnoreConfigurators(ignore));
}

#[tauri::command]
fn set_auto_mouse(app: AppHandle, enabled: bool) {
    send(&app, Control::SetAutoMouse(enabled));
}

#[tauri::command]
fn import_definition(app: AppHandle, path: String) {
    send(&app, Control::Import(path.into()));
}

#[tauri::command]
fn set_click_through(app: AppHandle, enabled: bool) {
    apply_click_through(&app, enabled);
}

fn apply_click_through(app: &AppHandle, enabled: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_ignore_cursor_events(enabled);
    }
    app.state::<AppState>().click_through.store(enabled, Ordering::Relaxed);
    let _ = app.emit("hud://click-through", enabled);
}

/// Sent by the window at start-up and whenever the settings menu switches it.
#[tauri::command]
fn set_highlight(app: AppHandle, enabled: bool) {
    apply_highlight(&app, enabled);
}

/// One place for both the settings menu and the tray: remember the choice,
/// tell the window (which saves it and redraws) and the tray's check mark.
fn apply_highlight(app: &AppHandle, enabled: bool) {
    app.state::<AppState>().highlight.store(enabled, Ordering::Relaxed);
    let _ = app.emit("hud://highlight", enabled);
}

/// Native handle of a window, for the Win32 calls below.
#[cfg(windows)]
fn hwnd_of(w: &tauri::WebviewWindow) -> Option<windows::Win32::Foundation::HWND> {
    w.hwnd().ok().map(|h| windows::Win32::Foundation::HWND(h.0 as _))
}

/// Minimise to the taskbar. On this transparent, undecorated window Tauri's
/// `minimize()` can leave it without a taskbar button, so it looks as if it
/// went to the tray; give it a taskbar button first, then minimise.
#[tauri::command]
fn minimize_main(app: AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    #[cfg(windows)]
    if let Some(hwnd) = hwnd_of(&w) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_MINIMIZE, WS_EX_APPWINDOW,
            WS_EX_TOOLWINDOW,
        };
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let ex = (ex & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
        }
        return;
    }
    let _ = w.minimize();
}

/// Put the window away in the tray (the close button and Alt+F4). Tauri's
/// `hide()` alone can be ignored by a transparent window, so hide it natively too.
fn hide_to_tray(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = w.hide();
    #[cfg(windows)]
    if let Some(hwnd) = hwnd_of(&w) {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

#[tauri::command]
fn hide_main_to_tray(app: AppHandle) {
    hide_to_tray(&app);
}

/// Bring the window back from the tray or the taskbar.
fn show_main(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

fn japanese() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.starts_with("ja"))
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let ja = japanese();
    let t = |ja_text: &'static str, en_text: &'static str| if ja { ja_text } else { en_text };
    let show = MenuItem::with_id(app, "show", t("表示", "Show"), true, None::<&str>)?;
    let click = CheckMenuItem::with_id(app, "click", t("クリックを透過", "Click-through"), true, false, None::<&str>)?;
    let highlight = CheckMenuItem::with_id(
        app,
        "highlight",
        t("押したキーを強調表示", "Highlight pressed keys"),
        true,
        app.state::<AppState>().highlight.load(Ordering::Relaxed),
        None::<&str>,
    )?;
    let pause = CheckMenuItem::with_id(
        app,
        "pause",
        t("一時停止（Vial で編集するとき）", "Pause (while editing in Vial)"),
        true,
        false,
        None::<&str>,
    )?;
    let reload = MenuItem::with_id(app, "reload", t("キー配列を読み直す", "Reload keymap"), true, None::<&str>)?;
    let reset = MenuItem::with_id(app, "reset", t("レイヤーを初期状態に戻す", "Reset layer state"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t("終了", "Quit"), true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &click, &highlight, &sep, &pause, &reload, &reset, &sep2, &quit])?;

    let click_item = click.clone();
    let highlight_item = highlight.clone();
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("CoCoKey")
        .menu(&menu)
        // Left click brings the window back; the menu is on the right button.
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "click" => {
                let enabled = !app.state::<AppState>().click_through.load(Ordering::Relaxed);
                apply_click_through(app, enabled);
                let _ = click_item.set_checked(enabled);
            }
            "highlight" => {
                let enabled = !app.state::<AppState>().highlight.load(Ordering::Relaxed);
                apply_highlight(app, enabled);
                let _ = highlight_item.set_checked(enabled);
            }
            "pause" => {
                let checked = pause.is_checked().unwrap_or(false);
                send(app, Control::SetPaused(checked));
            }
            "reload" => send(app, Control::Reload),
            "reset" => send(app, Control::Reset),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;

    // Keep the tray check mark in sync when the window toggles click-through.
    let click_sync = click.clone();
    tauri::Listener::listen(app, "hud://click-through", move |e| {
        let _ = click_sync.set_checked(e.payload() == "true");
    });
    // ... and when the settings menu switches "highlight pressed keys".
    let highlight_sync = highlight.clone();
    tauri::Listener::listen(app, "hud://highlight", move |e| {
        let _ = highlight_sync.set_checked(e.payload() == "true");
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must come first: a second launch only wakes up the running copy.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| show_main(app)))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let shared = Arc::new(Mutex::new(Shared::default()));
            let target: pointer::Target = Arc::new(Mutex::new(None));
            let control = device::spawn(handle.clone(), shared.clone(), target.clone());
            let pointer_tx = control.clone();
            let emitter = handle.clone();
            pointer::spawn(
                target,
                Box::new(move |event| match event {
                    pointer::RawEvent::PointerMoved(dx, dy, dw) => {
                        let _ = pointer_tx.send(Control::PointerMoved(dx, dy, dw));
                    }
                    pointer::RawEvent::TypingOn(tracked) => {
                        let _ = emitter.emit("hud://typing", tracked);
                    }
                    pointer::RawEvent::OtherKeys(keys) => {
                        let _ = emitter.emit("hud://other-keys", keys);
                    }
                    pointer::RawEvent::Escape => {
                        let _ = emitter.emit("hud://escape", ());
                    }
                }),
            );
            app.manage(AppState {
                shared,
                control: Mutex::new(control),
                click_through: AtomicBool::new(false),
                highlight: AtomicBool::new(false),
            });
            build_tray(&handle)?;
            Ok(())
        })
        // Alt+F4 puts the window in the tray like the close button; quit from the tray menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    hide_to_tray(window.app_handle());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            reload,
            reset_layers,
            set_paused,
            set_auto_mouse,
            minimize_main,
            hide_main_to_tray,
            select_device,
            import_definition,
            set_ignore_configurators,
            set_click_through,
            set_highlight
        ])
        .run(tauri::generate_context!())
        .expect("error while running CoCoKey");
}
