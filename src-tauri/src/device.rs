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
/// While the keyboard's own pointing device is moving, read the switch matrix
/// at most this often. Every request makes the keyboard stop and answer; at
/// the normal pace (about 100 a second) that can make its built-in pointing
/// device move the cursor unevenly. 40 ms (25 a second) still sees every key
/// held for 50 ms or longer, e.g. a quick tap of a layer key.
const POINTER_POLL: Duration = Duration::from_millis(40);
/// The longest scheduled wait from the start of the previous read while the
/// pointer moves. At most 72 ms allows the next 8 ms round to see an 80 ms
/// press when a matrix answer takes up to 40 ms. Longer answers can still
/// block the synchronous worker beyond this limit.
const POINTER_POLL_MAX: Duration = Duration::from_millis(72);
/// The pointer counts as moving until it has been still for this long.
const POINTER_REST: Duration = Duration::from_millis(200);
/// While the pointer moves, a read also waits this many times as long as the
/// previous read took. A normal read takes a few milliseconds, so this only
/// matters when the keyboard (or the PC) is slow to answer: then it is asked
/// less often instead of being kept busy.
const SLOW_READ_SPACING: u32 = 4;

/// Keyboards whose firmware turns on a mouse layer by itself when the
/// built-in pointing device moves, and whether it does so out of the box.
///
/// The values come from each keyboard's stock VIA / Vial firmware. Most of
/// them let the user switch the feature and change the timeout on the
/// keyboard (stored in EEPROM, not readable over VIA), so the HUD has a
/// per-keyboard switch and otherwise assumes the defaults.
fn auto_mouse_for(vendor_id: u16, product_id: u16, layers: usize, tapping_term: u16) -> Option<(AutoMouse, bool)> {
    let qmk = |layer: u8, threshold: i32, timeout_ms: u64, blocked_layers: u32, mouse_keys: &'static [(u16, u16)]| {
        ((layer as usize) < layers).then_some(AutoMouse {
            layer,
            rules: AutoMouseRules::Qmk(QmkAutoMouse {
                threshold,
                timeout: Duration::from_millis(timeout_ms),
                delay: Duration::from_millis(tapping_term as u64),
                blocked_layers,
                mouse_keys,
            }),
        })
    };
    let (am, on_by_default) = match (vendor_id, product_id) {
        // Svalboard: the last layer, until a key outside it is pressed.
        (0x303A, 0x4044) => {
            (Some(AutoMouse { layer: (layers - 1) as u8, rules: AutoMouseRules::UntilUnmappedKey }), true)
        }
        // mtk64erp: layer 7, 1000 ms, threshold 100. The firmware switches the
        // feature off while layers 1-6 are on top.
        (0x4D59, 0x0300) => (qmk(7, 100, 1000, 0b0111_1110, &[]), true),
        // Keyball 46 / 61 / 39 / 44 / plus (QMK 0.22: no threshold). Off until
        // the user turns it on with AML_TO. SCRL_MO counts as a mouse key.
        (0x5957, 0x0100) => (qmk(2, 0, 650, 0, &[(0x7E07, 0x7E07)]), false),
        (0x5957, 0x0001 | 0x0200 | 0x0400 | 0x0500) => (qmk(1, 0, 650, 0, &[(0x7E07, 0x7E07)]), false),
        // cocot36plus / cocot38mini / cocot_micro (Vial, no threshold): on by
        // default; layers 1-2 (scroll) switch it off. SCRL_MO is a mouse key.
        (0x1727, 0x0014 | 0x0011) => (qmk(4, 0, 650, 0b110, &[(0x7E04, 0x7E04)]), true),
        (0x1727, 0x0013) => (qmk(4, 0, 650, 0b110, &[]), true),
        // Bastard Keyboards Charybdis / Dilemma (threshold 10). Off until
        // turned on in Argos. Its pointer mode keys count as mouse keys.
        (0xA8F8, 0x1832 | 0x1836) => (qmk(4, 10, 650, 0, &[(0x7E00, 0x7E20)]), false),
        (0xA8F8, 0x1833 | 0x1834 | 0x1837) => (qmk(3, 10, 650, 0, &[(0x7E00, 0x7E20)]), false),
        (0xA8F8, 0x1835) => (qmk(2, 10, 650, 0, &[(0x7E00, 0x7E20)]), false),
        // torabo-tsuki (Vial): off by default, layer 1, 100 ms, threshold 10.
        // KC_NO and modifiers count as mouse keys.
        (0xFEED, 0xBCAD) => (qmk(1, 10, 100, 0, &[(0x0000, 0x0000), (0x00E0, 0x00E7)]), false),
        _ => return None,
    };
    am.map(|am| (am, on_by_default))
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
    /// Whether the keyboard's firmware has the mouse layer on out of the box.
    pub auto_mouse_default: bool,
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
    PointerMoved(i32, i32, i32),
    /// A key (or mouse button) of the tracked keyboard went down, seen through
    /// Raw Input: read the matrix right away, even while the pointer moves.
    KeyActivity,
    /// The user switched the keyboard's automatic mouse layer on or off.
    SetAutoMouse(bool),
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
    /// Mouse layer rules for this keyboard (used when the user enables it).
    auto_mouse: Option<AutoMouse>,
    /// When the keyboard's own pointing device last moved, and when the
    /// matrix was last read (see POINTER_POLL).
    last_pointer_move: Option<Instant>,
    last_matrix_read: Option<Instant>,
    /// How long the last matrix read took (see SLOW_READ_SPACING).
    last_read_took: Duration,
    /// Raw Input saw a key of this keyboard go down since the last read.
    read_now: bool,
    /// Where that time comes from (a stand-in clock in the tests).
    clock: Box<dyn Fn() -> Instant>,
}

impl Connection {
    /// The keyboard's own pointing device moved (seen through Raw Input).
    fn pointer_moved(&mut self, dx: i32, dy: i32, dw: i32, now: Instant) {
        self.last_pointer_move = Some(now);
        self.engine.pointer_moved(dx, dy, dw, now);
    }

    /// A key of this keyboard went down (Raw Input): the next round reads the
    /// matrix whatever the pace. Keys that send nothing to the PC (layer keys)
    /// are only seen by the pace itself.
    fn key_activity(&mut self) {
        self.read_now = true;
    }

    /// Read the matrix this round? Every round, except while the keyboard's
    /// pointer is moving: then only every POINTER_POLL, or longer after slow
    /// reads (SLOW_READ_SPACING) but never longer than POINTER_POLL_MAX. A key
    /// going down (Raw Input) makes the next round read in any case. The
    /// resting pace never changes. Rounds that come late never make up for the
    /// reads they missed: the next read is counted from the last one done.
    fn matrix_due(&mut self, now: Instant) -> bool {
        let moving = self.last_pointer_move.is_some_and(|t| now.duration_since(t) < POINTER_REST);
        let wait = if moving {
            POINTER_POLL.max(self.last_read_took * SLOW_READ_SPACING).min(POINTER_POLL_MAX)
        } else {
            Duration::ZERO
        };
        let read_now = std::mem::take(&mut self.read_now);
        if !read_now && self.last_matrix_read.is_some_and(|t| now.duration_since(t) < wait) {
            return false;
        }
        self.last_matrix_read = Some(now);
        true
    }

    /// One round of the worker loop: layer reports, the switch matrix (or the
    /// unlock check) and the engine's timers.
    fn poll(&mut self, now: Instant) {
        for r in self.raw.take_reports() {
            self.firmware = Some((r.layer_state, r.default_layer));
            self.engine.sync_from_firmware(r.layer_state, r.default_layer);
        }
        if self.info.matrix_available {
            if self.matrix_due(now) {
                let started = (self.clock)();
                self.read_matrix(now);
                self.last_read_took = (self.clock)().saturating_duration_since(started);
            }
        } else if self.last_unlock_check.is_none_or(|t| now.duration_since(t) >= UNLOCK_CHECK) {
            // Locked in Vial: check now and then whether the user unlocked it.
            self.last_unlock_check = Some(now);
            if self.info.vial_protocol.is_some() {
                if self.raw.vial_unlocked() {
                    self.info.matrix_available = true;
                } else if self.raw.via_protocol().is_err() {
                    self.errors += 1;
                }
            } else if self.raw.via_protocol().is_err() {
                self.errors += 1;
            }
        }
        self.engine.tick(now);
        if let Some((state, default)) = self.firmware {
            // The firmware is authoritative; keep the replica aligned.
            self.engine.layer_state = state;
            if default != 0 {
                self.engine.default_layer = default;
            }
        }
    }

    fn read_matrix(&mut self, now: Instant) {
        match self.raw.matrix_state(self.info.rows, self.info.cols) {
            Ok(mut m) => {
                for (row, bits) in m.iter_mut().enumerate() {
                    if let Some(ign) = self.ignored.get_mut(row) {
                        *ign &= *bits; // released once -> tracked from now on
                        *bits &= !*ign;
                    }
                }
                self.errors = 0;
                for row in 0..self.info.rows.min(m.len()) {
                    let changed = m[row] ^ self.matrix[row];
                    if changed == 0 {
                        continue;
                    }
                    for col in 0..self.info.cols {
                        if changed & (1 << col) == 0 {
                            continue;
                        }
                        if m[row] & (1 << col) != 0 {
                            self.engine.key_down(row, col, now);
                        } else {
                            self.engine.key_up(row, col, now);
                        }
                    }
                }
                self.matrix = m;
            }
            Err(_) => self.errors += 1,
        }
    }
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
                Ok(Control::PointerMoved(dx, dy, dw)) => {
                    if let Some(c) = self.conn.as_mut() {
                        c.pointer_moved(dx, dy, dw, now);
                    }
                }
                Ok(Control::KeyActivity) => {
                    if let Some(c) = self.conn.as_mut() {
                        c.key_activity();
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
                Ok(Control::SetAutoMouse(on)) => {
                    if let Some(c) = self.conn.as_mut() {
                        c.engine.set_auto_mouse(c.auto_mouse.filter(|_| on));
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
                    // The pointer is reported also without a mouse layer rule:
                    // the matrix is read less often while it moves.
                    *self.pointer_target.lock().unwrap() = Some(TargetInfo {
                        vendor_id: conn.info.device.vendor_id,
                        product_id: conn.info.device.product_id,
                        pointer: true,
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
        c.poll(now);
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
    let auto_mouse_layer = auto_mouse.map(|(am, _)| am.layer);
    let auto_mouse_default = auto_mouse.is_some_and(|(_, on)| on);
    let auto_mouse = auto_mouse.map(|(am, _)| am);
    let id = match vial {
        Some((_, uid)) => uid.iter().map(|b| format!("{b:02x}")).collect(),
        None => format!("{:04x}:{:04x}", d.vendor_id, d.product_id),
    };

    let mut engine = Engine::new(keymap.clone(), tap_dances.clone(), tapping_term);
    engine.set_auto_mouse(auto_mouse.filter(|_| auto_mouse_default));
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
        auto_mouse_default,
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
        auto_mouse,
        last_pointer_move: None,
        last_matrix_read: None,
        last_read_took: Duration::ZERO,
        read_now: false,
        clock: Box::new(Instant::now),
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;
    use crate::engine::Keymap;
    use crate::protocol::Transport;

    const ROWS: usize = 14;
    const COLS: usize = 7;
    /// MO(1) at row 0, col 0.
    const MO1: u16 = 0x5221;

    /// What the stand-in keyboard saw, shared with the test.
    struct Log {
        /// Simulated time zero.
        t0: Instant,
        /// Simulated time in ms (the test moves it forward).
        now_ms: Cell<u64>,
        /// How long the keyboard takes to answer a matrix request (ms).
        read_ms: Cell<u64>,
        /// When each matrix request arrived (ms).
        matrix_requests: RefCell<Vec<u64>>,
        /// Keys held: (row, col, from ms, until ms).
        held: RefCell<Vec<(usize, usize, u64, u64)>>,
    }

    impl Default for Log {
        fn default() -> Self {
            Self {
                t0: Instant::now(),
                now_ms: Cell::new(0),
                read_ms: Cell::new(0),
                matrix_requests: RefCell::new(vec![]),
                held: RefCell::new(vec![]),
            }
        }
    }

    /// Answers like a VIA protocol 12 keyboard with a 14 x 7 matrix (one
    /// paged reply holds every row) and counts the matrix requests.
    struct FakeKeyboard {
        log: Rc<Log>,
        replies: VecDeque<[u8; 32]>,
    }

    impl Transport for FakeKeyboard {
        fn write(&mut self, data: &[u8]) -> Result<usize, String> {
            let req = &data[1..];
            let mut reply = [0u8; 32];
            let n = req.len().min(32);
            reply[..n].copy_from_slice(&req[..n]);
            if req[0] == 0x02 && req[1] == 0x03 {
                let now = self.log.now_ms.get();
                self.log.matrix_requests.borrow_mut().push(now);
                let offset = req[2] as usize;
                for &(row, col, from, until) in self.log.held.borrow().iter() {
                    if (from..until).contains(&now) && row >= offset && 3 + row - offset < 32 {
                        reply[3 + row - offset] |= 1 << col;
                    }
                }
                // The answer takes this long (the round's own time is set again next round).
                self.log.now_ms.set(now + self.log.read_ms.get());
            }
            self.replies.push_back(reply);
            Ok(data.len())
        }

        fn read_timeout(&mut self, buf: &mut [u8], _timeout_ms: i32) -> Result<usize, String> {
            match self.replies.pop_front() {
                Some(r) => {
                    buf[..32].copy_from_slice(&r);
                    Ok(32)
                }
                None => Ok(0),
            }
        }
    }

    fn connection(log: Rc<Log>) -> Connection {
        let clock_log = log.clone();
        let mut raw = RawHid::with_transport(Box::new(FakeKeyboard { log, replies: VecDeque::new() }));
        raw.set_paged_matrix(true);
        let mut codes = vec![0u16; 2 * ROWS * COLS];
        codes[0] = MO1;
        let keymap = Keymap { layers: 2, rows: ROWS, cols: COLS, codes: codes.clone() };
        let info = KeyboardInfo {
            id: "4d59:0300".into(),
            device: DeviceInfo {
                path: "fake".into(),
                vendor_id: 0x4D59,
                product_id: 0x0300,
                product: "fake".into(),
                manufacturer: "fake".into(),
            },
            via_protocol: 12,
            vial_protocol: None,
            layers: 2,
            rows: ROWS,
            cols: COLS,
            keymap: codes,
            tap_dances: vec![],
            tapping_term: 200,
            definition: None,
            matrix_available: true,
            firmware_reports: false,
            auto_mouse_layer: None,
            auto_mouse_default: false,
            legacy_keycodes: false,
            macro_buffer: vec![],
            macro_count: 0,
            encoders: vec![],
        };
        Connection {
            raw,
            engine: Engine::new(keymap, vec![], 200),
            info,
            matrix: vec![0; ROWS],
            ignored: vec![0; ROWS],
            errors: 0,
            firmware: None,
            last_unlock_check: None,
            auto_mouse: None,
            last_pointer_move: None,
            last_matrix_read: None,
            last_read_took: Duration::ZERO,
            read_now: false,
            clock: Box::new(move || clock_log.t0 + Duration::from_millis(clock_log.now_ms.get())),
        }
    }

    /// Runs the worker's rounds for `ms` milliseconds, sleeping one POLL after
    /// each round finishes. `moving(t)` says whether the keyboard's pointer
    /// reported movement just before the round at time t; `inspect` sees the
    /// connection after every round.
    fn run(c: &mut Connection, log: &Log, ms: u64, moving: impl Fn(u64) -> bool, inspect: impl FnMut(&Connection)) {
        let step = POLL.as_millis() as u64;
        let mut t = 0;
        let mut inspect = inspect;
        while t < ms {
            log.now_ms.set(t);
            let now = log.t0 + Duration::from_millis(t);
            if moving(t) {
                c.pointer_moved(3, -2, 0, now);
            }
            c.poll(now);
            inspect(c);
            t = (t + step).max(log.now_ms.get() + step);
        }
    }

    /// Like `run`, with the rounds at the given times (ms), e.g. with a gap
    /// where the PC was too busy to run the loop.
    fn run_at(
        c: &mut Connection,
        log: &Log,
        rounds: impl IntoIterator<Item = u64>,
        moving: impl Fn(u64) -> bool,
        mut inspect: impl FnMut(&Connection),
    ) {
        for t in rounds {
            log.now_ms.set(t);
            let now = log.t0 + Duration::from_millis(t);
            if moving(t) {
                c.pointer_moved(3, -2, 0, now);
            }
            c.poll(now);
            inspect(c);
        }
    }

    fn requests_between(log: &Log, from: u64, until: u64) -> usize {
        log.matrix_requests.borrow().iter().filter(|&&t| (from..until).contains(&t)).count()
    }

    #[test]
    fn matrix_is_read_every_round_while_the_pointer_rests() {
        // Unchanged from before: short taps (layer keys, TG, OSL) need the fast pace.
        let log = Rc::new(Log::default());
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |_| false, |_| {});
        assert_eq!(requests_between(&log, 0, 1000), 125);
    }

    #[test]
    fn matrix_reads_back_off_while_the_keyboards_pointer_moves() {
        // Answering a matrix request keeps the keyboard busy; while its own
        // pointer is moving, ask less often.
        let log = Rc::new(Log::default());
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |_| true, |_| {});
        let n = requests_between(&log, 0, 1000);
        assert!(n <= 26, "{n} matrix requests in 1 s while the pointer moved");
        assert!(n >= 20, "{n}: quick taps while moving (layer keys) must still show");
        assert!(min_gap(&log) >= 40, "requests only {} ms apart while the pointer moved", min_gap(&log));
    }

    fn min_gap(log: &Log) -> u64 {
        let times = log.matrix_requests.borrow().clone();
        times.windows(2).map(|w| w[1] - w[0]).min().unwrap()
    }

    #[test]
    fn the_pointer_counts_as_moving_between_sparse_reports() {
        // On a busy PC the movement reports reach the loop late and far apart
        // (up to 90 ms with every core busy). The gaps between them must not
        // count as the pointer having stopped.
        let log = Rc::new(Log::default());
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |t| t % 120 == 0, |_| {});
        let n = requests_between(&log, 0, 1000);
        assert!(n <= 26, "{n} matrix requests in 1 s with a movement report every 120 ms");
    }

    #[test]
    fn slow_reads_are_spaced_out_while_the_pointer_moves() {
        // Reads that take 15 ms mean the keyboard or the PC is struggling:
        // while the pointer moves, leave the keyboard alone for longer.
        let log = Rc::new(Log::default());
        log.read_ms.set(15);
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |_| true, |_| {});
        let n = requests_between(&log, 0, 1000);
        assert!(n <= 17, "{n} matrix requests in 1 s although each took 15 ms");
        assert!(min_gap(&log) >= 60, "slow requests only {} ms apart", min_gap(&log));
    }

    #[test]
    fn the_wait_after_slow_reads_has_a_ceiling() {
        // The scheduled wait has a ceiling, but a 200 ms answer still blocks
        // the synchronous worker before it can start the next round.
        let log = Rc::new(Log::default());
        log.read_ms.set(200);
        let mut c = connection(log.clone());
        run(&mut c, &log, 2000, |_| true, |_| {});
        let times = log.matrix_requests.borrow().clone();
        let max_gap = times.windows(2).map(|w| w[1] - w[0]).max().unwrap();
        assert!(max_gap >= 208, "reads only {max_gap} ms apart after 200 ms answers");
        assert!(max_gap <= 216, "reads {max_gap} ms apart after 200 ms answers");
    }

    #[test]
    fn slow_reads_do_not_slow_the_resting_pace() {
        // At rest every round reads (short taps, TG, OSL), after the prior
        // answer and the worker's sleep.
        let log = Rc::new(Log::default());
        log.read_ms.set(40);
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |_| false, |_| {});
        assert_eq!(requests_between(&log, 0, 1000), 21);
    }

    #[test]
    fn a_late_round_does_not_make_up_for_missed_reads() {
        // The loop did not run from 400 to 900 ms (PC too busy). When it comes
        // back, it reads once and keeps the pace instead of catching up.
        let log = Rc::new(Log::default());
        let mut c = connection(log.clone());
        let rounds = (0u64..400).step_by(8).chain((900u64..1100).step_by(8));
        run_at(&mut c, &log, rounds, |_| true, |_| {});
        assert_eq!(requests_between(&log, 900, 940), 1, "requests right after the late round");
        assert!(requests_between(&log, 900, 1100) <= 5);
    }

    #[test]
    fn matrix_reads_return_to_the_fast_pace_soon_after_the_pointer_stops() {
        let log = Rc::new(Log::default());
        let mut c = connection(log.clone());
        run(&mut c, &log, 1000, |t| t < 300, |_| {});
        // Movement ends at 296 ms; from 200 ms later every round reads again.
        assert_eq!(requests_between(&log, 504, 1000), (1000 - 504) / 8);
    }

    #[test]
    fn a_key_held_while_the_pointer_moves_still_shows() {
        // MO(1) held for 120 ms in the middle of a pointer movement.
        let log = Rc::new(Log::default());
        log.held.borrow_mut().push((0, 0, 400, 520));
        let mut c = connection(log.clone());
        let mut seen_pressed = false;
        let mut seen_layer = false;
        run(&mut c, &log, 1000, |_| true, |c| {
            seen_pressed |= c.matrix[0] & 1 != 0;
            seen_layer |= c.engine.layer_state & 0b10 != 0;
        });
        assert!(seen_pressed, "the held key never showed as pressed");
        assert!(seen_layer, "MO(1) never turned layer 1 on");
        assert_eq!(c.matrix[0] & 1, 0, "released key still shows as pressed");
        assert_eq!(c.engine.layer_state & 0b10, 0, "layer 1 stayed on after release");
    }

    /// Holds MO(1) from `from` for `ms` while the pointer moves the whole time;
    /// returns whether the key and layer 1 were seen, and whether both were
    /// gone again at the end.
    fn short_press_while_moving(from: u64, ms: u64, read_ms: u64, key_activity: bool) -> (bool, bool) {
        let log = Rc::new(Log::default());
        log.read_ms.set(read_ms);
        log.held.borrow_mut().push((0, 0, from, from + ms));
        let mut c = connection(log.clone());
        let mut seen = false;
        let step = POLL.as_millis() as u64;
        let mut t = 0;
        let mut previous_round = None;
        while t < 1000 {
            log.now_ms.set(t);
            let now = log.t0 + Duration::from_millis(t);
            c.pointer_moved(3, -2, 0, now);
            // Raw Input reports the key going down between this round and the previous one.
            if key_activity && from <= t && previous_round.is_none_or(|previous| from > previous) {
                c.key_activity();
            }
            c.poll(now);
            seen |= c.matrix[0] & 1 != 0 && c.engine.layer_state & 0b10 != 0;
            previous_round = Some(t);
            t = (t + step).max(log.now_ms.get() + step);
        }
        let gone = c.matrix[0] & 1 == 0 && c.engine.layer_state & 0b10 == 0;
        (seen, gone)
    }

    #[test]
    fn a_50_ms_tap_while_the_pointer_moves_is_seen() {
        // Every start time within one read interval, so no lucky timing.
        for from in (400..440).step_by(2) {
            let (seen, gone) = short_press_while_moving(from, 50, 0, false);
            assert!(seen, "MO(1) held 50 ms from {from} ms was missed");
            assert!(gone, "MO(1) held 50 ms from {from} ms stayed on");
        }
    }

    #[test]
    fn an_80_ms_tap_is_seen_with_20_to_40_ms_answers_at_every_phase() {
        for read_ms in 20..=40 {
            for from in 400..480 {
                let (seen, gone) = short_press_while_moving(from, 80, read_ms, false);
                assert!(seen, "MO(1) held 80 ms from {from} ms was missed with {read_ms} ms answers");
                assert!(gone, "MO(1) held 80 ms from {from} ms stayed on");
            }
        }
    }

    #[test]
    fn an_80_ms_tap_can_be_missed_with_200_ms_answers() {
        let missed = (400..600).filter(|&from| !short_press_while_moving(from, 80, 200, false).0).count();
        assert!(missed > 0, "every 80 ms tap was seen despite 200 ms answers");
    }

    #[test]
    fn a_key_that_reaches_the_pc_is_read_at_once_while_the_pointer_moves() {
        // 20 ms is shorter than the pace; a key that sends something to the PC
        // (Raw Input sees it go down) is read in the next round anyway.
        for from in (401..441).step_by(4) {
            let (seen, _) = short_press_while_moving(from, 20, 0, true);
            assert!(seen, "a 20 ms press from {from} ms reported by Raw Input was missed");
        }
        // Without the report the pace alone misses some of them: this is the
        // case the report is for.
        let missed = (401..441).step_by(4).filter(|&from| !short_press_while_moving(from, 20, 0, false).0).count();
        assert!(missed > 0, "the pace alone already sees every 20 ms press; the test proves nothing");
    }

    #[test]
    fn the_worker_hands_key_activity_to_the_connection() {
        let src = include_str!("device.rs");
        let body: String = src.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        let wanted = ["Ok(Control::KeyActivity)=>{", "ifletSome(c)=self.conn.as_mut(){", "c.key_activity();"].concat();
        assert!(body.contains(&wanted), "KeyActivity does not reach Connection::key_activity");
        let lib: String = include_str!("lib.rs").split_whitespace().collect();
        let sent = ["pointer::RawEvent::TrackedKey=>{", "let_=pointer_tx.send(Control::KeyActivity);"].concat();
        assert!(lib.contains(&sent), "Raw Input's TrackedKey is not sent to the worker");
        let pointer: String = include_str!("pointer.rs").split_whitespace().collect();
        let on_key = ["ifon_target{", "ifflags&RI_KEY_BREAK==0{", "(ctx.sink)(RawEvent::TrackedKey);"].concat();
        assert!(pointer.contains(&on_key), "a key of the tracked keyboard going down is not reported");
    }

    #[test]
    fn the_worker_hands_pointer_movement_to_the_connection() {
        // The pace above only works if the loop passes Raw Input movement to
        // Connection::pointer_moved, and Raw Input reports it for every
        // tracked keyboard (not only those with a mouse layer rule).
        let src = include_str!("device.rs");
        let body: String = src.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        let wanted = [
            "Ok(Control::PointerMoved(dx,dy,dw))=>{",
            "ifletSome(c)=self.conn.as_mut(){",
            "c.pointer_moved(dx,dy,dw,now);",
        ]
        .concat();
        assert!(body.contains(&wanted), "PointerMoved does not reach Connection::pointer_moved");
        let report = ["product_id:conn.info.device.product_id,", "pointer:true,"].concat();
        assert!(body.contains(&report), "the tracked keyboard's pointer movement is not reported");
    }
}
