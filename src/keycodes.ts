// Human-readable labels for QMK keycodes (numbering used since QMK 0.19,
// i.e. Vial protocol 6+ / VIA protocol 12+).

import { t } from "./i18n";

export type Category =
  | "alpha" | "num" | "sym" | "mod" | "edit" | "nav" | "fn" | "layer" | "dance"
  | "mouse" | "macro" | "media" | "system" | "trns" | "none";

export interface TapDance {
  onTap: number;
  onHold: number;
  onDoubleTap: number;
  onTapHold: number;
  termMs: number;
}

export interface CustomKeycode {
  name?: string;
  title?: string;
  shortName?: string;
}

export interface Context {
  tapDances: TapDance[];
  custom: CustomKeycode[];
  /** Show what the keys type while Shift is held. */
  shift?: boolean;
  /** One-line description of each macro, shown on macro keys. */
  macroText?: string[];
}

export interface Label {
  text: string;
  cat: Category;
}

export const KC_NO = 0x0000;
export const KC_TRNS = 0x0001;

const BASIC: Record<number, [string, Category]> = {};
const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
for (let i = 0; i < 26; i++) BASIC[0x04 + i] = [letters[i], "alpha"];
"1234567890".split("").forEach((d, i) => (BASIC[0x1e + i] = [d, "num"]));
Object.assign(BASIC, {
  0x28: ["Enter", "edit"], 0x29: ["Esc", "edit"], 0x2a: ["BS", "edit"], 0x2b: ["Tab", "edit"],
  0x2c: ["Space", "edit"], 0x2d: ["-", "sym"], 0x2e: ["=", "sym"], 0x2f: ["[", "sym"], 0x30: ["]", "sym"],
  0x31: ["\\", "sym"], 0x32: ["#", "sym"], 0x33: [";", "sym"], 0x34: ["'", "sym"], 0x35: ["`", "sym"],
  0x36: [",", "sym"], 0x37: [".", "sym"], 0x38: ["/", "sym"], 0x39: ["Caps", "mod"],
  0x46: ["PrtSc", "edit"], 0x47: ["ScrLk", "edit"], 0x48: ["Pause", "edit"], 0x49: ["Ins", "edit"],
  0x4a: ["Home", "nav"], 0x4b: ["PgUp", "nav"], 0x4c: ["Del", "edit"], 0x4d: ["End", "nav"],
  0x4e: ["PgDn", "nav"], 0x4f: ["→", "nav"], 0x50: ["←", "nav"], 0x51: ["↓", "nav"], 0x52: ["↑", "nav"],
  0x53: ["NumLk", "num"], 0x54: ["N/", "num"], 0x55: ["N*", "num"], 0x56: ["N-", "num"], 0x57: ["N+", "num"],
  0x58: ["NEnt", "num"], 0x62: ["N0", "num"], 0x63: ["N.", "num"], 0x64: ["\\|", "sym"],
  0x65: ["Menu", "edit"], 0x66: ["Power", "system"], 0x67: ["N=", "num"], 0x74: ["Exec", "edit"],
  0x75: ["Help", "edit"], 0x77: ["Select", "edit"], 0x78: ["Stop", "edit"], 0x79: ["Again", "edit"],
  0x7a: ["Undo", "edit"], 0x7b: ["Cut", "edit"], 0x7c: ["Copy", "edit"], 0x7d: ["Paste", "edit"],
  0x7e: ["Find", "edit"], 0x7f: ["Mute", "media"], 0x80: ["Vol+", "media"], 0x81: ["Vol-", "media"],
  0x85: ["N,", "num"], 0x87: ["ろ", "sym"], 0x88: ["かな", "edit"], 0x89: ["¥", "sym"],
  0x8a: ["変換", "edit"], 0x8b: ["無変換", "edit"], 0x90: ["かな", "edit"], 0x91: ["英数", "edit"],
  0xa5: ["Power", "system"], 0xa6: ["Sleep", "system"], 0xa7: ["Wake", "system"],
  0xa8: ["Mute", "media"], 0xa9: ["Vol+", "media"], 0xaa: ["Vol-", "media"], 0xab: ["⏭", "media"],
  0xac: ["⏮", "media"], 0xad: ["⏹", "media"], 0xae: ["⏯", "media"], 0xaf: ["Select", "media"],
  0xb0: ["Eject", "media"], 0xb1: ["Mail", "media"], 0xb2: ["Calc", "media"], 0xb3: ["PC", "media"],
  0xb4: ["Search", "media"], 0xb5: ["WWW", "media"], 0xb6: ["back", "nav"], 0xb7: ["forward", "nav"],
  0xb8: ["Stop", "media"], 0xb9: ["Reload", "media"], 0xba: ["Fav", "media"], 0xbb: ["⏩", "media"],
  0xbc: ["⏪", "media"], 0xbd: ["Bri+", "media"], 0xbe: ["Bri-", "media"],
  0xcd: ["Ms↑", "mouse"], 0xce: ["Ms↓", "mouse"], 0xcf: ["Ms←", "mouse"], 0xd0: ["Ms→", "mouse"],
  0xd1: ["click1", "mouse"], 0xd2: ["click2", "mouse"], 0xd3: ["click3", "mouse"], 0xd4: ["Btn4", "mouse"],
  0xd5: ["Btn5", "mouse"], 0xd6: ["Btn6", "mouse"], 0xd7: ["Btn7", "mouse"], 0xd8: ["Btn8", "mouse"],
  0xd9: ["Wh↑", "mouse"], 0xda: ["Wh↓", "mouse"], 0xdb: ["Wh←", "mouse"], 0xdc: ["Wh→", "mouse"],
  0xdd: ["Acc0", "mouse"], 0xde: ["Acc1", "mouse"], 0xdf: ["Acc2", "mouse"],
  0xe0: ["Ctrl", "mod"], 0xe1: ["Shift", "mod"], 0xe2: ["Alt", "mod"], 0xe3: ["Win", "mod"],
  0xe4: ["Ctrl", "mod"], 0xe5: ["Shift", "mod"], 0xe6: ["Alt", "mod"], 0xe7: ["Win", "mod"],
} as Record<number, [string, Category]>);
for (let i = 0; i < 12; i++) BASIC[0x3a + i] = ["F" + (i + 1), "fn"];
for (let i = 0; i < 12; i++) BASIC[0x68 + i] = ["F" + (i + 13), "fn"];
for (let i = 0; i < 9; i++) BASIC[0x59 + i] = ["N" + (i + 1), "num"];

// Shifted symbols on a US layout.
const SHIFTED: Record<number, string> = {
  0x1e: "!", 0x1f: "@", 0x20: "#", 0x21: "$", 0x22: "%", 0x23: "^", 0x24: "&", 0x25: "*",
  0x26: "(", 0x27: ")", 0x2d: "_", 0x2e: "+", 0x2f: "{", 0x30: "}", 0x31: "|", 0x33: ":",
  0x34: '"', 0x35: "~", 0x36: "<", 0x37: ">", 0x38: "?",
};

const LIGHTING: Record<number, string> = {
  0x7800: "BL On", 0x7801: "BL Off", 0x7802: "BL Tog", 0x7803: "BL-", 0x7804: "BL+", 0x7805: "BL Cyc",
  0x7806: "BL Br", 0x7820: "RGB Tog", 0x7821: "RGB Mod+", 0x7822: "RGB Mod-", 0x7823: "Hue+",
  0x7824: "Hue-", 0x7825: "Sat+", 0x7826: "Sat-", 0x7827: "Bri+", 0x7828: "Bri-", 0x7829: "Spd+",
  0x782a: "Spd-",
};

const QUANTUM: Record<number, string> = {
  0x7c00: "Boot", 0x7c01: "Reboot", 0x7c02: "Debug", 0x7c03: "EE Clr", 0x7c16: "Esc`",
  0x7c73: "CapsWd", 0x7c77: "Tri L", 0x7c78: "Tri U", 0x7c79: "Repeat", 0x7c7a: "AltRep",
};

function modNames(mods: number): string {
  const right = (mods & 0x10) !== 0;
  const out: string[] = [];
  if (mods & 0x01) out.push("Ctrl");
  if (mods & 0x02) out.push("Shift");
  if (mods & 0x04) out.push("Alt");
  if (mods & 0x08) out.push("Win");
  return (right ? "R" : "") + out.join("+");
}

function basic(kc: number, shift = false): Label {
  const b = BASIC[kc & 0xff];
  if (!b) return { text: "0x" + kc.toString(16).padStart(4, "0"), cat: "edit" };
  const [text, cat] = b;
  if (shift && SHIFTED[kc & 0xff]) return { text: SHIFTED[kc & 0xff], cat: "sym" };
  if (cat === "alpha") return { text: shift ? text : text.toLowerCase(), cat };
  if (cat === "mouse" && text.startsWith("click")) return { text: t(text), cat };
  if (text === "back" || text === "forward") return { text: t(text), cat };
  return { text, cat };
}

function layerText(kind: string, n: number): string {
  return t(`layer.${kind}`).replace("{n}", String(n));
}

/** Label for one keycode. */
export function label(kc: number, ctx: Context): Label {
  if (kc === KC_NO) return { text: "", cat: "none" };
  if (kc === KC_TRNS) return { text: "▽", cat: "trns" };
  if (kc <= 0xff) return basic(kc, ctx.shift);
  if (kc <= 0x1fff) {
    const mods = (kc >> 8) & 0x1f;
    const base = kc & 0xff;
    if (mods === 0x02 && SHIFTED[base]) return { text: SHIFTED[base], cat: "sym" };
    const b = basic(base);
    return { text: `${modNames(mods)}+\n${b.cat === "alpha" ? b.text.toUpperCase() : b.text}`, cat: "edit" };
  }
  if (kc <= 0x3fff) {
    const mods = (kc >> 8) & 0x1f;
    // tap key on top, "(Ctrl)" = the modifier while held
    return { text: `${basic(kc & 0xff, ctx.shift).text}\n(${modNames(mods)})`, cat: "mod" };
  }
  if (kc <= 0x4fff) {
    // tap key on top, "(L1)" = the layer while held
    return { text: `${basic(kc & 0xff, ctx.shift).text}\n(L${(kc >> 8) & 0x0f})`, cat: "layer" };
  }
  if (kc <= 0x51ff) return { text: `L${(kc >> 5) & 0x0f}\n+${modNames(kc & 0x1f)}`, cat: "layer" };
  if (kc <= 0x521f) return { text: layerText("to", kc & 0x1f), cat: "layer" };
  if (kc <= 0x523f) return { text: layerText("mo", kc & 0x1f), cat: "layer" };
  if (kc <= 0x525f) return { text: layerText("df", kc & 0x1f), cat: "layer" };
  if (kc <= 0x527f) return { text: layerText("tg", kc & 0x1f), cat: "layer" };
  if (kc <= 0x529f) return { text: layerText("osl", kc & 0x1f), cat: "layer" };
  if (kc <= 0x52bf) return { text: `${t("oneshot")}\n${modNames(kc & 0x1f)}`, cat: "mod" };
  if (kc <= 0x52df) return { text: layerText("tt", kc & 0x1f), cat: "layer" };
  if (kc <= 0x52ff) return { text: layerText("df", kc & 0x1f), cat: "layer" };
  if (kc >= 0x5700 && kc <= 0x57ff) return danceLabel(kc & 0xff, ctx);
  if (kc >= 0x7700 && kc <= 0x777f) {
    const n = kc & 0x7f;
    const d = ctx.macroText?.[n];
    // Long macros only show their number; the full text is in the lists.
    return { text: d && [...d].length <= 6 ? `M${n}\n${d}` : `M${n}`, cat: "macro" };
  }
  if (kc >= 0x7800 && kc <= 0x78ff) return { text: LIGHTING[kc] ?? "Light", cat: "system" };
  if (kc >= 0x7c00 && kc <= 0x7dff) return { text: QUANTUM[kc] ?? "0x" + kc.toString(16), cat: "system" };
  if (kc >= 0x7e00 && kc <= 0x7e3f) {
    const c = ctx.custom[kc - 0x7e00];
    const text = c?.shortName ?? c?.name ?? `USER${String(kc - 0x7e00).padStart(2, "0")}`;
    return { text, cat: "system" };
  }
  return { text: "0x" + kc.toString(16).padStart(4, "0"), cat: "edit" };
}

function short(kc: number, ctx: Context): string {
  if (kc === KC_NO) return "–";
  const e = layerEffect(kc);
  if (e) {
    switch (e.kind) {
      case "to": return `L${e.layer}`;
      case "mo": case "lt": case "lm": return `(L${e.layer})`;
      case "tg": case "tt": return `L${e.layer}⇄`;
      case "osl": return `L${e.layer}¹`;
      case "df": return `L${e.layer}*`;
    }
  }
  return label(kc, ctx).text.replace(/\n/g, " ");
}

function danceLabel(index: number, ctx: Context): Label {
  const td = ctx.tapDances[index];
  if (!td) return { text: `TD${index}`, cat: "dance" };
  const lines = [`${t("tap1")} ${short(td.onTap, ctx)}`];
  if (td.onHold) lines.push(`${t("holdShort")} ${short(td.onHold, ctx)}`);
  // A momentary action on a double tap is released at once and does nothing.
  const dbl = layerEffect(td.onDoubleTap);
  if (td.onDoubleTap && !(dbl && (dbl.kind === "mo" || dbl.kind === "lm"))) {
    lines.push(`${t("tap2")} ${short(td.onDoubleTap, ctx)}`);
  }
  if (td.onTapHold) lines.push(`${t("tapHold")} ${short(td.onTapHold, ctx)}`);
  return { text: lines.join("\n"), cat: "dance" };
}

export type SwitchKind = "to" | "mo" | "tg" | "osl" | "df" | "tt" | "lt" | "lm";

export interface LayerEffect {
  kind: SwitchKind;
  layer: number;
}

/** Layer effect of a keycode pressed on its own, if any. */
export function layerEffect(kc: number): LayerEffect | null {
  if (kc >= 0x4000 && kc <= 0x4fff) return { kind: "lt", layer: (kc >> 8) & 0x0f };
  if (kc >= 0x5000 && kc <= 0x51ff) return { kind: "lm", layer: (kc >> 5) & 0x0f };
  const ranges: [number, SwitchKind][] = [
    [0x5200, "to"], [0x5220, "mo"], [0x5240, "df"], [0x5260, "tg"], [0x5280, "osl"], [0x52c0, "tt"], [0x52e0, "df"],
  ];
  for (const [start, kind] of ranges) {
    if (kc >= start && kc < start + 0x20) return { kind, layer: kc - start };
  }
  return null;
}

export type Gesture = "press" | "tap" | "hold" | "double" | "tapHold";

/** Every way a key can change the layer, including tap dance branches. */
export function switchesOf(kc: number, ctx: Context): { gesture: Gesture; effect: LayerEffect }[] {
  if (kc >= 0x5700 && kc <= 0x57ff) {
    const td = ctx.tapDances[kc & 0xff];
    if (!td) return [];
    const out: { gesture: Gesture; effect: LayerEffect }[] = [];
    const add = (gesture: Gesture, code: number) => {
      const e = layerEffect(code);
      // A momentary action on a double tap is released immediately and does nothing.
      if (e && !(gesture === "double" && (e.kind === "mo" || e.kind === "lm"))) out.push({ gesture, effect: e });
    };
    add("tap", td.onTap);
    add("hold", td.onHold);
    add("double", td.onDoubleTap);
    add("tapHold", td.onTapHold);
    return out;
  }
  const e = layerEffect(kc);
  if (!e) return [];
  if (e.kind === "lt") return [{ gesture: "hold", effect: e }];
  if (e.kind === "tt") return [{ gesture: "hold", effect: { kind: "mo", layer: e.layer } }, { gesture: "tap", effect: { kind: "tg", layer: e.layer } }];
  return [{ gesture: "press", effect: e }];
}
