//! Background worker: finds the keyboard, loads its keymap and keeps the
//! layer state up to date.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hidapi::HidApi;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::definitions::Store;
use crate::engine::{AutoMouse, AutoMouseRules, Engine, QmkAutoMouse, TapDance};
use crate::keycode;
use crate::pointer::{Target, TargetInfo};
use crate::protocol::{self, DeviceInfo, RawHid};

const POLL: Duration = Duration::from_millis(8);
const IDLE: Duration = Duration::from_millis(60);
const RECONNECT: Duration = Duration::from_millis(1500);
const MAX_ERRORS: u32 = 5;
const UNLOCK_CHECK: Duration = Duration::from_secs(2);
const APP_CHECK: Duration = Duration::from_secs(2);

/// Keyboards whose firmware turns on a mouse layer by itself when the
/// built-in pointing device moves.
fn auto_mouse_for(vendor_id: u16, product_id: u16, layers: usize, tapping_term: u16) -> Option<AutoMouse> {
    match (vendor_id, product_id) {
        // Svalboard: the last layer, until a key outside it is pressed.
        (0x303A, 0x4044) => Some(AutoMouse { layer: (layers - 1) as u8, rules: AutoMouseRules::UntilUnmappedKey }),
        // mtk64erp: QMK auto mouse on layer 7 with the keyboard's defaults
        // (AUTO_MOUSE_TIME 1000, AUTO_MOUSE_THRESHOLD 100). The firmware
        // switches it off while layers 1-6 are on top. The timeout can be
        // changed on the keyboard (AM_TOUT_INC / DEC), which is not readable
        // over VIA, so the default is assumed.
        (0x4D59, 0x0300) if layers > 7 => Some(AutoMouse {
            layer: 7,
            rules: AutoMouseRules::Qmk(QmkAutoMouse {
                threshold: 100,
                timeout: Duration::from_millis(1000),
                delay: Duration::from_millis(tapping_term as u64),
                blocked_layers: 0b0111_1110,
            }),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyboardInfo {
    pub id: String,
    pub device: DeviceInfo,
    pub via_protocol: u16,
    pub vial_protocol: Option<u32>,
    pub layers: usize,
    pub rows: usize,
    pub cols: usize,
    pub keymap: Vec<u16>,
    pub tap_dances: Vec<TapDance>,
    pub tapping_term: u16,
    pub definition: Option<serde_json::Value>,
    pub matrix_available: bool,
    pub firmware_reports: bool,
    pub auto_mouse_layer: Option<u8>,
    pub legacy_keycodes: bool,
    /// Raw dynamic macro buffer (see `src/macros.ts` for the format).
    pub macro_buffer: Vec<u8>,
    pub macro_count: u8,
    /// `[layer][encoder] = [counter-clockwise, clockwise]` keycodes.
    pub encoders: Vec<Vec<[u16; 2]>>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LiveState {
    pub connected: bool,
    pub layer_state: u32,
    pub default_layer: u32,
    pub top: u8,
    /// "keys": replayed from key presses, "firmware": reported by the keyboard.
    pub source: &'static str,
    pub pressed: Vec<[u8; 2]>,
    pub auto_mouse: bool,
    pub paused: bool,
    pub notice: Option<String>,
}

#[derive(Default)]
pub struct Shared {
    pub keyboard: Option<KeyboardInfo>,
    pub state: LiveState,
    pub devices: Vec<DeviceInfo>,
}

pub enum Control {
    Reload,
    Reset,
    SetPaused(bool),
    PointerMoved(i32, i32),
    Select(Option<String>),
    /// A definition file was dropped on the window.
    Import(std::path::PathBuf),
    /// Keep tracking even while Vial / VIA is running (at the user's risk).
    IgnoreConfigurators(bool),
}

struct Connection {
    raw: RawHid,
    info: KeyboardInfo,
    engine: Engine,
    matrix: Vec<u32>,
    /// Switches that already read as pressed when we connected. Some boards
    /// report a position as permanently pressed (e.g. a split keyboard's
    /// hand-detection diode wired into the matrix), so ignore those until released.
    ignored: Vec<u32>,
    errors: u32,
    firmware: Option<(u32, u32)>,
    last_unlock_check: Option<Instant>,
}

pub fn spawn(app: AppHandle, shared: Arc<Mutex<Shared>>, pointer_target: Target) -> Sender<Control> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("keyboard-worker".into())
        .spawn(move || Worker::new(app, shared, pointer_target, rx).run())
        .expect("worker thread");
    tx
}

struct Worker {
    app: AppHandle,
    shared: Arc<Mutex<Shared>>,
    pointer_target: Target,
    rx: Receiver<Control>,
    api: Option<HidApi>,
    conn: Option<Connection>,
    paused: bool,
    preferred: Option<String>,
    store: Store,
    /// VIA keyboard found without a known layout: (vendor id, product id).
    awaiting_definition: Option<(u16, u16)>,
    last_attempt: Option<Instant>,
    last_state: Option<LiveState>,
    notice: Option<String>,
    /// Configurator apps running right now (Vial / VIA).
    running: crate::apps::Running,
    /// A keyboard we are leaving alone because a configurator may be using it.
    held_back: Option<(u16, u16)>,
    /// The user chose to keep tracking while a configurator is open.
    ignore_configurators: bool,
    last_app_check: Option<Instant>,
}

impl Worker {
    fn new(app: AppHandle, shared: Arc<Mutex<Shared>>, pointer_target: Target, rx: Receiver<Control>) -> Self {
        let store = {
            use tauri::Manager;
            let paths = app.path();
            Store::new(
                paths.app_config_dir().unwrap_or_else(|_| std::env::temp_dir()),
                paths.app_cache_dir().unwrap_or_else(|_| std::env::temp_dir()),
            )
        };
        Self {
            app,
            shared,
            pointer_target,
            rx,
            api: None,
            conn: None,
            paused: false,
            preferred: None,
            store,
            awaiting_definition: None,
            last_attempt: None,
            last_state: None,
            notice: None,
            running: Default::default(),
            held_back: None,
            ignore_configurators: false,
            last_app_check: None,
        }
    }

    fn run(mut self) {
        loop {
            let now = Instant::now();
            if !self.handle_controls(now) {
                return;
            }
            self.check_configurator(now);
            if self.conn.is_none() {
                self.try_connect(now);
            }
            let active = !self.paused && !self.conn_blocked();
            if active {
                self.poll(now);
            }
            self.publish();
            std::thread::sleep(if self.conn.is_some() && active { POLL } else { IDLE });
        }
    }

    fn handle_controls(&mut self, now: Instant) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(Control::Reload) => {
                    self.disconnect();
                    self.last_attempt = None;
                }
                Ok(Control::Reset) => {
                    if let Some(c) = self.conn.as_mut() {
                        c.engine.reset();
                    }
                }
                Ok(Control::SetPaused(p)) => {
                    self.paused = p;
                    // Vial and the HUD talk over the same interface; start over
                    // cleanly after the user is done editing.
                    if !p {
                        self.disconnect();
                        self.last_attempt = None;
                    }
                }
                Ok(Control::PointerMoved(dx, dy)) => {
                    if let Some(c) = self.conn.as_mut() {
                        c.engine.pointer_moved(dx, dy, now);
                    }
                }
                Ok(Control::Import(path)) => {
                    let result = self.store.import(&path, self.awaiting_definition);
                    let _ = self.app.emit(
                        "hud://import",
                        match &result {
                            Ok((v, p)) => serde_json::json!({ "ok": true, "id": format!("{v:04x}:{p:04x}") }),
                            Err(e) => serde_json::json!({ "ok": false, "error": e }),
                        },
                    );
                    if result.is_ok() {
                        self.disconnect();
                        self.last_attempt = None;
                    }
                }
                Ok(Control::IgnoreConfigurators(on)) => {
                    self.ignore_configurators = on;
                    self.last_app_check = None;
                    self.last_attempt = None;
                }
                Ok(Control::Select(path)) => {
                    self.preferred = path;
                    self.disconnect();
                    self.last_attempt = None;
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    /// Is the connected keyboard one that a running configurator may be editing?
    fn conn_blocked(&self) -> bool {
        self.conn.as_ref().is_some_and(|c| self.running.blocks(c.info.vial_protocol.is_some()))
    }

    fn check_configurator(&mut self, now: Instant) {
        if self.last_app_check.is_some_and(|t| now.duration_since(t) < APP_CHECK) {
            return;
        }
        self.last_app_check = Some(now);
        let running = if self.ignore_configurators { Default::default() } else { crate::apps::configurators() };
        if running == self.running {
            return;
        }
        let before = self.running;
        self.running = running;
        match self.conn.as_ref().map(|c| c.info.vial_protocol.is_some()) {
            // Released by the configurator: the keymap may have been edited meanwhile.
            Some(vial) if before.blocks(vial) && !running.blocks(vial) => {
                self.disconnect();
                self.last_attempt = None;
            }
            Some(vial) if running.blocks(vial) => {
                if let Some(c) = self.conn.as_mut() {
                    c.matrix.iter_mut().for_each(|r| *r = 0);
                }
            }
            None => self.last_attempt = None,
            _ => {}
        }
    }

    fn disconnect(&mut self) {
        if self.conn.take().is_some() {
            *self.pointer_target.lock().unwrap() = None;
            self.shared.lock().unwrap().keyboard = None;
            let _ = self.app.emit("hud://keyboard", Option::<KeyboardInfo>::None);
        }
    }

    fn try_connect(&mut self, now: Instant) {
        if self.paused || self.last_attempt.is_some_and(|t| now.duration_since(t) < RECONNECT) {
            return;
        }
        self.last_attempt = Some(now);
        if self.api.is_none() {
            match HidApi::new() {
                Ok(api) => self.api = Some(api),
                Err(e) => {
                    self.notice = Some(format!("hid: {e}"));
                    return;
                }
            }
        }
        let api = self.api.as_mut().unwrap();
        let _ = api.refresh_devices();
        let devices = protocol::list_devices(api);
        self.shared.lock().unwrap().devices = devices.clone();
        let _ = self.app.emit("hud://devices", &devices);

        let mut order: Vec<&DeviceInfo> = devices.iter().collect();
        if let Some(p) = &self.preferred {
            order.sort_by_key(|d| &d.path != p);
        }
        let mut last_err = None;
        self.awaiting_definition = None;
        self.held_back = None;
        for d in order {
            // Never talk to a keyboard a running configurator may be using: its
            // replies would reach that app too. Unknown keyboards count as busy.
            let busy = match self.store.is_vial(d.vendor_id, d.product_id) {
                Some(vial) => self.running.blocks(vial),
                None => self.running.vial || self.running.via,
            };
            if busy {
                self.held_back.get_or_insert((d.vendor_id, d.product_id));
                continue;
            }
            match load(api, d, &self.store) {
                Ok(conn) => {
                    self.store.remember_kind(d.vendor_id, d.product_id, conn.info.vial_protocol.is_some());
                    self.held_back = None;
                    *self.pointer_target.lock().unwrap() = Some(TargetInfo {
                        vendor_id: conn.info.device.vendor_id,
                        product_id: conn.info.device.product_id,
                        pointer: conn.info.auto_mouse_layer.is_some(),
                    });
                    self.notice = None;
                    self.shared.lock().unwrap().keyboard = Some(conn.info.clone());
                    let _ = self.app.emit("hud://keyboard", Some(&conn.info));
                    self.conn = Some(conn);
                    return;
                }
                Err(LoadError::NeedDefinition) => {
                    self.awaiting_definition = Some((d.vendor_id, d.product_id));
                    last_err = Some(format!("definition-missing:{}", d.product));
                }
                Err(LoadError::Other(e)) => {
                    if self.awaiting_definition.is_none() {
                        last_err = Some(format!("{}: {e}", d.product));
                    }
                }
            }
        }
        self.notice = last_err;
        // Typing on a keyboard we are leaving alone still counts as typing on
        // the tracked keyboard, so the HUD does not flip to a plain layout.
        if let Some((vendor_id, product_id)) = self.held_back {
            *self.pointer_target.lock().unwrap() = Some(TargetInfo { vendor_id, product_id, pointer: false });
        }
    }

    fn poll(&mut self, now: Instant) {
        let Some(c) = self.conn.as_mut() else { return };
        for r in c.raw.take_reports() {
            c.firmware = Some((r.layer_state, r.default_layer));
            c.engine.sync_from_firmware(r.layer_state, r.default_layer);
        }
        if c.info.matrix_available {
            match c.raw.matrix_state(c.info.rows, c.info.cols) {
                Ok(mut m) => {
                    for (row, bits) in m.iter_mut().enumerate() {
                        if let Some(ign) = c.ignored.get_mut(row) {
                            *ign &= *bits; // released once -> tracked from now on
                            *bits &= !*ign;
                        }
                    }
                    c.errors = 0;
                    for row in 0..c.info.rows.min(m.len()) {
                        let changed = m[row] ^ c.matrix[row];
                        if changed == 0 {
                            continue;
                        }
                        for col in 0..c.info.cols {
                            if changed & (1 << col) == 0 {
                                continue;
                            }
                            if m[row] & (1 << col) != 0 {
                                c.engine.key_down(row, col, now);
                            } else {
                                c.engine.key_up(row, col, now);
                            }
                        }
                    }
                    c.matrix = m;
                }
                Err(_) => c.errors += 1,
            }
        } else if c.last_unlock_check.is_none_or(|t| now.duration_since(t) >= UNLOCK_CHECK) {
            // Locked in Vial: check now and then whether the user unlocked it.
            c.last_unlock_check = Some(now);
            if c.info.vial_protocol.is_some() {
                if c.raw.vial_unlocked() {
                    c.info.matrix_available = true;
                } else if c.raw.via_protocol().is_err() {
                    c.errors += 1;
                }
            } else if c.raw.via_protocol().is_err() {
                c.errors += 1;
            }
        }
        c.engine.tick(now);
        if let Some((state, default)) = c.firmware {
            // The firmware is authoritative; keep the replica aligned.
            c.engine.layer_state = state;
            if default != 0 {
                c.engine.default_layer = default;
            }
        }
        if c.errors >= MAX_ERRORS {
            self.notice = Some("disconnected".into());
            self.disconnect();
        }
    }

    fn publish(&mut self) {
        let state = match &self.conn {
            Some(c) => {
                let mut pressed = vec![];
                for (row, bits) in c.matrix.iter().enumerate() {
                    for col in 0..c.info.cols {
                        if bits & (1 << col) != 0 {
                            pressed.push([row as u8, col as u8]);
                        }
                    }
                }
                LiveState {
                    connected: true,
                    layer_state: c.engine.layer_state,
                    default_layer: c.engine.default_layer,
                    top: c.engine.top_layer(),
                    source: if c.firmware.is_some() { "firmware" } else { "keys" },
                    pressed,
                    auto_mouse: c.engine.auto_mouse_active(),
                    paused: self.paused || self.conn_blocked(),
                    notice: if self.conn_blocked() {
                        Some("configurator-running".into())
                    } else if c.info.matrix_available || c.firmware.is_some() {
                        self.notice.clone()
                    } else {
                        Some("matrix-locked".into())
                    },
                }
            }
            None => LiveState {
                connected: false,
                paused: self.paused || self.held_back.is_some(),
                notice: if self.held_back.is_some() { Some("configurator-running".into()) } else { self.notice.clone() },
                ..Default::default()
            },
        };
        if self.last_state.as_ref() != Some(&state) {
            self.shared.lock().unwrap().state = state.clone();
            let _ = self.app.emit("hud://state", &state);
            self.last_state = Some(state);
        }
    }
}

/// Why a keyboard could not be loaded.
enum LoadError {
    /// A VIA keyboard whose layout is not known yet.
    NeedDefinition,
    Other(String),
}

impl From<String> for LoadError {
    fn from(e: String) -> Self {
        LoadError::Other(e)
    }
}

/// Encoder indexes used in a definition: KLE legends carry them as "e<n>" in
/// the 10th legend slot, the pre-parsed catalog format as an "ei" field.
fn encoder_ids(def: &serde_json::Value) -> Vec<u8> {
    let mut ids = vec![];
    let layouts = def.get("layouts");
    for row in layouts.and_then(|l| l.get("keymap")).and_then(|k| k.as_array()).into_iter().flatten() {
        for item in row.as_array().into_iter().flatten() {
            if let Some(legend) = item.as_str() {
                if let Some(e) = legend.split('\n').nth(9) {
                    if let Some(n) = e.strip_prefix('e').and_then(|n| n.parse::<u8>().ok()) {
                        ids.push(n);
                    }
                }
            }
        }
    }
    for key in layouts.and_then(|l| l.get("keys")).and_then(|k| k.as_array()).into_iter().flatten() {
        if let Some(n) = key.get("ei").and_then(|v| v.as_u64()) {
            ids.push(n as u8);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn load(api: &HidApi, d: &DeviceInfo, store: &Store) -> Result<Connection, LoadError> {
    let mut raw = RawHid::open(api, &d.path)?;
    let via_protocol = raw.via_protocol()?;
    let layers = raw.layer_count()? as usize;
    let vial = raw.vial_id();
    let definition = match vial {
        Some(_) => raw.vial_definition()?,
        // VIA / Remap keyboards: the layout comes from a definition file.
        None => store.lookup(d.vendor_id, d.product_id).ok_or(LoadError::NeedDefinition)?,
    };
    let (rows, cols) = definition
        .get("matrix")
        .and_then(|m| Some((m.get("rows")?.as_u64()? as usize, m.get("cols")?.as_u64()? as usize)))
        .ok_or_else(|| "definition without matrix size".to_string())?;
    let definition = Some(definition);
    if layers == 0 || rows == 0 || cols == 0 || cols > 32 {
        return Err(format!("unsupported matrix {rows}x{cols}, {layers} layers").into());
    }
    let legacy_keycodes = match vial {
        Some((proto, _)) => proto < 6,
        None => via_protocol < 12,
    };
    let mut keymap = raw.read_keymap(layers, rows, cols)?;
    let mut tap_dances = if vial.is_some() { raw.tap_dances() } else { vec![] };
    if legacy_keycodes {
        // Older firmware numbers some keycodes differently; use one numbering everywhere.
        keymap.codes.iter_mut().for_each(|kc| *kc = keycode::from_legacy(*kc));
        for td in &mut tap_dances {
            for kc in [&mut td.on_tap, &mut td.on_hold, &mut td.on_double_tap, &mut td.on_tap_hold] {
                *kc = keycode::from_legacy(*kc);
            }
        }
    }
    let tapping_term = if vial.is_some() { raw.tapping_term().unwrap_or(200) } else { 200 };
    let (macro_count, macro_buffer) = raw.read_macros().unwrap_or_default();
    let encoder_list = definition.as_ref().map(encoder_ids).unwrap_or_default();
    let mut encoders = if encoder_list.is_empty() || (vial.is_none() && via_protocol < 10) {
        vec![]
    } else {
        raw.encoder_keycodes(vial.is_some(), layers, &encoder_list)
    };
    if legacy_keycodes {
        for kc in encoders.iter_mut().flatten().flatten() {
            *kc = keycode::from_legacy(*kc);
        }
    }
    if vial.is_some() {
        // Vial keeps the original single-reply matrix format.
        raw.set_paged_matrix(false);
    }
    let fits = raw.matrix_readable(rows, cols);
    let first_matrix = if fits { raw.matrix_state(rows, cols).ok() } else { None };
    let matrix_available = first_matrix.is_some() && (vial.is_none() || raw.vial_unlocked());
    let ignored = if matrix_available { first_matrix.unwrap_or_default() } else { vec![0; rows] };
    let firmware = raw.subscribe_layer_reports();
    let auto_mouse = auto_mouse_for(d.vendor_id, d.product_id, layers, tapping_term);
    let auto_mouse_layer = auto_mouse.map(|am| am.layer);
    let id = match vial {
        Some((_, uid)) => uid.iter().map(|b| format!("{b:02x}")).collect(),
        None => format!("{:04x}:{:04x}", d.vendor_id, d.product_id),
    };

    let mut engine = Engine::new(keymap.clone(), tap_dances.clone(), tapping_term);
    engine.set_auto_mouse(auto_mouse);
    if let Some(r) = firmware {
        engine.sync_from_firmware(r.layer_state, r.default_layer);
    }
    let info = KeyboardInfo {
        id,
        device: d.clone(),
        via_protocol,
        vial_protocol: vial.map(|v| v.0),
        layers,
        rows,
        cols,
        keymap: keymap.codes,
        tap_dances,
        tapping_term,
        definition,
        matrix_available,
        firmware_reports: firmware.is_some(),
        auto_mouse_layer,
        legacy_keycodes,
        macro_buffer,
        macro_count,
        encoders,
    };
    Ok(Connection {
        raw,
        info,
        engine,
        matrix: vec![0; rows],
        ignored,
        errors: 0,
        firmware: firmware.map(|r| (r.layer_state, r.default_layer)),
        last_unlock_check: None,
    })
}
