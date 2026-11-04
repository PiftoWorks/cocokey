import { describe, expect, it } from "vitest";

import { createStore } from "./settings";

/** Stand-in for the window's localStorage, shared between "launches". */
function memoryStorage() {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, String(v)),
  };
}

describe("settings store", () => {
  it("keeps values under cocokey.<key> as JSON, as before", () => {
    const mem = memoryStorage();
    createStore(() => mem).set("opacity", 0.5);
    expect(mem.data.get("cocokey.opacity")).toBe("0.5");
  });

  it("reads back what an earlier launch saved", () => {
    const mem = memoryStorage();
    createStore(() => mem).set("keysOnly", true);
    expect(createStore(() => mem).get("keysOnly", false)).toBe(true);
  });

  it("falls back when nothing or something unreadable is saved", () => {
    const mem = memoryStorage();
    mem.data.set("cocokey.broken", "{not json");
    const store = createStore(() => mem);
    expect(store.get("missing", 7)).toBe(7);
    expect(store.get("broken", "x")).toBe("x");
  });

  it("keeps working when storage cannot be reached", () => {
    const store = createStore(() => {
      throw new Error("storage disabled");
    });
    expect(store.get("opacity", 0.86)).toBe(0.86);
    expect(() => store.set("opacity", 1)).not.toThrow();
  });
});
