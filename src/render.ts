import type { Label } from "./keycodes";
import type { Layout, PhysKey } from "./layout";

export interface KeyView {
  key: PhysKey;
  label: Label;
  /** Label comes from a lower layer because this layer is transparent here. */
  inherited: boolean;
  pressed: boolean;
  /** Extra tooltip text (e.g. the full macro). */
  tip?: string;
  /** How the user wants this part drawn (see "edit parts"). */
  part?: Part;
  /** Parts are being edited: show hidden ones faintly. */
  editing?: boolean;
}

/** Per-key override for keyboards whose build differs from the definition. */
export type Part = "hidden" | "trackball";

const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

function textWidth(s: string): number {
  let w = 0;
  for (const ch of s) {
    const c = ch.charCodeAt(0);
    // rough advance widths of the semibold UI font, in ems
    w += c > 0x2e80 ? 1.0 : /[MWmw%@]/.test(ch) ? 0.9 : /[A-Z0-9]/.test(ch) ? 0.68 : /[iljtf.,:;'!|]/.test(ch) ? 0.34 : 0.58;
  }
  return w;
}

/** Smallest label size (in key widths); below this text is cut instead. */
const MIN_SIZE = 0.2;

function fitText(lines: string[], w: number, h: number): number {
  const widest = Math.max(...lines.map(textWidth), 1);
  const byWidth = (w - 0.08) / widest;
  const byHeight = (h - 0.08) / (lines.length * 1.08);
  const cap = lines.length === 1 && [...lines[0]].length <= 2 ? 0.52 : lines.length === 1 ? 0.38 : 0.32;
  return Math.max(MIN_SIZE, Math.min(cap, byWidth, byHeight));
}

/** Shorten a line with "…" so it fits `max` width units at the given size. */
function clip(line: string, w: number, size: number): string {
  const max = (w - 0.06) / size;
  if (textWidth(line) <= max) return line;
  let out = "";
  for (const ch of line) {
    if (textWidth(out + ch + "…") > max) break;
    out += ch;
  }
  return out + "…";
}

/** Key name, its matrix position (useful when checking a definition) and extras. */
function tooltip(v: KeyView): string {
  const k = v.key;
  const pos = k.row >= 0 ? ` (R${k.row}C${k.col})` : "";
  const head = k.name.startsWith(`R${k.row}C${k.col}`) ? k.name : `${k.name}${pos}`;
  return v.tip ? `${head}\n${v.tip}` : head;
}

export function renderBoard(svg: SVGSVGElement, layout: Layout, views: KeyView[]): void {
  svg.setAttribute("viewBox", `0 0 ${layout.width.toFixed(3)} ${layout.height.toFixed(3)}`);
  // Centred in the window. The list below has a fixed height, so the board
  // does not move when its contents change.
  svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
  const parts: string[] = [];
  for (const c of layout.captions) {
    parts.push(`<text class="caption" x="${c.x.toFixed(3)}" y="${c.y.toFixed(3)}">${esc(c.text)}</text>`);
  }
  for (const v of views) {
    const k = v.key;
    const inset = 0.035;
    const x = k.x + inset, y = k.y + inset, w = k.w - inset * 2, h = k.h - inset * 2;
    const rot = k.r ? ` transform="rotate(${k.r} ${k.rx ?? 0} ${k.ry ?? 0})"` : "";
    if (v.part === "hidden" && !v.editing) continue;
    const ball = v.part === "trackball";
    const cls = [
      "key",
      ball ? "trackball" : `cat-${v.label.cat}`,
      v.inherited && !ball ? "inherited" : "",
      v.pressed ? "pressed" : "",
      v.part === "hidden" ? "hidden-part" : "",
      v.editing ? "editable" : "",
    ].join(" ");
    const labelText = ball ? "🖲" : v.label.text;
    let lines = labelText ? labelText.split("\n") : [];
    const size = lines.length ? fitText(lines, w, h) : 0;
    // drop lines that no longer fit vertically, then clip the rest
    const maxLines = Math.max(1, Math.floor((h - 0.06) / (size * 1.08)));
    if (lines.length > maxLines) lines = lines.slice(0, maxLines);
    lines = lines.map((l) => clip(l, w, size));
    const lh = size * 1.08;
    const top = y + h / 2 - (lh * lines.length) / 2 + size * 0.86;
    const text = lines
      .map((line, i) => `<text x="${(x + w / 2).toFixed(3)}" y="${(top + i * lh).toFixed(3)}" font-size="${size.toFixed(3)}">${esc(line)}</text>`)
      .join("");
    parts.push(
      `<g class="${cls}" data-key="${k.row},${k.col}"${rot}><title>${esc(tooltip(v))}</title>` +
        (k.encoder !== undefined || ball
          ? `<circle class="dial" cx="${(x + w / 2).toFixed(3)}" cy="${(y + h / 2).toFixed(3)}" r="${((Math.min(w, h) / 2) * (ball ? 1.35 : 1)).toFixed(3)}" />`
          : `<rect x="${x.toFixed(3)}" y="${y.toFixed(3)}" width="${w.toFixed(3)}" height="${h.toFixed(3)}" rx="0.12" />`) +
        text +
        `</g>`,
    );
  }
  svg.innerHTML = parts.join("");
}
