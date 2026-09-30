// Layouts for ordinary keyboards that cannot report their keymap.
// Keys are identified by their PS/2 set-1 scan code (0xE0 prefix in the high
// byte), which is what Windows raw input reports.

import { label, type Category, type Context } from "./keycodes";
import type { Layout, PhysKey } from "./layout";

export type StandardKind = "us" | "jis";

interface StdKey {
  sc: number;
  /** HID usage, used for the label and colour. */
  usage: number;
  w?: number;
  h?: number;
  /** Space before the key, in key widths. */
  gap?: number;
  text?: string;
  shifted?: string;
}

const k = (sc: number, usage: number, w?: number, gap?: number, text?: string, shifted?: string): StdKey =>
  ({ sc, usage, w, gap, text, shifted });

const F_ROW: StdKey[] = [
  k(0x01, 0x29),
  k(0x3b, 0x3a, 1, 1), k(0x3c, 0x3b), k(0x3d, 0x3c), k(0x3e, 0x3d),
  k(0x3f, 0x3e, 1, 0.5), k(0x40, 0x3f), k(0x41, 0x40), k(0x42, 0x41),
  k(0x43, 0x42, 1, 0.5), k(0x44, 0x43), k(0x57, 0x44), k(0x58, 0x45),
  k(0xe037, 0x46, 1, 0.25), k(0x46, 0x47), k(0xe11d, 0x48),
];
const NAV_1 = [k(0xe052, 0x49, 1, 0.25), k(0xe047, 0x4a), k(0xe049, 0x4b)];
const NAV_2 = [k(0xe053, 0x4c, 1, 0.25), k(0xe04f, 0x4d), k(0xe051, 0x4e)];
const DIGITS = [0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b].map((sc, i) => k(sc, 0x1e + i));
const QWERTY = [0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13].map((u, i) => k(0x10 + i, u));
const ASDF = [0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f].map((u, i) => k(0x1e + i, u));
const ZXCV = [0x1d, 0x1b, 0x06, 0x19, 0x05, 0x11, 0x10].map((u, i) => k(0x2c + i, u));
const ARROWS = [k(0xe04b, 0x50, 1, 0.25), k(0xe050, 0x51), k(0xe04d, 0x4f)];

const US: StdKey[][] = [
  F_ROW,
  [k(0x29, 0x35), ...DIGITS, k(0x0c, 0x2d), k(0x0d, 0x2e), k(0x0e, 0x2a, 2), ...NAV_1],
  [k(0x0f, 0x2b, 1.5), ...QWERTY, k(0x1a, 0x2f), k(0x1b, 0x30), k(0x2b, 0x31, 1.5), ...NAV_2],
  [k(0x3a, 0x39, 1.75), ...ASDF, k(0x27, 0x33), k(0x28, 0x34), k(0x1c, 0x28, 2.25)],
  [k(0x2a, 0xe1, 2.25), ...ZXCV, k(0x33, 0x36), k(0x34, 0x37), k(0x35, 0x38), k(0x36, 0xe5, 2.75), k(0xe048, 0x52, 1, 1.25)],
  [
    k(0x1d, 0xe0, 1.25), k(0xe05b, 0xe3, 1.25), k(0x38, 0xe2, 1.25), k(0x39, 0x2c, 6.25),
    k(0xe038, 0xe6, 1.25), k(0xe05c, 0xe7, 1.25), k(0xe05d, 0x65, 1.25), k(0xe01d, 0xe4, 1.25), ...ARROWS,
  ],
];

const JIS_DIGIT_SHIFT = ["!", '"', "#", "$", "%", "&", "'", "(", ")", ""];
const JIS: StdKey[][] = [
  F_ROW,
  [
    k(0x29, 0x35, 1, 0, "半/全", ""), ...DIGITS.map((d, i) => ({ ...d, shifted: JIS_DIGIT_SHIFT[i] })),
    k(0x0c, 0x2d, 1, 0, "-", "="), k(0x0d, 0x2e, 1, 0, "^", "~"), k(0x7d, 0x89, 1, 0, "¥", "|"), k(0x0e, 0x2a), ...NAV_1,
  ],
  [
    k(0x0f, 0x2b, 1.5), ...QWERTY, k(0x1a, 0x2f, 1, 0, "@", "`"), k(0x1b, 0x30, 1, 0, "[", "{"),
    { ...k(0x1c, 0x28, 1.25, 0.25), h: 2 }, ...NAV_2,
  ],
  [
    k(0x3a, 0x39, 1.75, 0, "英数", "Caps"), ...ASDF, k(0x27, 0x33, 1, 0, ";", "+"), k(0x28, 0x34, 1, 0, ":", "*"),
    k(0x2b, 0x31, 1, 0, "]", "}"),
  ],
  [
    k(0x2a, 0xe1, 2.25), ...ZXCV, k(0x33, 0x36, 1, 0, ",", "<"), k(0x34, 0x37, 1, 0, ".", ">"),
    k(0x35, 0x38, 1, 0, "/", "?"), k(0x73, 0x87, 1, 0, "\\", "_"), k(0x36, 0xe5, 1.75), k(0xe048, 0x52, 1, 1.25),
  ],
  [
    k(0x1d, 0xe0, 1.25), k(0xe05b, 0xe3, 1.25), k(0x38, 0xe2, 1.25), k(0x7b, 0x8b, 1.25), k(0x39, 0x2c, 3.75),
    k(0x79, 0x8a, 1.25), k(0x70, 0x88, 1.25), k(0xe038, 0xe6, 1.25), k(0xe05d, 0x65, 1.25), k(0xe01d, 0xe4, 1.25),
    ...ARROWS,
  ],
];

const US_SHIFT: Record<number, string> = {
  0x1e: "!", 0x1f: "@", 0x20: "#", 0x21: "$", 0x22: "%", 0x23: "^", 0x24: "&", 0x25: "*", 0x26: "(", 0x27: ")",
  0x2d: "_", 0x2e: "+", 0x2f: "{", 0x30: "}", 0x31: "|", 0x33: ":", 0x34: '"', 0x35: "~", 0x36: "<", 0x37: ">", 0x38: "?",
};

export interface StandardKey extends PhysKey {
  usage: number;
  text?: string;
  shifted?: string;
}

export interface StandardLayout extends Layout {
  keys: StandardKey[];
}

export function standardLayout(kind: StandardKind): StandardLayout {
  const rows = kind === "jis" ? JIS : US;
  const keys: StandardKey[] = [];
  let y = 0;
  rows.forEach((row, i) => {
    let x = 0;
    for (const key of row) {
      x += key.gap ?? 0;
      const w = key.w ?? 1;
      keys.push({
        row: -1, col: key.sc, x: x + 0.04, y: y + 0.04, w, h: key.h ?? 1,
        name: key.text ?? "", usage: key.usage, text: key.text, shifted: key.shifted,
      });
      x += w;
    }
    y += i === 0 ? 1.25 : 1;
  });
  const width = Math.max(...keys.map((k) => k.x + k.w)) + 0.04;
  const height = Math.max(...keys.map((k) => k.y + k.h)) + 0.04;
  return { keys, captions: [], width, height };
}

/** Label of a standard key, taking Shift into account. */
export function standardLabel(key: StandardKey, shift: boolean, ctx: Context): { text: string; cat: Category } {
  const base = label(key.usage, { ...ctx, shift });
  if (shift && key.shifted !== undefined) return { text: key.shifted, cat: key.shifted ? "sym" : "none" };
  if (shift && US_SHIFT[key.usage]) return { text: US_SHIFT[key.usage], cat: "sym" };
  if (key.text !== undefined) return { text: key.text, cat: base.cat };
  return base;
}
