import { describe, expect, it } from "vitest";

import { describeMacro, parseMacros } from "./macros";

const ctx = { tapDances: [], custom: [] };

describe("macros", () => {
  it("parses taps, 16-bit keycodes, delays and text", () => {
    const buf = [
      // macro 0: click, Ctrl+K, wait 500ms, Ctrl+V, wait 200ms, Enter
      0x01, 0x01, 0xd1, 0x01, 0x05, 0x0e, 0x01, 0x01, 0x04, 246, 2, 0x01, 0x05, 0x19, 0x01,
      0x01, 0x04, 201, 1, 0x01, 0x01, 0x28, 0x00,
      // macro 1: Ctrl down, A down, Ctrl up, A up
      0x01, 0x02, 0xe0, 0x01, 0x02, 0x04, 0x01, 0x03, 0xe0, 0x01, 0x03, 0x04, 0x00,
      // macro 2: plain text
      0x68, 0x69, 0x00,
    ];
    const macros = parseMacros(buf, 3);
    expect(macros).toHaveLength(3);
    expect(macros[0][2]).toEqual({ kind: "delay", ms: 500 });
    expect(macros[0][3]).toEqual({ kind: "tap", kc: 0x0119 });
    expect(describeMacro(macros[1], ctx)).toBe("Ctrl+A");
    expect(macros[2]).toEqual([{ kind: "text", text: "hi" }]);
  });

  it("maps 0xFFxx back to modifier-only keycodes", () => {
    const macros = parseMacros([0x01, 0x05, 0x02, 0xff, 0x00], 1);
    expect(macros[0][0]).toEqual({ kind: "tap", kc: 0x0200 });
  });
});
