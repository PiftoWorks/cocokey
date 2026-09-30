import { describe, expect, it } from "vitest";

import { label, layerEffect, switchesOf, type Context } from "./keycodes";

const ctx: Context = {
  tapDances: [
    { onTap: 0x5201, onHold: 0x5221, onDoubleTap: 0x522f, onTapHold: 0, termMs: 300 },
    { onTap: 0x5200, onHold: 0x5222, onDoubleTap: 0x5202, onTapHold: 0, termMs: 300 },
  ],
  custom: [{ name: "SV_LEFT_DPI_INC", shortName: "Left\nDPI +" }],
};

describe("label", () => {
  it("names basic keys", () => {
    expect(label(0x04, ctx).text).toBe("a");
    expect(label(0x04, { ...ctx, shift: true }).text).toBe("A");
    expect(label(0x1e, { ...ctx, shift: true }).text).toBe("!");
    expect(label(0x2c, ctx).text).toBe("Space");
    expect(label(0xe1, ctx).cat).toBe("mod");
    expect(label(0x3a, ctx).text).toBe("F1");
  });

  it("shows shifted symbols on a US layout", () => {
    expect(label(0x021e, ctx).text).toBe("!");
    expect(label(0x0238, ctx).text).toBe("?");
  });

  it("handles transparent and empty keys", () => {
    expect(label(0x0001, ctx).cat).toBe("trns");
    expect(label(0x0000, ctx).cat).toBe("none");
  });

  it("uses custom keycode names from the definition", () => {
    expect(label(0x7e00, ctx).text).toBe("Left\nDPI +");
  });

  it("labels macros", () => {
    expect(label(0x7706, ctx).text).toBe("M6");
  });
});

describe("layer switches", () => {
  it("decodes layer keycodes", () => {
    expect(layerEffect(0x5203)).toEqual({ kind: "to", layer: 3 });
    expect(layerEffect(0x5284)).toEqual({ kind: "osl", layer: 4 });
    expect(layerEffect(0x0004)).toBeNull();
  });

  it("expands tap dance branches and drops momentary double taps", () => {
    const s = switchesOf(0x5700, ctx);
    expect(s.map((x) => x.gesture)).toEqual(["tap", "hold"]);
    expect(switchesOf(0x5701, ctx).map((x) => [x.gesture, x.effect.kind, x.effect.layer])).toEqual([
      ["tap", "to", 0],
      ["hold", "mo", 2],
      ["double", "to", 2],
    ]);
  });
});
