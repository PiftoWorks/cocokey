import { beforeAll, describe, expect, it } from "vitest";

import { GLOW_CLASS, glowActive, raisePressed, readHighlight, saveHighlight, type GlowInput } from "./highlight";
import type { KeyView } from "./render";
import { createStore } from "./settings";

// Source text for the wiring checks. Read from disk: vitest hands CSS imports
// over empty, even with ?raw. (The module name is built so tsc, which has no
// Node types here, does not look it up.)
let css = "", html = "", mainTs = "", i18n = "", libRs = "";
beforeAll(async () => {
  const fs = await import(/* @vite-ignore */ "node:" + "fs");
  const read = (rel: string): string => fs.readFileSync(new URL(rel, import.meta.url), "utf8");
  css = read("./styles.css");
  html = read("../index.html");
  mainTs = read("./main.ts");
  i18n = read("./i18n.ts");
  libRs = read("../src-tauri/src/lib.rs");
});

function memoryStorage() {
  const data = new Map<string, string>();
  return { data, getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
}

describe("highlight pressed keys: saved choice", () => {
  it("is off by default", () => {
    expect(readHighlight(createStore(() => memoryStorage()))).toBe(false);
  });

  it("is still on at the next launch after switching it on", () => {
    const mem = memoryStorage();
    saveHighlight(createStore(() => mem), true);
    expect(readHighlight(createStore(() => mem))).toBe(true);
    saveHighlight(createStore(() => mem), false);
    expect(readHighlight(createStore(() => mem))).toBe(false);
  });

  it("treats anything but true as off", () => {
    const mem = memoryStorage();
    mem.data.set("cocokey.highlightPressed", '"yes"');
    expect(readHighlight(createStore(() => mem))).toBe(false);
  });
});

describe("highlight pressed keys: when the board is dimmed", () => {
  const live: GlowInput = {
    enabled: true,
    previewing: false,
    editingParts: false,
    standard: false,
    connected: true,
    cached: false,
    paused: false,
    notice: null,
  };

  it("dims while tracking a connected keyboard", () => {
    expect(glowActive(live)).toBe(true);
  });

  it("never dims when the setting is off (the default view)", () => {
    expect(glowActive({ ...live, enabled: false })).toBe(false);
    expect(glowActive({ ...live, enabled: false, standard: true })).toBe(false);
  });

  it("shows the board normally when no key can light up", () => {
    expect(glowActive({ ...live, previewing: true })).toBe(false);
    expect(glowActive({ ...live, editingParts: true })).toBe(false);
    expect(glowActive({ ...live, paused: true })).toBe(false);
    expect(glowActive({ ...live, paused: true, notice: "configurator-running" })).toBe(false);
    expect(glowActive({ ...live, notice: "configurator-running" })).toBe(false);
    expect(glowActive({ ...live, notice: "matrix-locked" })).toBe(false);
    expect(glowActive({ ...live, connected: false })).toBe(false);
    expect(glowActive({ ...live, cached: true })).toBe(false);
  });

  it("comes back once tracking resumes (Vial closed, unpaused)", () => {
    const stopped = { ...live, paused: true, notice: "configurator-running" };
    expect(glowActive(stopped)).toBe(false);
    expect(glowActive({ ...stopped, paused: false, notice: null })).toBe(true);
  });

  it("dims the standard keyboard even while Vial is open (its keys come from every keyboard)", () => {
    expect(glowActive({ ...live, standard: true, connected: false, paused: true, notice: "configurator-running" })).toBe(true);
  });
});

describe("highlight pressed keys: drawing order", () => {
  const view = (col: number, pressed: boolean): KeyView => ({
    key: { row: 0, col, x: col, y: 0, w: 1, h: 1, name: `R0C${col}` } as KeyView["key"],
    label: { text: "", cat: "alpha" } as KeyView["label"],
    inherited: false,
    pressed,
  });
  const views = [view(0, false), view(1, true), view(2, false), view(3, true)];

  it("draws pressed keys last so the glow is on top", () => {
    expect(raisePressed(views, true).map((v) => v.key.col)).toEqual([0, 2, 1, 3]);
  });

  it("leaves the order alone in the default view", () => {
    expect(raisePressed(views, false)).toBe(views);
  });
});

describe("highlight pressed keys: wiring", () => {
  it("only styles keys under #hud.glow-mode, and the default pressed look is unchanged", () => {
    expect(GLOW_CLASS).toBe("glow-mode");
    const glowRules = css.match(/^[^{}\n]*\.key[^{}\n]*\{/gm)!.filter((s) => s.includes("glow"));
    expect(glowRules.length).toBeGreaterThan(0);
    for (const sel of glowRules) expect(sel).toContain("#hud.glow-mode");
    expect(css).toMatch(/\.key\.pressed rect \{\s*stroke: var\(--accent\);\s*stroke-width: 0\.08;\s*opacity: 1;\s*\}/);
    expect(css).toMatch(/\.key\.pressed circle \{\s*stroke: var\(--accent\);\s*stroke-width: 0\.08;\s*\}/);
  });

  it("has the glow filter the CSS points at", () => {
    const ref = css.match(/url\(#([\w-]+)\)/)![1];
    expect(html).toContain(`<filter id="${ref}"`);
    // the defs must not take the board's id
    expect(html.match(/id="board"/g)).toHaveLength(1);
  });

  it("names the switch the same in the settings menu and the tray, in both languages", () => {
    expect(i18n).toContain('"menu.highlight": "Highlight pressed keys"');
    expect(i18n).toContain('"menu.highlight": "押したキーを強調表示"');
    expect(libRs).toContain('t("押したキーを強調表示", "Highlight pressed keys")');
  });

  it("keeps the tray and the window in step", () => {
    expect(libRs).toMatch(/generate_handler!\[[^\]]*set_highlight/);
    expect(libRs).toContain('"hud://highlight"');
    expect(mainTs).toContain('listen<boolean>("hud://highlight"');
    expect(mainTs).toContain('invoke("set_highlight"');
  });
});
