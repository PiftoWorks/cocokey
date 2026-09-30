//! VIA / Vial raw HID protocol (32-byte reports on usage page 0xFF60).

use std::io::Read;
use std::time::Duration;

use hidapi::{HidApi, HidDevice};
use serde::Serialize;

use crate::engine::{Keymap, TapDance};

pub const USAGE_PAGE: u16 = 0xFF60;
pub const USAGE: u16 = 0x61;
const REPORT: usize = 32;
const TIMEOUT: Duration = Duration::from_millis(500);

// VIA command ids
const VIA_GET_PROTOCOL_VERSION: u8 = 0x01;
const VIA_GET_KEYBOARD_VALUE: u8 = 0x02;
const VIA_SWITCH_MATRIX_STATE: u8 = 0x03;
const VIA_GET_LAYER_COUNT: u8 = 0x11;
const VIA_GET_KEYMAP_BUFFER: u8 = 0x12;
const VIA_GET_ENCODER: u8 = 0x14;
const VIA_MACRO_GET_COUNT: u8 = 0x0C;
const VIA_MACRO_GET_BUFFER_SIZE: u8 = 0x0D;
const VIA_MACRO_GET_BUFFER: u8 = 0x0E;

// Vial command ids (prefixed with 0xFE)
const VIAL_PREFIX: u8 = 0xFE;
const VIAL_GET_KEYBOARD_ID: u8 = 0x00;
const VIAL_GET_SIZE: u8 = 0x01;
const VIAL_GET_DEFINITION: u8 = 0x02;
const VIAL_GET_ENCODER: u8 = 0x03;
const VIAL_GET_UNLOCK_STATUS: u8 = 0x05;
const VIAL_QMK_SETTINGS_GET: u8 = 0x0A;
const VIAL_DYNAMIC_ENTRY_OP: u8 = 0x0D;
const DYNAMIC_GET_COUNTS: u8 = 0x00;
const DYNAMIC_TAP_DANCE_GET: u8 = 0x01;
const QMK_SETTING_TAPPING_TERM: u16 = 7;

// Optional firmware extension that reports the layer state
// (see docs/firmware-layer-report.md).
pub const LAYER_REPORT_ID: u8 = 0x7A;
const LAYER_REPORT_MAGIC: [u8; 2] = [b'L', b'H'];
const LAYER_REPORT_SUBSCRIBE: u8 = 0x00;
const LAYER_REPORT_ACK: u8 = 0x01;
const LAYER_REPORT_EVENT: u8 = 0x02;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub path: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub manufacturer: String,
}

/// Layer state pushed by a firmware that implements the extension.
#[derive(Debug, Clone, Copy)]
pub struct LayerReport {
    pub layer_state: u32,
    pub default_layer: u32,
}

pub fn list_devices(api: &HidApi) -> Vec<DeviceInfo> {
    let mut out: Vec<DeviceInfo> = api
        .device_list()
        .filter(|d| d.usage_page() == USAGE_PAGE && d.usage() == USAGE)
        .map(|d| DeviceInfo {
            path: d.path().to_string_lossy().into_owned(),
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            product: d.product_string().unwrap_or_default().to_string(),
            manufacturer: d.manufacturer_string().unwrap_or_default().to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

pub struct RawHid {
    dev: HidDevice,
    reports: Vec<LayerReport>,
    layer_report_enabled: bool,
    /// VIA protocol 12+ (QMK 0.19+) answers matrix queries with a row offset
    /// byte before the data; Vial and older VIA do not.
    paged_matrix: bool,
}

impl RawHid {
    pub fn open(api: &HidApi, path: &str) -> Result<Self, String> {
        let cpath = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
        let dev = api.open_path(&cpath).map_err(|e| e.to_string())?;
        let mut me = Self { dev, reports: vec![], layer_report_enabled: false, paged_matrix: false };
        me.drain();
        Ok(me)
    }

    /// Discard reports left over from earlier traffic.
    fn drain(&mut self) {
        let mut buf = [0u8; REPORT];
        while let Ok(n) = self.dev.read_timeout(&mut buf, 0) {
            if n == 0 {
                break;
            }
        }
    }

    fn parse_layer_report(&mut self, buf: &[u8; REPORT]) -> bool {
        if !self.layer_report_enabled
            || buf[0] != LAYER_REPORT_ID
            || buf[1..3] != LAYER_REPORT_MAGIC
            || buf[3] != LAYER_REPORT_EVENT
        {
            return false;
        }
        self.reports.push(LayerReport {
            layer_state: u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]),
            default_layer: u32::from_le_bytes([buf[9], buf[10], buf[11], buf[12]]),
        });
        true
    }

    /// Send one request and return the reply.
    ///
    /// Every program that has the interface open receives every reply, so
    /// replies meant for another program (e.g. Vial) may arrive first. VIA
    /// replies echo the command, which lets us skip foreign ones; Vial (0xFE)
    /// replies do not, so those are only checked for timing.
    pub fn xfer(&mut self, req: &[u8]) -> Result<[u8; REPORT], String> {
        let echo = req.len() >= 1 && req[0] != VIAL_PREFIX;
        self.xfer_matching(req, |r| !echo || r[0] == req[0])
    }

    fn xfer_matching(&mut self, req: &[u8], accept: impl Fn(&[u8; REPORT]) -> bool) -> Result<[u8; REPORT], String> {
        // Throw away anything left over before asking.
        let mut buf = [0u8; REPORT];
        while let Ok(n) = self.dev.read_timeout(&mut buf, 0) {
            if n == 0 {
                break;
            }
            self.parse_layer_report(&buf);
        }
        let mut out = [0u8; REPORT + 1];
        out[1..1 + req.len().min(REPORT)].copy_from_slice(&req[..req.len().min(REPORT)]);
        self.dev.write(&out).map_err(|e| e.to_string())?;
        for _ in 0..16 {
            let n = self
                .dev
                .read_timeout(&mut buf, TIMEOUT.as_millis() as i32)
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("timeout".into());
            }
            if self.parse_layer_report(&buf) || !accept(&buf) {
                continue;
            }
            return Ok(buf);
        }
        Err("no matching reply".into())
    }

    /// Collect layer reports that arrived without a request.
    pub fn take_reports(&mut self) -> Vec<LayerReport> {
        if self.layer_report_enabled {
            let mut buf = [0u8; REPORT];
            while let Ok(n) = self.dev.read_timeout(&mut buf, 0) {
                if n == 0 {
                    break;
                }
                self.parse_layer_report(&buf);
            }
        }
        std::mem::take(&mut self.reports)
    }

    pub fn via_protocol(&mut self) -> Result<u16, String> {
        let r = self.xfer(&[VIA_GET_PROTOCOL_VERSION])?;
        let v = u16::from_be_bytes([r[1], r[2]]);
        self.paged_matrix = v >= 12;
        Ok(v)
    }

    /// Tell the reader which matrix reply format the firmware uses.
    pub fn set_paged_matrix(&mut self, paged: bool) {
        self.paged_matrix = paged;
    }

    /// Whether the whole matrix can be read (the old format fits in one reply).
    pub fn matrix_readable(&self, rows: usize, cols: usize) -> bool {
        self.paged_matrix || rows * cols.div_ceil(8) <= 28
    }

    pub fn layer_count(&mut self) -> Result<u8, String> {
        let r = self.xfer(&[VIA_GET_LAYER_COUNT])?;
        Ok(r[1])
    }

    pub fn read_keymap(&mut self, layers: usize, rows: usize, cols: usize) -> Result<Keymap, String> {
        let total = layers * rows * cols * 2;
        let mut bytes = Vec::with_capacity(total);
        while bytes.len() < total {
            let offset = bytes.len() as u16;
            let size = (total - bytes.len()).min(28) as u8;
            let [hi, lo] = offset.to_be_bytes();
            let r = self.xfer_matching(&[VIA_GET_KEYMAP_BUFFER, hi, lo, size], |r| {
                r[0] == VIA_GET_KEYMAP_BUFFER && r[1] == hi && r[2] == lo && r[3] == size
            })?;
            bytes.extend_from_slice(&r[4..4 + size as usize]);
        }
        let codes = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Ok(Keymap { layers, rows, cols, codes })
    }

    /// Rotary encoder keycodes as `[layer][encoder] = [counter-clockwise, clockwise]`.
    pub fn encoder_keycodes(&mut self, vial: bool, layers: usize, ids: &[u8]) -> Vec<Vec<[u16; 2]>> {
        let mut out = vec![vec![[0u16; 2]; ids.iter().max().map_or(0, |m| *m as usize + 1)]; layers];
        for layer in 0..layers {
            for &id in ids {
                let pair = if vial {
                    match self.xfer(&[VIAL_PREFIX, VIAL_GET_ENCODER, layer as u8, id]) {
                        Ok(r) => [u16::from_be_bytes([r[0], r[1]]), u16::from_be_bytes([r[2], r[3]])],
                        Err(_) => return out,
                    }
                } else {
                    let mut pair = [0u16; 2];
                    for (dir, slot) in pair.iter_mut().enumerate() {
                        match self.xfer_matching(&[VIA_GET_ENCODER, layer as u8, id, dir as u8], |r| {
                            r[0] == VIA_GET_ENCODER && r[1] == layer as u8 && r[2] == id
                        }) {
                            Ok(r) => *slot = u16::from_be_bytes([r[4], r[5]]),
                            Err(_) => return out,
                        }
                    }
                    pair
                };
                out[layer][id as usize] = pair;
            }
        }
        out
    }

    /// Macro count and the raw macro buffer (macros separated by 0x00).
    pub fn read_macros(&mut self) -> Result<(u8, Vec<u8>), String> {
        let count = self.xfer(&[VIA_MACRO_GET_COUNT])?[1];
        let r = self.xfer(&[VIA_MACRO_GET_BUFFER_SIZE])?;
        let size = u16::from_be_bytes([r[1], r[2]]) as usize;
        let mut buf = Vec::with_capacity(size);
        while buf.len() < size {
            let offset = buf.len() as u16;
            let chunk = (size - buf.len()).min(28) as u8;
            let [hi, lo] = offset.to_be_bytes();
            let r = self.xfer_matching(&[VIA_MACRO_GET_BUFFER, hi, lo, chunk], |r| {
                r[0] == VIA_MACRO_GET_BUFFER && r[1] == hi && r[2] == lo
            })?;
            buf.extend_from_slice(&r[4..4 + chunk as usize]);
            // Everything after the last macro is empty; stop early.
            if buf.iter().filter(|&&b| b == 0).count() >= count as usize {
                break;
            }
        }
        Ok((count, buf))
    }

    /// Bitmap of pressed switches, one entry per row.
    pub fn matrix_state(&mut self, rows: usize, cols: usize) -> Result<Vec<u32>, String> {
        let per_row = cols.div_ceil(8).max(1);
        let mut out = Vec::with_capacity(rows);
        // Old format: rows start at byte 2, all in one reply.
        // Paged format: byte 2 echoes the first row, rows start at byte 3, and
        // at most 28 bytes of rows fit per reply.
        let (data_start, rows_per_reply) = if self.paged_matrix { (3, 28 / per_row) } else { (2, rows) };
        while out.len() < rows {
            let offset = out.len() as u8;
            let paged = self.paged_matrix;
            let r = self.xfer_matching(&[VIA_GET_KEYBOARD_VALUE, VIA_SWITCH_MATRIX_STATE, offset], |r| {
                r[0] == VIA_GET_KEYBOARD_VALUE && r[1] == VIA_SWITCH_MATRIX_STATE && (!paged || r[2] == offset)
            })?;
            for i in 0..rows_per_reply.min(rows - out.len()) {
                let start = data_start + i * per_row;
                let mut v = 0u32;
                for b in 0..per_row {
                    v = (v << 8) | *r.get(start + b).unwrap_or(&0) as u32;
                }
                out.push(v);
            }
        }
        Ok(out)
    }

    /// Vial protocol version and 64-bit keyboard id, or `None` for plain VIA.
    pub fn vial_id(&mut self) -> Option<(u32, [u8; 8])> {
        let r = self.xfer(&[VIAL_PREFIX, VIAL_GET_KEYBOARD_ID]).ok()?;
        let proto = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
        // Plain VIA answers unknown commands with 0xFF (id_unhandled) or an echo.
        if proto == 0 || proto > 0xFFFF || r[0] == 0xFF || r[0] == VIAL_PREFIX {
            return None;
        }
        let mut uid = [0u8; 8];
        uid.copy_from_slice(&r[4..12]);
        Some((proto, uid))
    }

    /// Keyboard definition (the JSON that Vial shows) stored in the firmware.
    pub fn vial_definition(&mut self) -> Result<serde_json::Value, String> {
        let r = self.xfer(&[VIAL_PREFIX, VIAL_GET_SIZE])?;
        let size = u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize;
        if size == 0 || size > 1 << 20 {
            return Err(format!("unexpected definition size {size}"));
        }
        let mut blob = Vec::with_capacity(size + REPORT);
        let mut page: u16 = 0;
        while blob.len() < size {
            let [lo, hi] = page.to_le_bytes();
            blob.extend_from_slice(&self.xfer(&[VIAL_PREFIX, VIAL_GET_DEFINITION, lo, hi])?);
            page += 1;
        }
        blob.truncate(size);
        let mut json = Vec::new();
        lzma_rs::xz_decompress(&mut std::io::BufReader::new(blob.as_slice()), &mut json)
            .or_else(|_| {
                json.clear();
                lzma_rs::lzma_decompress(&mut std::io::BufReader::new(blob.as_slice()), &mut json)
            })
            .map_err(|e| format!("definition: {e:?}"))?;
        let mut text = String::new();
        json.as_slice().read_to_string(&mut text).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    /// Vial only answers matrix queries while unlocked; a locked keyboard
    /// echoes the request, which looks like "nothing pressed".
    pub fn vial_unlocked(&mut self) -> bool {
        self.xfer(&[VIAL_PREFIX, VIAL_GET_UNLOCK_STATUS]).is_ok_and(|r| r[0] == 1)
    }

    pub fn tap_dances(&mut self) -> Vec<TapDance> {
        let Ok(r) = self.xfer(&[VIAL_PREFIX, VIAL_DYNAMIC_ENTRY_OP, DYNAMIC_GET_COUNTS]) else {
            return vec![];
        };
        let count = r[0] as usize;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let Ok(r) = self.xfer(&[VIAL_PREFIX, VIAL_DYNAMIC_ENTRY_OP, DYNAMIC_TAP_DANCE_GET, i as u8]) else {
                break;
            };
            if r[0] != 0 {
                break;
            }
            let w = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
            out.push(TapDance { on_tap: w(1), on_hold: w(3), on_double_tap: w(5), on_tap_hold: w(7), term_ms: w(9) });
        }
        out
    }

    pub fn tapping_term(&mut self) -> Option<u16> {
        let [lo, hi] = QMK_SETTING_TAPPING_TERM.to_le_bytes();
        let r = self.xfer(&[VIAL_PREFIX, VIAL_QMK_SETTINGS_GET, lo, hi]).ok()?;
        if r[0] != 0 {
            return None;
        }
        let v = u16::from_le_bytes([r[1], r[2]]);
        (v > 0 && v < 5000).then_some(v)
    }

    /// Ask the firmware to push layer changes. Returns the current state if
    /// the firmware supports it.
    pub fn subscribe_layer_reports(&mut self) -> Option<LayerReport> {
        let r = self
            .xfer(&[LAYER_REPORT_ID, LAYER_REPORT_MAGIC[0], LAYER_REPORT_MAGIC[1], LAYER_REPORT_SUBSCRIBE])
            .ok()?;
        if r[0] != LAYER_REPORT_ID || r[1..3] != LAYER_REPORT_MAGIC || r[3] != LAYER_REPORT_ACK {
            return None;
        }
        self.layer_report_enabled = true;
        Some(LayerReport {
            layer_state: u32::from_le_bytes([r[5], r[6], r[7], r[8]]),
            default_layer: u32::from_le_bytes([r[9], r[10], r[11], r[12]]),
        })
    }
}
