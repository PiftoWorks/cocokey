//! Decoding of the QMK keycodes that affect the layer state.
//!
//! The numbering follows the keycode layout used by QMK since 0.19
//! (Vial protocol 6 and later, VIA protocol 12 and later).

pub const KC_NO: u16 = 0x0000;
pub const KC_TRNS: u16 = 0x0001;

/// What a keycode does to the layer state when it is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Ordinary key (letters, modifiers, macros, ...). No layer effect.
    Key,
    /// `MO(n)`: layer on while held.
    Momentary(u8),
    /// `TO(n)`: switch to layer n only.
    To(u8),
    /// `DF(n)` / `PDF(n)`: change the default layer.
    Default(u8),
    /// `TG(n)`: toggle layer n.
    Toggle(u8),
    /// `OSL(n)`: layer n for the next key only.
    OneShot(u8),
    /// `TT(n)`: momentary when held, toggle when tapped.
    TapToggle(u8),
    /// `LT(n, kc)`: layer n when held, `kc` when tapped.
    LayerTap(u8, u16),
    /// `LM(n, mod)`: layer n with modifiers while held.
    LayerMod(u8),
    /// `TD(i)`: tap dance entry i.
    TapDance(u8),
}

pub fn action(kc: u16) -> Action {
    match kc {
        0x4000..=0x4FFF => Action::LayerTap(((kc >> 8) & 0x0F) as u8, kc & 0xFF),
        0x5000..=0x51FF => Action::LayerMod(((kc >> 5) & 0x0F) as u8),
        0x5200..=0x521F => Action::To((kc & 0x1F) as u8),
        0x5220..=0x523F => Action::Momentary((kc & 0x1F) as u8),
        0x5240..=0x525F => Action::Default((kc & 0x1F) as u8),
        0x5260..=0x527F => Action::Toggle((kc & 0x1F) as u8),
        0x5280..=0x529F => Action::OneShot((kc & 0x1F) as u8),
        0x52C0..=0x52DF => Action::TapToggle((kc & 0x1F) as u8),
        0x52E0..=0x52FF => Action::Default((kc & 0x1F) as u8),
        0x5700..=0x57FF => Action::TapDance((kc & 0xFF) as u8),
        _ => Action::Key,
    }
}

/// Translate a keycode from the numbering used before QMK 0.19 (VIA protocol
/// below 12, Vial protocol below 6) to the current one. Ranges that did not
/// move are returned unchanged.
pub fn from_legacy(kc: u16) -> u16 {
    match kc {
        // mouse keys moved from 0xF0.. to 0xCD..
        0x00F0..=0x00F3 => kc - 0xF0 + 0xCD,
        0x00F4..=0x00F8 => kc - 0xF4 + 0xD1,
        0x00F9..=0x00FC => kc - 0xF9 + 0xD9,
        0x00FD..=0x00FF => kc - 0xFD + 0xDD,
        0x5000..=0x50FF => 0x5200 | (kc & 0x0F),                             // TO (bit 4 = "on press")
        0x5100..=0x51FF => 0x5220 | (kc & 0x1F),                             // MO
        0x5200..=0x52FF => 0x5240 | (kc & 0x1F),                             // DF
        0x5300..=0x53FF => 0x5260 | (kc & 0x1F),                             // TG
        0x5400..=0x54FF => 0x5280 | (kc & 0x1F),                             // OSL
        0x5500..=0x55FF => 0x52A0 | (kc & 0x1F),                             // OSM
        0x5800..=0x58FF => 0x52C0 | (kc & 0x1F),                             // TT
        0x5900..=0x59FF => 0x5000 | (((kc >> 4) & 0x0F) << 5) | (kc & 0x0F), // LM
        0x5C00 => 0x7C00,                                                    // bootloader
        0x5F12..=0x5F21 => 0x7700 + (kc - 0x5F12),                           // VIA macros
        0x5F80..=0x5F8F => 0x7E00 + (kc - 0x5F80),                           // VIA user keycodes
        0x6000..=0x7FFF => 0x2000 | (kc & 0x1FFF),                           // mod-tap
        _ => kc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_legacy_keycodes() {
        assert_eq!(from_legacy(0x5011), 0x5201); // TO(1)
        assert_eq!(from_legacy(0x5102), 0x5222); // MO(2)
        assert_eq!(from_legacy(0x5403), 0x5283); // OSL(3)
        assert_eq!(from_legacy(0x00F4), 0x00D1); // mouse button 1
        assert_eq!(from_legacy(0x412C), 0x412C); // LT unchanged
        assert_eq!(from_legacy(0x0004), 0x0004);
        assert_eq!(from_legacy(0x5F12), 0x7700); // macro 0
    }

    #[test]
    fn decodes_layer_keycodes() {
        assert_eq!(action(0x5203), Action::To(3));
        assert_eq!(action(0x5221), Action::Momentary(1));
        assert_eq!(action(0x522F), Action::Momentary(15));
        assert_eq!(action(0x5284), Action::OneShot(4));
        assert_eq!(action(0x5701), Action::TapDance(1));
        assert_eq!(action(0x412C), Action::LayerTap(1, 0x2C));
        assert_eq!(action(0x0004), Action::Key);
        assert_eq!(action(0x021E), Action::Key);
    }
}
