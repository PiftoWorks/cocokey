//! Host-side replica of the QMK layer logic.
//!
//! The keyboard does not tell the host which layer is active, so the HUD
//! watches the switch matrix and replays the layer keycodes it finds in the
//! keymap. Only the parts that change the layer state are modelled; ordinary
//! keys are ignored. When the firmware reports its layer state directly
//! (see `docs/firmware-layer-report.md`) this replica is overwritten by the
//! reported value and only keeps the pressed-key bookkeeping.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::keycode::{action, Action, KC_NO, KC_TRNS};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TapDance {
    pub on_tap: u16,
    pub on_hold: u16,
    pub on_double_tap: u16,
    pub on_tap_hold: u16,
    pub term_ms: u16,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Keymap {
    pub layers: usize,
    pub rows: usize,
    pub cols: usize,
    /// Flattened `[layer][row][col]`.
    pub codes: Vec<u16>,
}

impl Keymap {
    pub fn get(&self, layer: usize, row: usize, col: usize) -> u16 {
        self.codes
            .get((layer * self.rows + row) * self.cols + col)
            .copied()
            .unwrap_or(KC_NO)
    }
}

/// Settings for layers that the firmware turns on by itself when a pointing
/// device moves (QMK "auto mouse" and similar).
#[derive(Debug, Clone, Copy)]
pub struct AutoMouse {
    pub layer: u8,
    pub rules: AutoMouseRules,
}

/// How the firmware enters and leaves the mouse layer.
#[derive(Debug, Clone, Copy)]
pub enum AutoMouseRules {
    /// Any movement turns the layer on; it stays until a key that is
    /// transparent or empty on the mouse layer is pressed (Svalboard).
    UntilUnmappedKey,
    /// QMK's built-in auto mouse layer (`POINTING_DEVICE_AUTO_MOUSE_ENABLE`).
    Qmk(QmkAutoMouse),
}

#[derive(Debug, Clone, Copy)]
pub struct QmkAutoMouse {
    /// Movement on one axis needed to turn the layer on (`AUTO_MOUSE_THRESHOLD`).
    /// 0 = any movement (QMK before the threshold was added, around 0.24).
    pub threshold: i32,
    /// The layer goes off this long after the last movement (`AUTO_MOUSE_TIME`).
    pub timeout: Duration,
    /// Movement is ignored this long after a non-mouse key (`AUTO_MOUSE_DELAY`).
    pub delay: Duration,
    /// Layers that disable the feature while one of them is the highest layer
    /// (some keyboards do this in `layer_state_set_kb`).
    pub blocked_layers: u32,
    /// Keycode ranges the keyboard adds as mouse keys (`is_mouse_record_kb`).
    pub mouse_keys: &'static [(u16, u16)],
}

/// `AUTO_MOUSE_DEBOUNCE`: movement right after an activation is not counted.
const AUTO_MOUSE_DEBOUNCE: Duration = Duration::from_millis(25);

/// How a key pressed while QMK auto mouse is enabled is treated.
#[derive(Debug, Clone, Copy)]
enum MouseKey {
    /// Mouse buttons and keys that hold the mouse layer: keep the layer on.
    Button,
    /// Mod-tap / layer-tap: an ordinary key when tapped, ignored when held.
    TapOrHold { since: Instant, held: bool },
    /// Any other key: leaves the mouse layer.
    Other,
}

/// Something a key press registered and must undo on release.
#[derive(Debug, Clone, Copy)]
enum Reg {
    LayerOn(u8),
    TapToggle(u8, Instant),
}

#[derive(Debug)]
enum Hold {
    Plain { regs: Vec<Reg>, ends_oneshot: Option<u8> },
    LayerTap { layer: u8, since: Instant, active: bool },
    OneShotKey,
    TapDance { index: u8 },
}

#[derive(Debug)]
struct DanceState {
    key: (usize, usize),
    count: u8,
    pressed: bool,
    interrupted: bool,
    last_press: Instant,
    finished: bool,
    regs: Vec<Reg>,
}

#[derive(Debug, Clone, Copy)]
struct OneShot {
    layer: u8,
    held: bool,
    used_while_held: bool,
    armed: bool,
}

pub struct Engine {
    keymap: Keymap,
    dances: Vec<TapDance>,
    tapping_term: Duration,
    pub layer_state: u32,
    pub default_layer: u32,
    holds: HashMap<(usize, usize), Hold>,
    pending_dances: HashMap<u8, DanceState>,
    oneshot: Option<OneShot>,
    auto_mouse: Option<AutoMouse>,
    auto_mouse_active: bool,
    last_move: Option<Instant>,
    /// QMK auto mouse: movement (x, y, wheel steps) summed since the last activation.
    mouse_total: (i32, i32, i32),
    /// QMK auto mouse: when the last non-mouse key went down or up.
    mouse_delay_from: Option<Instant>,
    mouse_keys: HashMap<(usize, usize), MouseKey>,
}

impl Engine {
    pub fn new(keymap: Keymap, dances: Vec<TapDance>, tapping_term_ms: u16) -> Self {
        Self {
            keymap,
            dances,
            tapping_term: Duration::from_millis(tapping_term_ms.max(1) as u64),
            layer_state: 0,
            default_layer: 1,
            holds: HashMap::new(),
            pending_dances: HashMap::new(),
            oneshot: None,
            auto_mouse: None,
            auto_mouse_active: false,
            last_move: None,
            mouse_total: (0, 0, 0),
            mouse_delay_from: None,
            mouse_keys: HashMap::new(),
        }
    }

    pub fn set_auto_mouse(&mut self, cfg: Option<AutoMouse>) {
        self.auto_mouse = cfg;
        if cfg.is_none() && self.auto_mouse_active {
            self.auto_mouse_off();
        }
    }

    pub fn auto_mouse_active(&self) -> bool {
        self.auto_mouse_active
    }

    /// Highest layer that is currently on (default layer included).
    pub fn top_layer(&self) -> u8 {
        let all = self.layer_state | self.default_layer;
        if all == 0 {
            0
        } else {
            (31 - all.leading_zeros()) as u8
        }
    }

    /// Forget every pending state and go back to the default layer.
    pub fn reset(&mut self) {
        self.layer_state = 0;
        self.holds.clear();
        self.pending_dances.clear();
        self.oneshot = None;
        self.auto_mouse_active = false;
        self.mouse_total = (0, 0, 0);
        self.mouse_delay_from = None;
        self.mouse_keys.clear();
    }

    /// Adopt the state reported by the firmware.
    pub fn sync_from_firmware(&mut self, layer_state: u32, default_layer: u32) {
        self.layer_state = layer_state;
        if default_layer != 0 {
            self.default_layer = default_layer;
        }
        self.pending_dances.clear();
        self.oneshot = None;
        if let Some(am) = self.auto_mouse {
            self.auto_mouse_active = layer_state & (1 << am.layer) != 0;
        }
    }

    fn on(&mut self, layer: u8) {
        if (layer as usize) < 32 {
            self.layer_state |= 1 << layer;
        }
    }

    fn off(&mut self, layer: u8) {
        if (layer as usize) < 32 {
            self.layer_state &= !(1 << layer);
        }
    }

    /// Keycode the firmware would use for a key right now.
    pub fn resolve(&self, row: usize, col: usize) -> u16 {
        let active = self.layer_state | self.default_layer;
        for layer in (0..self.keymap.layers.min(32)).rev() {
            if active & (1 << layer) == 0 {
                continue;
            }
            let kc = self.keymap.get(layer, row, col);
            if kc != KC_TRNS {
                return kc;
            }
        }
        KC_NO
    }

    fn apply_down(&mut self, kc: u16, now: Instant) -> Vec<Reg> {
        match action(kc) {
            Action::Momentary(n) | Action::LayerMod(n) | Action::LayerTap(n, _) => {
                self.on(n);
                vec![Reg::LayerOn(n)]
            }
            Action::To(n) => {
                self.layer_state = 1 << n;
                vec![]
            }
            Action::Default(n) => {
                self.default_layer = 1 << n;
                vec![]
            }
            Action::Toggle(n) => {
                self.layer_state ^= 1 << n;
                vec![]
            }
            Action::TapToggle(n) => {
                self.on(n);
                vec![Reg::TapToggle(n, now)]
            }
            Action::OneShot(n) => {
                self.on(n);
                self.oneshot = Some(OneShot { layer: n, held: false, used_while_held: false, armed: true });
                vec![]
            }
            Action::TapDance(_) | Action::Key => vec![],
        }
    }

    fn apply_up(&mut self, regs: &[Reg], now: Instant) {
        for reg in regs {
            match *reg {
                Reg::LayerOn(n) => self.off(n),
                Reg::TapToggle(n, since) => {
                    if now.duration_since(since) >= self.tapping_term {
                        self.off(n);
                    }
                }
            }
        }
    }

    fn tap(&mut self, kc: u16, now: Instant) {
        let regs = self.apply_down(kc, now);
        self.apply_up(&regs, now);
    }

    fn auto_mouse_off(&mut self) {
        if let Some(am) = self.auto_mouse {
            self.off(am.layer);
        }
        self.auto_mouse_active = false;
    }

    fn auto_mouse_on(&mut self, layer: u8) {
        self.on(layer);
        self.auto_mouse_active = true;
    }

    /// The pointing device of the keyboard moved by (`dx`, `dy`) counts and
    /// scrolled `dw` wheel steps.
    pub fn pointer_moved(&mut self, dx: i32, dy: i32, dw: i32, now: Instant) {
        let Some(am) = self.auto_mouse else { return };
        let q = match am.rules {
            AutoMouseRules::UntilUnmappedKey => {
                self.last_move = Some(now);
                if !self.auto_mouse_active {
                    self.auto_mouse_on(am.layer);
                }
                return;
            }
            AutoMouseRules::Qmk(q) => q,
        };
        if self.qmk_blocked(am.layer, q)
            || self.last_move.is_some_and(|t| now.duration_since(t) <= AUTO_MOUSE_DEBOUNCE)
            || self.mouse_delay_from.is_some_and(|t| now.duration_since(t) <= q.delay)
        {
            return;
        }
        let moved = if q.threshold == 0 {
            dx != 0 || dy != 0 || dw != 0
        } else {
            self.mouse_total.0 += dx;
            self.mouse_total.1 += dy;
            self.mouse_total.2 += dw;
            let (x, y, w) = self.mouse_total;
            x.abs() > q.threshold || y.abs() > q.threshold || w > q.threshold
        };
        if moved || self.mouse_buttons_held() {
            self.mouse_total = (0, 0, 0);
            self.last_move = Some(now);
            self.mouse_delay_from = None;
            if !self.auto_mouse_active {
                self.auto_mouse_on(am.layer);
            }
        }
    }

    /// The keyboard turns QMK auto mouse off while a blocked layer is on top.
    fn qmk_blocked(&self, mouse_layer: u8, q: QmkAutoMouse) -> bool {
        let others = self.layer_state & !(1u32 << mouse_layer);
        others != 0 && q.blocked_layers & (1 << (31 - others.leading_zeros())) != 0
    }

    fn mouse_buttons_held(&self) -> bool {
        self.mouse_keys.values().any(|k| matches!(k, MouseKey::Button))
    }

    /// A non-mouse key went down or up (QMK `auto_mouse_reset_trigger`).
    fn qmk_mouse_reset(&mut self, pressed: bool, now: Instant) {
        if self.mouse_buttons_held() {
            return;
        }
        if pressed && self.auto_mouse_active {
            self.auto_mouse_off();
            self.last_move = None;
        }
        self.mouse_delay_from = Some(now);
    }

    /// QMK `process_auto_mouse` for a key press. `kc` is what the key resolved to.
    fn qmk_mouse_key_down(&mut self, row: usize, col: usize, kc: u16, now: Instant) {
        let Some(AutoMouse { layer, rules: AutoMouseRules::Qmk(q) }) = self.auto_mouse else { return };
        // Another key decides pending mod-taps / layer-taps as held.
        for k in self.mouse_keys.values_mut() {
            if let MouseKey::TapOrHold { held, .. } = k {
                *held = true;
            }
        }
        if self.qmk_blocked(layer, q) {
            return;
        }
        let kind = match kc {
            _ if q.mouse_keys.iter().any(|&(lo, hi)| (lo..=hi).contains(&kc)) => Some(MouseKey::Button),
            // mouse keys (movement, buttons, wheel, acceleration)
            0x00CD..=0x00DF => Some(MouseKey::Button),
            // modifiers and modified keys are skipped
            0x00E0..=0x00E7 | 0x0100..=0x1FFF => None,
            // mod-tap
            0x2000..=0x3FFF => Some(MouseKey::TapOrHold { since: now, held: false }),
            // one-shot modifiers
            0x52A0..=0x52BF => None,
            _ => match action(kc) {
                Action::Momentary(n) | Action::LayerMod(n) | Action::TapToggle(n) => {
                    (n == layer).then_some(MouseKey::Button)
                }
                Action::LayerTap(n, _) if n == layer => Some(MouseKey::Button),
                Action::LayerTap(..) => Some(MouseKey::TapOrHold { since: now, held: false }),
                Action::To(_) | Action::Toggle(_) | Action::Default(_) | Action::OneShot(_) => None,
                Action::TapDance(_) | Action::Key => Some(MouseKey::Other),
            },
        };
        match kind {
            Some(MouseKey::Button) => self.mouse_delay_from = None,
            Some(MouseKey::Other) => self.qmk_mouse_reset(true, now),
            _ => {}
        }
        if let Some(kind) = kind {
            self.mouse_keys.insert((row, col), kind);
        }
    }

    fn qmk_mouse_key_up(&mut self, row: usize, col: usize, now: Instant) {
        match self.mouse_keys.remove(&(row, col)) {
            Some(MouseKey::Other) => self.qmk_mouse_reset(false, now),
            Some(MouseKey::TapOrHold { since, held }) => {
                if !held && now.duration_since(since) < self.tapping_term {
                    self.qmk_mouse_reset(true, now);
                    self.qmk_mouse_reset(false, now);
                }
            }
            _ => {}
        }
    }

    /// QMK auto mouse: timeout and blocked layers.
    fn qmk_mouse_tick(&mut self, now: Instant) {
        let Some(AutoMouse { layer, rules: AutoMouseRules::Qmk(q) }) = self.auto_mouse else { return };
        if !self.auto_mouse_active {
            return;
        }
        // A held mouse button keeps restarting the timeout.
        if self.mouse_buttons_held() {
            self.last_move = Some(now);
        }
        let timed_out =
            !self.mouse_buttons_held() && self.last_move.is_none_or(|t| now.duration_since(t) > q.timeout);
        if timed_out || self.qmk_blocked(layer, q) {
            self.auto_mouse_off();
            self.mouse_total = (0, 0, 0);
            self.last_move = None;
        }
    }

    pub fn key_down(&mut self, row: usize, col: usize, now: Instant) {
        // A key that does nothing on the mouse layer drops out of it.
        if self.auto_mouse_active {
            if let Some(am @ AutoMouse { rules: AutoMouseRules::UntilUnmappedKey, .. }) = self.auto_mouse {
                let on_mouse_layer = self.keymap.get(am.layer as usize, row, col);
                if on_mouse_layer == KC_TRNS || on_mouse_layer == KC_NO {
                    self.auto_mouse_off();
                    if self.resolve(row, col) == KC_NO {
                        return;
                    }
                }
            }
        }

        // Another key interrupts pending tap dances and layer-taps.
        let interrupted: Vec<u8> = self
            .pending_dances
            .iter()
            .filter(|(_, d)| d.key != (row, col) && !d.finished)
            .map(|(i, _)| *i)
            .collect();
        for index in interrupted {
            if let Some(d) = self.pending_dances.get_mut(&index) {
                d.interrupted = true;
            }
            self.finish_dance(index, now);
        }
        let waiting_lt: Vec<u8> = self
            .holds
            .values()
            .filter_map(|h| match h {
                Hold::LayerTap { layer, active: false, .. } => Some(*layer),
                _ => None,
            })
            .collect();
        for layer in waiting_lt {
            self.on(layer);
        }
        for h in self.holds.values_mut() {
            if let Hold::LayerTap { active, .. } = h {
                *active = true;
            }
        }

        let kc = self.resolve(row, col);
        self.qmk_mouse_key_down(row, col, kc, now);
        let hold = match action(kc) {
            Action::TapDance(index) => {
                let entry = self.pending_dances.entry(index).or_insert(DanceState {
                    key: (row, col),
                    count: 0,
                    pressed: false,
                    interrupted: false,
                    last_press: now,
                    finished: false,
                    regs: vec![],
                });
                entry.count = entry.count.saturating_add(1);
                entry.pressed = true;
                entry.last_press = now;
                Hold::TapDance { index }
            }
            Action::LayerTap(layer, _) => Hold::LayerTap { layer, since: now, active: false },
            Action::OneShot(layer) => {
                self.on(layer);
                self.oneshot = Some(OneShot { layer, held: true, used_while_held: false, armed: false });
                Hold::OneShotKey
            }
            _ => {
                let mut ends_oneshot = None;
                if let Some(os) = self.oneshot.as_mut() {
                    if os.held {
                        os.used_while_held = true;
                    } else if os.armed {
                        os.armed = false;
                        ends_oneshot = Some(os.layer);
                    }
                }
                let regs = self.apply_down(kc, now);
                Hold::Plain { regs, ends_oneshot }
            }
        };
        self.holds.insert((row, col), hold);
    }

    pub fn key_up(&mut self, row: usize, col: usize, now: Instant) {
        self.qmk_mouse_key_up(row, col, now);
        let Some(hold) = self.holds.remove(&(row, col)) else { return };
        match hold {
            Hold::Plain { regs, ends_oneshot } => {
                self.apply_up(&regs, now);
                if let Some(layer) = ends_oneshot {
                    self.off(layer);
                    self.oneshot = None;
                }
            }
            Hold::LayerTap { layer, active, .. } => {
                if active {
                    self.off(layer);
                }
            }
            Hold::OneShotKey => {
                if let Some(os) = self.oneshot.as_mut() {
                    if os.used_while_held {
                        let layer = os.layer;
                        self.oneshot = None;
                        self.off(layer);
                    } else {
                        os.held = false;
                        os.armed = true;
                    }
                }
            }
            Hold::TapDance { index } => {
                let mut release = None;
                if let Some(d) = self.pending_dances.get_mut(&index) {
                    d.pressed = false;
                    if d.finished {
                        release = Some(std::mem::take(&mut d.regs));
                    }
                }
                if let Some(regs) = release {
                    self.pending_dances.remove(&index);
                    self.apply_up(&regs, now);
                }
            }
        }
    }

    /// Advance timers. Call this every polling cycle.
    pub fn tick(&mut self, now: Instant) {
        let due: Vec<u8> = self
            .pending_dances
            .iter()
            .filter(|(i, d)| {
                let term = self.dance_term(**i);
                !d.finished && now.duration_since(d.last_press) >= term
            })
            .map(|(i, _)| *i)
            .collect();
        for index in due {
            self.finish_dance(index, now);
        }

        let term = self.tapping_term;
        let mut activate = vec![];
        for h in self.holds.values_mut() {
            if let Hold::LayerTap { layer, since, active } = h {
                if !*active && now.duration_since(*since) >= term {
                    *active = true;
                    activate.push(*layer);
                }
            }
        }
        for layer in activate {
            self.on(layer);
        }

        self.qmk_mouse_tick(now);
    }

    fn dance_term(&self, index: u8) -> Duration {
        match self.dances.get(index as usize) {
            Some(td) if td.term_ms > 0 => Duration::from_millis(td.term_ms as u64),
            _ => self.tapping_term,
        }
    }

    /// Decide what a tap dance resolved to (same rules as Vial).
    fn finish_dance(&mut self, index: u8, now: Instant) {
        let td = self.dances.get(index as usize).copied().unwrap_or_default();
        let (count, pressed, interrupted) = match self.pending_dances.get(&index) {
            Some(d) if !d.finished => (d.count, d.pressed, d.interrupted),
            _ => return,
        };
        let mut regs = vec![];
        let fallback = |kc: u16, alt: u16| if kc != KC_NO { kc } else { alt };
        match count {
            1 => {
                let kc = if interrupted || !pressed { td.on_tap } else { fallback(td.on_hold, td.on_tap) };
                regs.extend(self.apply_down(kc, now));
            }
            2 => {
                if interrupted {
                    self.tap(td.on_tap, now);
                    regs.extend(self.apply_down(td.on_tap, now));
                } else {
                    let kc = if pressed { td.on_tap_hold } else { td.on_double_tap };
                    if kc != KC_NO {
                        regs.extend(self.apply_down(kc, now));
                    } else {
                        self.tap(td.on_tap, now);
                        regs.extend(self.apply_down(td.on_tap, now));
                    }
                }
            }
            n => {
                for _ in 0..n {
                    self.tap(td.on_tap, now);
                }
            }
        }
        if pressed {
            if let Some(d) = self.pending_dances.get_mut(&index) {
                d.finished = true;
                d.regs = regs;
            }
        } else {
            self.pending_dances.remove(&index);
            self.apply_up(&regs, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TO: u16 = 0x5200;
    const MO: u16 = 0x5220;
    const OSL: u16 = 0x5280;
    const TD: u16 = 0x5700;

    // 1 row x 4 cols, 3 layers:
    // col0 = TD(0) [tap TO(0), hold MO(2), double TO(2)]
    // col1 = TD(1) [tap TO(1), hold MO(1)]
    // col2 = OSL(2) on layer 0
    // col3 = 'A' on layer 0, transparent elsewhere
    fn engine() -> Engine {
        let codes = vec![
            TD, TD + 1, OSL + 2, 0x04, // layer 0
            KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, // layer 1
            KC_TRNS, KC_TRNS, KC_TRNS, KC_TRNS, // layer 2
        ];
        let keymap = Keymap { layers: 3, rows: 1, cols: 4, codes };
        let dances = vec![
            TapDance { on_tap: TO, on_hold: MO + 2, on_double_tap: TO + 2, on_tap_hold: 0, term_ms: 300 },
            TapDance { on_tap: TO + 1, on_hold: MO + 1, on_double_tap: 0, on_tap_hold: 0, term_ms: 300 },
        ];
        Engine::new(keymap, dances, 200)
    }

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn single_tap_locks_layer() {
        let mut e = engine();
        let t = Instant::now();
        e.key_down(0, 1, t);
        e.key_up(0, 1, ms(t, 50));
        e.tick(ms(t, 400));
        assert_eq!(e.top_layer(), 1);
        // tapping the other dance goes back to layer 0
        e.key_down(0, 0, ms(t, 500));
        e.key_up(0, 0, ms(t, 550));
        e.tick(ms(t, 900));
        assert_eq!(e.top_layer(), 0);
    }

    #[test]
    fn hold_is_momentary() {
        let mut e = engine();
        let t = Instant::now();
        e.key_down(0, 0, t);
        e.tick(ms(t, 350));
        assert_eq!(e.top_layer(), 2);
        e.key_up(0, 0, ms(t, 600));
        assert_eq!(e.top_layer(), 0);
    }

    #[test]
    fn double_tap_locks_second_action() {
        let mut e = engine();
        let t = Instant::now();
        e.key_down(0, 0, t);
        e.key_up(0, 0, ms(t, 40));
        e.key_down(0, 0, ms(t, 120));
        e.key_up(0, 0, ms(t, 160));
        e.tick(ms(t, 500));
        assert_eq!(e.top_layer(), 2);
    }

    #[test]
    fn one_shot_layer_lasts_one_key() {
        let mut e = engine();
        let t = Instant::now();
        e.key_down(0, 2, t);
        e.key_up(0, 2, ms(t, 30));
        assert_eq!(e.top_layer(), 2);
        e.key_down(0, 3, ms(t, 100));
        e.key_up(0, 3, ms(t, 130));
        assert_eq!(e.top_layer(), 0);
    }

    #[test]
    fn auto_mouse_exits_on_unmapped_key() {
        let mut e = engine();
        e.set_auto_mouse(Some(AutoMouse { layer: 2, rules: AutoMouseRules::UntilUnmappedKey }));
        let t = Instant::now();
        e.pointer_moved(1, 0, 0, t);
        assert_eq!(e.top_layer(), 2);
        e.key_down(0, 3, ms(t, 50));
        assert_eq!(e.top_layer(), 0);
    }

    // 1 row x 3 cols, 3 layers, QMK auto mouse on layer 2 (blocked by layer 1):
    // col0 = MO(1), col1 = 'A' on layer 0 / mouse button 1 on layer 2, col2 = 'B'
    fn qmk_engine() -> Engine {
        let codes = vec![
            MO + 1, 0x04, 0x05, // layer 0
            KC_TRNS, KC_TRNS, KC_TRNS, // layer 1
            KC_TRNS, 0x00D1, KC_TRNS, // layer 2
        ];
        let keymap = Keymap { layers: 3, rows: 1, cols: 3, codes };
        let mut e = Engine::new(keymap, vec![], 200);
        e.set_auto_mouse(Some(AutoMouse {
            layer: 2,
            rules: AutoMouseRules::Qmk(QmkAutoMouse {
                threshold: 100,
                timeout: Duration::from_millis(1000),
                delay: Duration::from_millis(200),
                blocked_layers: 0b10,
                mouse_keys: &[],
            }),
        }));
        e
    }

    #[test]
    fn qmk_auto_mouse_needs_threshold_and_times_out() {
        let mut e = qmk_engine();
        let t = Instant::now();
        e.pointer_moved(60, 0, 0, t);
        assert_eq!(e.top_layer(), 0);
        e.pointer_moved(50, 0, 0, ms(t, 10));
        assert_eq!(e.top_layer(), 2);
        e.tick(ms(t, 900));
        assert_eq!(e.top_layer(), 2);
        e.tick(ms(t, 1100));
        assert_eq!(e.top_layer(), 0);
    }

    #[test]
    fn qmk_auto_mouse_button_keeps_layer_other_key_leaves() {
        let mut e = qmk_engine();
        let t = Instant::now();
        e.pointer_moved(0, -150, 0, t);
        e.key_down(0, 1, ms(t, 100)); // mouse button
        e.tick(ms(t, 1500));
        assert_eq!(e.top_layer(), 2);
        e.key_up(0, 1, ms(t, 1600));
        e.tick(ms(t, 1610));
        assert_eq!(e.top_layer(), 2);
        e.tick(ms(t, 2600));
        assert_eq!(e.top_layer(), 0);

        e.pointer_moved(150, 0, 0, ms(t, 3000));
        assert_eq!(e.top_layer(), 2);
        e.key_down(0, 2, ms(t, 3100)); // 'B'
        assert_eq!(e.top_layer(), 0);
        e.key_up(0, 2, ms(t, 3150));
        // movement right after typing is ignored
        e.pointer_moved(150, 0, 0, ms(t, 3250));
        assert_eq!(e.top_layer(), 0);
        e.pointer_moved(150, 0, 0, ms(t, 3400));
        assert_eq!(e.top_layer(), 2);
    }

    #[test]
    fn qmk_auto_mouse_without_threshold() {
        let mut e = qmk_engine();
        if let Some(AutoMouse { rules: AutoMouseRules::Qmk(q), .. }) = e.auto_mouse.as_mut() {
            q.threshold = 0;
        }
        let t = Instant::now();
        e.pointer_moved(0, 0, 1, t); // one wheel step
        assert_eq!(e.top_layer(), 2);
        e.tick(ms(t, 700));
        assert_eq!(e.top_layer(), 2);
        e.tick(ms(t, 1100));
        assert_eq!(e.top_layer(), 0);
        e.pointer_moved(1, 0, 0, ms(t, 1200));
        assert_eq!(e.top_layer(), 2);
    }

    #[test]
    fn qmk_auto_mouse_blocked_by_layer() {
        let mut e = qmk_engine();
        let t = Instant::now();
        e.key_down(0, 0, t); // MO(1)
        e.pointer_moved(150, 0, 0, ms(t, 300));
        assert_eq!(e.top_layer(), 1);
        e.key_up(0, 0, ms(t, 400));
        e.pointer_moved(150, 0, 0, ms(t, 500));
        assert_eq!(e.top_layer(), 2);
        e.key_down(0, 0, ms(t, 600));
        e.tick(ms(t, 610));
        assert_eq!(e.top_layer(), 1);
    }
}
