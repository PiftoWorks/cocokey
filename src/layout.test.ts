import { describe, expect, it } from "vitest";

import { fromKle, fromParsed, layoutFor } from "./layout";

describe("layouts", () => {
  it("reads KLE rows with offsets", () => {
    const l = fromKle([["0,0", { x: 1 }, "0,1"], [{ y: 0.5, w: 2 }, "1,0"]]);
    expect(l.keys.map((k) => [k.row, k.col, k.w])).toEqual([[0, 0, 1], [0, 1, 1], [1, 0, 2]]);
    expect(l.keys[1].x - l.keys[0].x).toBeCloseTo(2);
    expect(l.keys[2].y - l.keys[0].y).toBeCloseTo(1.5);
  });

  it("hides non-default layout options in KLE", () => {
    const l = fromKle([["0,0\n\n\n0,0", "0,1\n\n\n0,1"]]);
    expect(l.keys.map((k) => k.col)).toEqual([0]);
  });

  it("reads VIA catalog layouts and the first choice of each option", () => {
    const l = fromParsed(
      [{ row: 0, col: 0, x: 0, y: 0 }, { row: 0, col: 1, x: 1, y: 0, d: true }],
      { "0": { "0": [{ row: 1, col: 0, x: 0, y: 1, w: 2 }], "1": [{ row: 1, col: 1, x: 0, y: 1 }] } },
    );
    expect(l.keys.map((k) => `${k.row},${k.col}`)).toEqual(["0,0", "1,0"]);
  });

  it("names Svalboard keys", () => {
    const def = { layouts: { keymap: [["0,4", "5,4", "1,3"]] } };
    const names = layoutFor(0x303a, 0x4044, def)!.keys.map((k) => k.name);
    expect(names).toHaveLength(3);
    expect(names[0]).not.toBe("R0C4");
  });
});
