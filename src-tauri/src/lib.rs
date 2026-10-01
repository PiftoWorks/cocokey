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
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};

use device::{Control, KeyboardInfo, LiveState, Shared};
use protocol::DeviceInfo;

struct AppState {
    shared: Arc<Mutex<Shared>>,
    control: Mutex<Sender<Control>>,
    click_through: AtomicBool,
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

fn japanese() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.starts_with("ja"))
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let ja = japanese();
    let t = |ja_text: &'static str, en_text: &'static str| if ja { ja_text } else { en_text };
    let show = MenuItem::with_id(app, "show", t("表示／非表示", "Show / hide"), true, None::<&str>)?;
    let click = CheckMenuItem::with_id(app, "click", t("クリックを透過", "Click-through"), true, false, None::<&str>)?;
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
    let menu = Menu::with_items(app, &[&show, &click, &sep, &pause, &reload, &reset, &sep2, &quit])?;

    let click_item = click.clone();
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("CoCoKey")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    if w.is_visible().unwrap_or(true) {
                        let _ = w.hide();
                    } else {
                        let _ = w.show();
                    }
                }
            }
            "click" => {
                let enabled = !app.state::<AppState>().click_through.load(Ordering::Relaxed);
                apply_click_through(app, enabled);
                let _ = click_item.set_checked(enabled);
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
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            app.manage(AppState { shared, control: Mutex::new(control), click_through: AtomicBool::new(false) });
            build_tray(&handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            reload,
            reset_layers,
            set_paused,
            set_auto_mouse,
            select_device,
            import_definition,
            set_ignore_configurators,
            set_click_through
        ])
        .run(tauri::generate_context!())
        .expect("error while running CoCoKey");
}
