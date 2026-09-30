// Decoding of the dynamic macro buffer used by VIA / Vial.
//
// Macros are stored back to back, each terminated by 0x00. Inside a macro,
// plain bytes are typed as text; 0x01 starts a command:
//   01 01 kc        tap      01 05 lo hi   tap (16-bit keycode)
//   01 02 kc        press    01 06 lo hi   press (16-bit)
//   01 03 kc        release  01 07 lo hi   release (16-bit)
//   01 04 d0 d1     wait (d0 - 1) + (d1 - 1) * 255 ms
// 16-bit keycodes 0xFF01..0xFFFF stand for 0x0100..0xFF00 (a zero byte would
// end the macro).

import { isJapanese, t } from "./i18n";
import { label, type Context } from "./keycodes";

export type Step =
  | { kind: "tap" | "down" | "up"; kc: number }
  | { kind: "delay"; ms: number }
  | { kind: "text"; text: string };

export function parseMacros(buffer: number[], count: number): Step[][] {
  const macros: Step[][] = [];
  let cur: Step[] = [];
  let text = "";
  const flushText = () => {
    if (text) cur.push({ kind: "text", text });
    text = "";
  };
  for (let i = 0; i < buffer.length && macros.length < count; i++) {
    const b = buffer[i];
    if (b === 0) {
      flushText();
      macros.push(cur);
      cur = [];
      continue;
    }
    if (b !== 0x01) {
      text += String.fromCharCode(b);
      continue;
    }
    flushText();
    const op = buffer[++i];
    if (op >= 1 && op <= 3) {
      cur.push({ kind: (["tap", "down", "up"] as const)[op - 1], kc: buffer[++i] });
    } else if (op >= 5 && op <= 7) {
      let kc = buffer[i + 1] | (buffer[i + 2] << 8);
      i += 2;
      if (kc > 0xff00) kc = (kc & 0xff) << 8;
      cur.push({ kind: (["tap", "down", "up"] as const)[op - 5], kc });
    } else if (op === 4) {
      cur.push({ kind: "delay", ms: buffer[i + 1] - 1 + (buffer[i + 2] - 1) * 255 });
      i += 2;
    }
  }
  return macros;
}

function keyName(kc: number, ctx: Context): string {
  const l = label(kc, { ...ctx, shift: false });
  if (l.cat === "alpha") return l.text.toUpperCase();
  return (l.text || `0x${kc.toString(16)}`).replace(/\n/g, isJapanese ? "" : " ");
}

/** Readable one-line description, e.g. "Ctrl+K → 0.5s → Ctrl+V → Enter". */
export function describeMacro(steps: Step[], ctx: Context): string {
  const parts: string[] = [];
  let i = 0;
  while (i < steps.length) {
    const s = steps[i];
    if (s.kind === "down") {
      // A run of presses followed by the matching releases is a key combination.
      const downs: number[] = [];
      let j = i;
      while (j < steps.length && steps[j].kind === "down") downs.push((steps[j++] as { kc: number }).kc);
      const ups: number[] = [];
      while (j < steps.length && steps[j].kind === "up") ups.push((steps[j++] as { kc: number }).kc);
      if (ups.length === downs.length && ups.every((kc) => downs.includes(kc))) {
        parts.push(downs.map((kc) => keyName(kc, ctx)).join("+"));
        i = j;
        continue;
      }
      parts.push(`${keyName(s.kc, ctx)}${t("macro.down")}`);
    } else if (s.kind === "up") {
      parts.push(`${keyName(s.kc, ctx)}${t("macro.up")}`);
    } else if (s.kind === "tap") {
      parts.push(keyName(s.kc, ctx));
    } else if (s.kind === "delay") {
      parts.push(s.ms >= 1000 ? `${(s.ms / 1000).toFixed(1)}s` : `${s.ms}ms`);
    } else if (s.kind === "text") {
      parts.push(`“${s.text}”`);
    }
    i++;
  }
  return parts.length ? parts.join(" → ") : t("macro.empty");
}
