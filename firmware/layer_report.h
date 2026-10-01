// Copyright 2026 Pifto Works
// SPDX-License-Identifier: GPL-2.0-or-later
//
// Layer report for CoCoKey
// ---------------------------------------------------------------------------
// Lets the keyboard tell the host which layers are active, so the HUD does not
// have to work it out from key presses. Works with VIA and Vial firmware.
//
// Usage (see docs/firmware-layer-report.md for details):
//   1. Copy this file next to your keymap.c and `#include "layer_report.h"`.
//   2. Call layer_report_handle() at the top of raw_hid_receive_kb().
//   3. Call layer_report_send() from layer_state_set_user() and
//      default_layer_state_set_user().
//
// Protocol (32-byte raw HID reports):
//   host  -> kb : 7A 'L' 'H' 00                     subscribe
//   kb    -> host: 7A 'L' 'H' 01 ver state[4] def[4]  reply to subscribe
//   kb    -> host: 7A 'L' 'H' 02 ver state[4] def[4]  layer changed
// state / def are little-endian 32-bit layer bitmasks.

#pragma once

#include <stdbool.h>
#include <stdint.h>
#include <string.h>

#include "quantum.h"
#include "raw_hid.h"

#define LAYER_REPORT_ID 0x7A
#define LAYER_REPORT_VERSION 1

#ifndef RAW_EPSIZE
#    define RAW_EPSIZE 32
#endif

static bool layer_report_subscribed = false;

static inline void layer_report_fill(uint8_t *buf, uint8_t kind, layer_state_t state, layer_state_t def) {
    uint32_t s = (uint32_t)state;
    uint32_t d = (uint32_t)def;
    buf[0] = LAYER_REPORT_ID;
    buf[1] = 'L';
    buf[2] = 'H';
    buf[3] = kind;
    buf[4] = LAYER_REPORT_VERSION;
    for (uint8_t i = 0; i < 4; i++) {
        buf[5 + i] = (s >> (8 * i)) & 0xFF;
        buf[9 + i] = (d >> (8 * i)) & 0xFF;
    }
}

// Call first thing in raw_hid_receive_kb(). Returns true if the report was
// for CoCoKey; the reply is written into `data` and sent back by VIA/Vial.
static inline bool layer_report_handle(uint8_t *data, uint8_t length) {
    if (length < 13 || data[0] != LAYER_REPORT_ID || data[1] != 'L' || data[2] != 'H' || data[3] != 0x00) {
        return false;
    }
    layer_report_subscribed = true;
    memset(data, 0, length);
    layer_report_fill(data, 0x01, layer_state, default_layer_state);
    return true;
}

// Call whenever the layer state changes.
static inline void layer_report_send(layer_state_t state, layer_state_t def) {
    if (!layer_report_subscribed) {
        return;
    }
    uint8_t buf[RAW_EPSIZE];
    memset(buf, 0, sizeof(buf));
    layer_report_fill(buf, 0x02, state, def);
    raw_hid_send(buf, sizeof(buf));
}
