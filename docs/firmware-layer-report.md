# Reporting the layer state from the firmware

Out of the box CoCoKey follows the active layer by watching the switch matrix
and replaying the layer keys it finds in your keymap (`MO`, `TO`, `TG`, `OSL`,
`TT`, `LT`, `DF`, tap dances). That works with any unmodified Vial keyboard,
but it is a replay: timing-sensitive features (tap-hold decisions, combos,
key overrides, custom code) can make it drift.

If you can build your own firmware, add the small extension below. The
keyboard then reports every layer change itself and the HUD shows
"reported by keyboard" in its status line.

## 1. Add the header

Copy [`firmware/layer_report.h`](../firmware/layer_report.h) next to your
`keymap.c` and include it:

```c
#include "layer_report.h"
```

## 2. Answer the subscribe request

VIA and Vial pass unknown raw HID commands to `raw_hid_receive_kb()`.

**If your keyboard does not define `raw_hid_receive_kb()` yet**, add it to
`keymap.c`:

```c
void raw_hid_receive_kb(uint8_t *data, uint8_t length) {
    if (layer_report_handle(data, length)) return;
    data[0] = 0xFF; // id_unhandled
}
```

**If it already defines one** (for example the Svalboard, in
`keyboards/svalboard/svalboard.c`), add the call at the very top of the
existing function instead:

```c
void raw_hid_receive_kb(uint8_t *data, uint8_t length) {
    if (layer_report_handle(data, length)) return;
    // ... existing code ...
}
```

## 3. Report changes

```c
layer_state_t layer_state_set_user(layer_state_t state) {
    layer_report_send(state, default_layer_state);
    return state;
}

layer_state_t default_layer_state_set_user(layer_state_t state) {
    layer_report_send(layer_state, state);
    return state;
}
```

If these functions already exist in your keymap, add the
`layer_report_send(...)` line to them.

## Protocol

32-byte reports on the VIA raw HID interface (usage page `0xFF60`):

| Direction | Bytes | Meaning |
|---|---|---|
| host → keyboard | `7A 4C 48 00` | subscribe |
| keyboard → host | `7A 4C 48 01 ver state[4] default[4]` | reply with the current state |
| keyboard → host | `7A 4C 48 02 ver state[4] default[4]` | the layer state changed |

`state` and `default` are little-endian 32-bit layer bitmasks. Reports are
only sent after a host has subscribed, so the keyboard never pushes data
nobody is reading.
