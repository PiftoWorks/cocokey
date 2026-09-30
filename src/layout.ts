// Physical key positions. Units are key widths (1 = one standard key).

import { isJapanese } from "./i18n";

export interface PhysKey {
  row: number;
  col: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Rotation in degrees around (rx, ry). */
  r?: number;
  rx?: number;
  ry?: number;
  /** Name used in the switch list, e.g. "Left thumb · Up". */
  name: string;
  /** Rotary encoder index when this key is a dial (its push switch). */
  encoder?: number;
}

export interface Caption {
  text: string;
  x: number;
  y: number;
}

export interface Layout {
  keys: PhysKey[];
  captions: Caption[];
  width: number;
  height: number;
}

function bounds(keys: PhysKey[], captions: Caption[]): Layout {
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const k of keys) {
    const corners = [[k.x, k.y], [k.x + k.w, k.y], [k.x, k.y + k.h], [k.x + k.w, k.y + k.h]];
    for (const [cx, cy] of corners) {
      let px = cx, py = cy;
      if (k.r) {
        const a = (k.r * Math.PI) / 180;
        const ox = k.rx ?? 0, oy = k.ry ?? 0;
        px = ox + (cx - ox) * Math.cos(a) - (cy - oy) * Math.sin(a);
        py = oy + (cx - ox) * Math.sin(a) + (cy - oy) * Math.cos(a);
      }
      minX = Math.min(minX, px); minY = Math.min(minY, py);
      maxX = Math.max(maxX, px); maxY = Math.max(maxY, py);
    }
  }
  for (const c of captions) minY = Math.min(minY, c.y - 0.35);
  const pad = 0.04;
  const dx = pad - minX, dy = pad - minY;
  const move = (k: PhysKey): PhysKey => ({
    ...k, x: k.x + dx, y: k.y + dy,
    rx: k.rx !== undefined ? k.rx + dx : undefined,
    ry: k.ry !== undefined ? k.ry + dy : undefined,
  });
  return {
    keys: keys.map(move),
    captions: captions.map((c) => ({ ...c, x: c.x + dx, y: c.y + dy })),
    width: maxX - minX + pad * 2,
    height: maxY - minY + pad * 2,
  };
}

/** Parse the KLE layout embedded in a Vial / VIA keyboard definition. */
export function fromKle(kle: unknown[]): Layout {
  const keys: PhysKey[] = [];
  const cur = { x: 0, y: 0, w: 1, h: 1, r: 0, rx: 0, ry: 0 };
  for (const row of kle) {
    if (!Array.isArray(row)) continue;
    for (const item of row) {
      if (item && typeof item === "object") {
        const p = item as Record<string, number>;
        if (p.r !== undefined) cur.r = p.r;
        if (p.rx !== undefined) { cur.rx = p.rx; cur.x = cur.rx; cur.y = cur.ry; }
        if (p.ry !== undefined) { cur.ry = p.ry; cur.x = cur.rx; cur.y = cur.ry; }
        if (p.x !== undefined) cur.x += p.x;
        if (p.y !== undefined) cur.y += p.y;
        if (p.w !== undefined) cur.w = p.w;
        if (p.h !== undefined) cur.h = p.h;
        continue;
      }
      if (typeof item === "string") {
        const legends = item.split("\n");
        const m = /^(\d+)\s*,\s*(\d+)$/.exec(legends[0] ?? "");
        const option = /^(\d+)\s*,\s*(\d+)$/.exec(legends[3] ?? "");
        const dial = /^e(\d+)$/.exec(legends[9] ?? "");
        const hidden = option !== null && option[2] !== "0";
        if (m && !hidden) {
          const row = Number(m[1]), col = Number(m[2]);
          keys.push({
            row, col, x: cur.x, y: cur.y, w: cur.w, h: cur.h,
            r: cur.r || undefined, rx: cur.r ? cur.rx : undefined, ry: cur.r ? cur.ry : undefined,
            name: `R${row}C${col}`,
            encoder: dial ? Number(dial[1]) : undefined,
          });
        }
        cur.x += cur.w;
        cur.w = 1;
        cur.h = 1;
      }
    }
    cur.y += 1;
    cur.x = cur.rx;
  }
  return bounds(keys, []);
}

interface ParsedKey {
  row: number;
  col: number;
  x: number;
  y: number;
  w?: number;
  h?: number;
  r?: number;
  rx?: number;
  ry?: number;
  /** Decal (drawn in KLE but not a real key). */
  d?: boolean;
  /** Encoder index. */
  ei?: number;
}

/** Layout already converted to key positions, as in VIA's definition catalog. */
export function fromParsed(keys: ParsedKey[], optionKeys: Record<string, Record<string, ParsedKey[]>> = {}): Layout {
  // Layout options: use the first choice of every group.
  const optional = Object.values(optionKeys).flatMap((choices) => choices["0"] ?? []);
  const out: PhysKey[] = [...keys, ...optional]
    .filter((k) => !k.d)
    .map((k) => ({
      row: k.row, col: k.col, x: k.x, y: k.y, w: k.w ?? 1, h: k.h ?? 1,
      r: k.r || undefined, rx: k.r ? k.rx : undefined, ry: k.r ? k.ry : undefined,
      name: `R${k.row}C${k.col}`,
      encoder: k.ei,
    }));
  return bounds(out, []);
}

// ---------------------------------------------------------------- Svalboard
// Matrix rows: 0 = left thumb, 1-4 = left index..pinky, 5 = right thumb,
// 6-9 = right index..pinky. Finger columns: 0 = south, 1 = east, 2 = center,
// 3 = north, 4 = west. Thumb columns: 0 = knuckle, 1 = nail, 2 = down,
// 3 = pad, 4 = up, 5 = double down. Positions come from the layout stored in
// the firmware (the same one Vial draws); only the key names are added here.

const FINGER = isJapanese
  ? ["", "人差し指", "中指", "薬指", "小指"]
  : ["", "index", "middle", "ring", "pinky"];
const DIRS = isJapanese
  ? ["南", "東", "中央", "北", "西"]
  : ["S", "E", "center", "N", "W"];
const THUMB = isJapanese
  ? ["関節側", "爪側", "下", "パッド", "上", "強く下"]
  : ["knuckle", "nail", "down", "pad", "up", "double-down"];
const SIDE = isJapanese ? ["左", "右"] : ["Left", "Right"];
const THUMB_WORD = isJapanese ? "親指" : "thumb";

function svalboardName(row: number, col: number): string {
  const side = row < 5 ? 0 : 1;
  const r = row % 5;
  if (r === 0) {
    return isJapanese ? `${SIDE[side]}${THUMB_WORD}・${THUMB[col]}` : `${SIDE[side]} ${THUMB_WORD} ${THUMB[col]}`;
  }
  return isJapanese ? `${SIDE[side]}${FINGER[r]}・${DIRS[col]}` : `${SIDE[side]} ${FINGER[r]} ${DIRS[col]}`;
}

/** Friendly key names for keyboards we know, by vendor/product id. */
const NAMERS: Record<string, (row: number, col: number) => string> = {
  "303a:4044": svalboardName,
};

/** Physical layout of a keyboard, taken from its own definition. */
export function layoutFor(vendorId: number, productId: number, definition: unknown): Layout | null {
  const layouts = (definition as { layouts?: { keymap?: unknown[]; keys?: ParsedKey[]; optionKeys?: Record<string, Record<string, ParsedKey[]>> } })?.layouts;
  let layout: Layout;
  if (Array.isArray(layouts?.keymap)) layout = fromKle(layouts.keymap);
  else if (Array.isArray(layouts?.keys)) layout = fromParsed(layouts.keys, layouts.optionKeys);
  else return null;
  const id = `${vendorId.toString(16).padStart(4, "0")}:${productId.toString(16).padStart(4, "0")}`;
  const namer = NAMERS[id];
  if (namer) layout.keys = layout.keys.map((k) => ({ ...k, name: namer(k.row, k.col) }));
  return layout;
}
