import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { AutostartSwitch, type AutostartState, type Call } from "./autostart";
import { GLOW_CLASS, glowActive, raisePressed, readHighlight, saveHighlight } from "./highlight";
import { t } from "./i18n";
import { KC_NO, KC_TRNS, label, switchesOf, type Context, type CustomKeycode, type TapDance } from "./keycodes";
import { layoutFor, type Layout } from "./layout";
import { describeMacro, parseMacros } from "./macros";
import { renderBoard, type KeyView, type Part } from "./render";
import { createStore } from "./settings";
import { standardLabel, standardLayout, type StandardKind, type StandardLayout } from "./standard";

interface DeviceInfo {
  path: string;
  vendor_id: number;
  product_id: number;
  product: string;
  manufacturer: string;
}

interface KeyboardInfo {
  id: string;
  device: DeviceInfo;
  layers: number;
  rows: number;
  cols: number;
  keymap: number[];
  tapDances: TapDance[];
  definition: { name?: string; customKeycodes?: CustomKeycode[] } | null;
  matrixAvailable: boolean;
  firmwareReports: boolean;
  autoMouseLayer: number | null;
  /** Whether the firmware has the mouse layer on out of the box. */
  autoMouseDefault?: boolean;
  legacyKeycodes: boolean;
  macroBuffer: number[];
  macroCount: number;
  /** [layer][encoder] = [counter-clockwise, clockwise] */
  encoders: number[][][];
}

interface LiveState {
  connected: boolean;
  layerState: number;
  defaultLayer: number;
  top: number;
  source: "keys" | "firmware" | "";
  pressed: [number, number][];
  autoMouse: boolean;
  paused: boolean;
  notice: string | null;
}

interface Snapshot {
  keyboard: KeyboardInfo | null;
  state: LiveState;
  devices: DeviceInfo[];
  clickThrough: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const win = getCurrentWindow();

let kb: KeyboardInfo | null = null;
/** Showing the keymap remembered from the last connection. */
let cached = false;
let st: LiveState = { connected: false, layerState: 0, defaultLayer: 1, top: 0, source: "", pressed: [], autoMouse: false, paused: false, notice: null };
let devices: DeviceInfo[] = [];
let layout: Layout | null = null;
let ctx: Context = { tapDances: [], custom: [] };
let preview: number | null = null;
let clickThrough = false;
let names: Record<string, string> = {};
/** Per-keyboard part overrides: "row,col" -> hidden / trackball. */
let parts: Record<string, Part> = {};
let editingParts = false;

// ---------------------------------------------------------------- settings

const store = createStore(() => localStorage);

let showSwitches = store.get("showSwitches", true);
let opacity = store.get("opacity", 0.86);
let clearBackground = store.get("clearBackground", false);
let alwaysOnTop = store.get("alwaysOnTop", true);
/**
 * Board size works like Vial: +/- set the size of one key in pixels; the board
 * stays centred inside the window and only overflows (and scrolls) when it no
 * longer fits. "Fit" mode instead follows the window size.
 */
/** Key size in px for the current keyboard; 0 = not chosen yet. */
let keyPx = 0;
let fitMode = store.get("fitMode", false);
const KEY_MIN = 14;
const KEY_MAX = 400;
let keysOnly = store.get("keysOnly", false);
let ignoreConfigurators = store.get("ignoreConfigurators", false);
/** Follow the current keyboard's trackball mouse layer (per keyboard). */
let autoMouse = false;
/** Width / height of the board currently drawn, and its height in key units. */
let boardAspect = 3;
let boardUnits = 7;
let keyOpacity = store.get("keyOpacity", 1);
/** Dim the board and light up only the pressed keys. */
let highlight = readHighlight(store);
/** "Start with Windows". Kept by the app side, not in the window's storage. */
const callApp: Call = (cmd, args) => invoke(cmd, args);
const autostart = new AutostartSwitch(callApp, showAutostart);
/** "auto": follow the keyboard typed on last; "tracked": always the Vial keyboard; "standard": always a plain layout. */
let displayMode = store.get<"auto" | "tracked" | "standard">("displayMode", "auto");
let standardKind = store.get<StandardKind>("standardKind", "us");
let typingOnTracked: boolean | null = null;
let otherKeys = new Set<number>();
let standardCache: { kind: StandardKind; layout: StandardLayout } | null = null;

function applyOpacity() {
  document.documentElement.style.setProperty("--hud-alpha", String(clearBackground ? 0 : opacity));
  document.documentElement.style.setProperty("--key-alpha", String(keyOpacity));
  document.getElementById("hud")?.classList.toggle("clear", clearBackground || opacity < 0.05);
}

// ---------------------------------------------------------------- keymap helpers

function code(layer: number, row: number, col: number): number {
  if (!kb) return KC_NO;
  return kb.keymap[(layer * kb.rows + row) * kb.cols + col] ?? KC_NO;
}

/** Default layer bitmask; layer 0 when nothing is known (e.g. not connected). */
function defaultMask(): number {
  return st.defaultLayer || 1;
}

function activeMask(): number {
  if (preview !== null) return (1 << preview) | defaultMask();
  return st.layerState | defaultMask();
}

function shownLayer(): number {
  return preview ?? st.top;
}

function resolve(row: number, col: number, mask: number): { kc: number; layer: number } {
  if (!kb) return { kc: KC_NO, layer: 0 };
  for (let l = Math.min(kb.layers, 32) - 1; l >= 0; l--) {
    if (!(mask & (1 << l))) continue;
    const kc = code(l, row, col);
    if (kc !== KC_TRNS) return { kc, layer: l };
  }
  return { kc: KC_NO, layer: 0 };
}

/** Rotation keycode of a dial on the active layers (transparent falls through). */
function resolveEncoder(id: number, dir: 0 | 1, mask: number): number {
  if (!kb?.encoders?.length) return KC_NO;
  for (let l = Math.min(kb.layers, 32) - 1; l >= 0; l--) {
    if (!(mask & (1 << l))) continue;
    const kc = kb.encoders[l]?.[id]?.[dir] ?? KC_NO;
    if (kc !== KC_TRNS) return kc;
  }
  return KC_NO;
}

/** Push label on top, then the two rotation directions. */
function dialLabel(key: { row: number; col: number; encoder?: number }, pushKc: number, mask: number, c: Context) {
  const push = label(pushKc, c);
  const flat = (kc: number) => label(kc, c).text.replace(/\n/g, " ");
  const ccw = resolveEncoder(key.encoder!, 0, mask);
  const cw = resolveEncoder(key.encoder!, 1, mask);
  const lines: string[] = [];
  if (push.text && push.cat !== "trns") lines.push(push.text.split("\n")[0]);
  if (ccw !== KC_NO || cw !== KC_NO) lines.push(`↺${flat(ccw)}`, `↻${flat(cw)}`);
  return { text: lines.join("\n"), cat: push.cat === "none" || push.cat === "trns" ? ("media" as const) : push.cat };
}

function layerUsed(l: number): boolean {
  if (!kb) return false;
  for (let i = 0; i < kb.rows * kb.cols; i++) {
    const kc = kb.keymap[l * kb.rows * kb.cols + i];
    if (kc !== KC_NO && kc !== KC_TRNS) return true;
  }
  return false;
}

function layerName(l: number): string {
  return names[l] ?? "";
}

function layerTitle(l: number): string {
  const n = layerName(l);
  return n ? `L${l} ${n}` : `L${l}`;
}

// ---------------------------------------------------------------- rendering

let queued = false;
function schedule() {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    render();
  });
}

/** A keyboard is connected but its layout is still unknown. */
function awaitingDefinition(): boolean {
  return !!st.notice?.startsWith("definition-missing:");
}

function showingStandard(): boolean {
  if (awaitingDefinition()) return false;
  if (displayMode === "standard") return true;
  if (displayMode === "tracked") return false;
  return typingOnTracked === false || !kb;
}

/** Turn the dimmed "highlight pressed keys" look on or off for this frame. */
function applyGlow(standard: boolean): boolean {
  const on = glowActive({
    enabled: highlight,
    previewing: preview !== null,
    editingParts,
    standard,
    connected: st.connected,
    cached,
    paused: st.paused,
    notice: st.notice,
  });
  $("hud").classList.toggle(GLOW_CLASS, on);
  return on;
}

function renderStandard(glow: boolean) {
  if (standardCache?.kind !== standardKind) standardCache = { kind: standardKind, layout: standardLayout(standardKind) };
  const lay = standardCache.layout;
  $("empty").hidden = true;
  $("switches").hidden = true;
  $("layer-num").textContent = "⌨";
  $("layer-num").className = "layer-color-0";
  $("layer-name").textContent = t(`std.${standardKind}`);
  $("chips").innerHTML = "";
  $("status").textContent = displayMode === "auto" ? t("std.auto") : "";
  const shift = otherKeys.has(0x2a) || otherKeys.has(0x36);
  const views: KeyView[] = lay.keys.map((key) => ({
    key,
    label: standardLabel(key, shift, ctx),
    inherited: false,
    pressed: otherKeys.has(key.col),
  }));
  renderBoard(document.getElementById("board") as unknown as SVGSVGElement, lay, raisePressed(views, glow));
  sizeBoard(lay.width / lay.height, lay.height);
}

function render() {
  const hud = $("hud");
  const warn = $("warn");
  warn.hidden = st.notice !== "configurator-running";
  warn.textContent = t("warn.vial");
  hud.classList.toggle("macro-mode", macroView);
  $("macro-view").hidden = !macroView;
  if (macroView) {
    renderMacroView();
    return;
  }
  hud.classList.toggle("click-through", clickThrough);
  hud.classList.toggle("previewing", preview !== null);
  hud.classList.toggle("no-switches", !showSwitches);

  const standard = showingStandard();
  const glow = applyGlow(standard);
  if (standard) {
    renderStandard(glow);
    return;
  }
  const empty = $("empty");
  $("switches").hidden = !kb;
  const board = document.getElementById("board") as unknown as SVGSVGElement;
  if (!kb || !layout || awaitingDefinition()) {
    empty.hidden = false;
    empty.textContent =
      st.notice?.startsWith("definition-missing:") ? t("st.needDefinition").replace("{name}", st.notice.slice(19))
      : st.notice === "configurator-running" ? t("st.vial")
      : st.paused ? t("st.paused")
      // Other notices are connection errors from the app side; they are not shown.
      : t("st.searching");
    board.innerHTML = "";
    $("switches").innerHTML = "";
    $("layer-num").textContent = "–";
    $("layer-name").textContent = "";
    $("chips").innerHTML = "";
    $("status").textContent = "";
    return;
  }
  empty.hidden = true;

  const shown = shownLayer();
  const mask = activeMask();
  $("layer-num").textContent = `L${shown}`;
  $("layer-num").className = `layer-color-${shown % 8}`;
  const nameEl = $("layer-name");
  if (!nameEl.isContentEditable) nameEl.textContent = layerName(shown) || (shown === 0 ? t("base") : "");

  // layer chips
  const chips: string[] = [];
  for (let l = 0; l < kb.layers; l++) {
    const on = (mask & (1 << l)) !== 0;
    if (!layerUsed(l) && !on) continue;
    const cls = ["chip", `lc-${l % 8}`, on ? "on" : "", l === shown ? "top" : "", (defaultMask() & (1 << l)) ? "default" : ""].join(" ");
    chips.push(`<button class="${cls}" data-layer="${l}" title="${layerTitle(l)}">${l}</button>`);
  }
  $("chips").innerHTML = chips.join("");

  // status line
  const status: string[] = [];
  if (preview !== null && !editingParts) status.push(t("tip.preview"));
  else if (!cached) {
    status.push(st.source === "firmware" ? t("src.firmware") : t("src.keys"));
    if (st.autoMouse) status.push(t("st.automouse"));
  }
  if (cached) status.push(t("st.cached"));
  if (st.notice === "configurator-running") status.push(t("st.vial"));
  else if (st.paused) status.push(t("st.paused"));
  if (st.notice === "matrix-locked") status.push(t("st.locked"));
  $("status").textContent = status.join(" · ");

  // board
  const pressed = new Set(st.pressed.map(([r, c]) => `${r},${c}`));
  const shift =
    preview === null &&
    (st.pressed.some(([r, c]) => {
      const kc = resolve(r, c, mask).kc;
      return kc === 0xe1 || kc === 0xe5;
    }) ||
      otherKeys.has(0x2a) ||
      otherKeys.has(0x36));
  const shown_ctx = { ...ctx, shift };
  const macroKeys = new Map<number, string>();
  const views: KeyView[] = layout.keys.map((key) => {
    const r = resolve(key.row, key.col, mask);
    const macro = r.kc >= 0x7700 && r.kc <= 0x777f ? r.kc & 0x7f : -1;
    if (macro >= 0 && !parts[`${key.row},${key.col}`]) macroKeys.set(macro, key.name);
    return {
      key,
      tip: macro >= 0 ? `M${macro}: ${ctx.macroText?.[macro] ?? ""}` : undefined,
      label: key.encoder !== undefined ? dialLabel(key, r.kc, mask, shown_ctx) : label(r.kc, shown_ctx),
      inherited: r.layer !== shown && r.kc !== KC_NO,
      pressed: preview === null && pressed.has(`${key.row},${key.col}`),
      part: parts[`${key.row},${key.col}`],
      editing: editingParts,
    };
  });
  renderBoard(board, layout, raisePressed(views, glow));
  sizeBoard(layout.width / layout.height, layout.height);

  // how to switch from here
  const list = $("switches");
  if (!showSwitches) {
    list.innerHTML = "";
    return;
  }
  // one entry per key, listing every gesture that changes the layer
  type Entry = { name: string; parts: string[]; home: boolean; first: number };
  const entries: Entry[] = [];
  const base = 31 - Math.clz32(st.defaultLayer || 1);
  for (const key of layout.keys) {
    if (parts[`${key.row},${key.col}`]) continue;
    const r = resolve(key.row, key.col, mask);
    const acts: string[] = [];
    let home = false;
    let first = 99;
    for (const s of switchesOf(r.kc, ctx)) {
      const momentary = s.effect.kind === "mo" || s.effect.kind === "lt" || s.effect.kind === "lm";
      if (momentary && s.effect.layer === shown) continue;
      if (s.effect.kind === "to" && s.effect.layer === shown && shown !== base) continue;
      const isHome = s.effect.kind === "to" && s.effect.layer === base;
      home ||= isHome;
      first = Math.min(first, s.effect.layer);
      const lname = layerName(s.effect.layer);
      acts.push(
        `<span class="g">${escapeHtml(t(`g.${s.gesture}`))}</span>` +
          `<b class="layer-color-${s.effect.layer % 8}" title="${escapeHtml(lname)}">L${s.effect.layer}</b>` +
          `<span class="k">${escapeHtml(t(`k.${s.effect.kind}`))}</span>`,
      );
    }
    if (acts.length) entries.push({ name: key.name, parts: acts, home, first });
  }
  entries.sort((a, b) => Number(b.home) - Number(a.home) || a.first - b.first);
  const rows = entries.map(
    (e) => `<li class="${e.home ? "home" : ""}"><span class="where">${escapeHtml(e.name.replace(/^\[(.+)\]$/, "$1"))}</span>${e.parts.map((p) => `<span class="act">${p}</span>`).join("")}</li>`,
  );
  const macroRows = [...macroKeys.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(
      ([n, where]) =>
        `<li class="macro"><span class="where">M${n}</span><span class="k">${escapeHtml(where)}</span>` +
        `<span class="desc">${escapeHtml(ctx.macroText?.[n] ?? "")}</span></li>`,
    );
  list.innerHTML =
    (rows.length ? `<ul>${rows.join("")}</ul>` : `<p>${t("switchNone")}</p>`) +
    (macroRows.length ? `<ul class="macros">${macroRows.join("")}</ul>` : "");
}

function toast(text: string) {
  const el = document.createElement("div");
  el.className = "toast";
  el.textContent = text;
  $("hud").appendChild(el);
  setTimeout(() => el.remove(), 4000);
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
}

// ---------------------------------------------------------------- data

function setKeyboard(info: KeyboardInfo | null) {
  preview = null;
  cached = false;
  if (info) {
    store.set("lastKeyboard", info);
  } else {
    // Keep showing the last keymap while the keyboard is away (or Vial has it).
    info = store.get<KeyboardInfo | null>("lastKeyboard", null);
    cached = info !== null;
  }
  kb = info;
  if (!info) {
    layout = null;
    schedule();
    return;
  }
  layout = layoutFor(info.device.vendor_id, info.device.product_id, info.definition);
  ctx = { tapDances: info.tapDances, custom: info.definition?.customKeycodes ?? [] };
  const macros = parseMacros(info.macroBuffer ?? [], info.macroCount ?? 0);
  ctx.macroText = macros.map((m) => describeMacro(m, ctx));
  // Keyboards without friendly key names: call keys by what they type on layer 0.
  if (layout) {
    layout.keys = layout.keys.map((k) => {
      if (!/^R\d+C\d+$/.test(k.name)) return k;
      const base = label(code(0, k.row, k.col), ctx).text.split("\n")[0].trim();
      return base && base !== "▽" ? { ...k, name: `[${base}]` } : k;
    });
  }
  names = store.get(`names.${info.id}`, {} as Record<string, string>);
  parts = store.get(`parts.${info.id}`, {} as Record<string, Part>);
  keyPx = store.get(`keyPx.${info.id}`, 0);
  autoMouse = info.autoMouseLayer != null && store.get(`autoMouse.${info.id}`, info.autoMouseDefault ?? true);
  if (!cached && info.autoMouseLayer != null) invoke("set_auto_mouse", { enabled: autoMouse });
  schedule();
}

// ---------------------------------------------------------------- menu

function buildMenu() {
  const menu = $("menu");
  const deviceOptions = devices
    .map((d) => `<option value="${escapeHtml(d.path)}" ${kb?.device.path === d.path ? "selected" : ""}>${escapeHtml(d.product || d.path)}</option>`)
    .join("");
  const hint = (key?: string) => (key ? `<span class="hint">${t(key)}</span>` : "");
  const sec = (key: string) => `<div class="sec">${t(key)}</div>`;
  const field = (label: string, control: string) => `<label class="row"><span class="lbl">${label}</span>${control}</label>`;
  const toggle = (id: string, on: boolean, label: string, hintKey?: string, title?: string) =>
    `<label class="row"${title ? ` title="${escapeHtml(title)}"` : ""}><span class="lbl">${label}${hint(hintKey)}</span>` +
    `<input id="${id}" class="sw" type="checkbox" ${on ? "checked" : ""}></label>`;
  const item = (act: string, label: string, hintKey?: string, disabled = false) =>
    `<button data-act="${act}" ${disabled ? "disabled" : ""}><span class="lbl">${label}${hint(hintKey)}</span></button>`;
  menu.innerHTML = `
    ${sec("sec.view")}
    ${field(t("menu.display"), `<select id="m-display">
      <option value="auto" ${displayMode === "auto" ? "selected" : ""}>${t("menu.display.auto")}</option>
      <option value="tracked" ${displayMode === "tracked" ? "selected" : ""}>${t("menu.display.tracked")}</option>
      <option value="standard" ${displayMode === "standard" ? "selected" : ""}>${t("menu.display.standard")}</option>
    </select>`)}
    ${field(t("menu.standard"), `<select id="m-standard">
      <option value="us" ${standardKind === "us" ? "selected" : ""}>${t("std.us")}</option>
      <option value="jis" ${standardKind === "jis" ? "selected" : ""}>${t("std.jis")}</option>
    </select>`)}
    ${field(t("menu.keyOpacity"), `<input id="m-keys" type="range" min="0.1" max="1" step="0.01" value="${keyOpacity}">`)}
    ${field(t("menu.opacity"), `<input id="m-opacity" type="range" min="0" max="1" step="0.01" value="${opacity}" ${clearBackground ? "disabled" : ""}>`)}
    ${toggle("m-highlight", highlight, t("menu.highlight"))}
    ${toggle("m-clear", clearBackground, t("menu.clear"))}
    ${toggle("m-switches", showSwitches, t("menu.layerNames"))}
    ${toggle("m-keysonly", keysOnly, t("menu.keysOnly"), "hint.keysOnly")}
    ${sec("sec.track")}
    ${devices.length > 1 ? field(t("menu.device"), `<select id="m-device">${deviceOptions}</select>`) : ""}
    ${kb?.autoMouseLayer != null ? toggle("m-automouse", autoMouse, t("menu.autoMouse"), "hint.autoMouse", t("menu.autoMouseNote")) : ""}
    ${toggle("m-ignore", ignoreConfigurators, t("menu.ignoreVial"), "hint.ignoreVial", t("menu.ignoreVialNote"))}
    ${toggle("m-pause", st.paused, t("menu.pause"), "hint.pause")}
    ${sec("sec.actions")}
    ${item("rename", t("menu.names"))}
    ${item("parts", editingParts ? t("parts.done") : t("menu.parts"), editingParts ? undefined : "hint.parts")}
    ${item("follow", t("menu.follow"), undefined, preview === null)}
    ${item("reload", t("menu.reload"))}
    ${item("reset", t("menu.reset"))}
    ${sec("sec.window")}
    ${item("fit", t("menu.fit"))}
    ${item("click", t("menu.click"), "hint.click")}
    ${item("hide", t("menu.hide"), "hint.hide")}
    ${toggle("m-autostart", autostart.state === true, t("menu.autostart"))}`;
  // Filled part of the sliders.
  for (const r of menu.querySelectorAll<HTMLInputElement>('input[type="range"]')) {
    const fill = () => r.style.setProperty("--val", `${((+r.value - +r.min) / (+r.max - +r.min)) * 100}%`);
    fill();
    r.addEventListener("input", fill);
  }
  ($("m-display") as HTMLSelectElement).onchange = (e) => {
    displayMode = (e.target as HTMLSelectElement).value as typeof displayMode;
    store.set("displayMode", displayMode);
    schedule();
  };
  ($("m-standard") as HTMLSelectElement).onchange = (e) => {
    standardKind = (e.target as HTMLSelectElement).value as StandardKind;
    store.set("standardKind", standardKind);
    schedule();
  };
  ($("m-keys") as HTMLInputElement).oninput = (e) => {
    keyOpacity = Number((e.target as HTMLInputElement).value);
    store.set("keyOpacity", keyOpacity);
    applyOpacity();
  };
  ($("m-highlight") as HTMLInputElement).onchange = (e) => {
    setHighlight((e.target as HTMLInputElement).checked);
    // Keeps the tray menu's check mark in step.
    invoke("set_highlight", { enabled: highlight }).catch(() => {});
  };
  ($("m-clear") as HTMLInputElement).onchange = (e) => {
    clearBackground = (e.target as HTMLInputElement).checked;
    store.set("clearBackground", clearBackground);
    ($("m-opacity") as HTMLInputElement).disabled = clearBackground;
    applyOpacity();
  };
  ($("m-opacity") as HTMLInputElement).oninput = (e) => {
    opacity = Number((e.target as HTMLInputElement).value);
    store.set("opacity", opacity);
    applyOpacity();
  };
  ($("m-ignore") as HTMLInputElement).onchange = (e) => {
    ignoreConfigurators = (e.target as HTMLInputElement).checked;
    store.set("ignoreConfigurators", ignoreConfigurators);
    invoke("set_ignore_configurators", { ignore: ignoreConfigurators });
  };
  const am = document.getElementById("m-automouse") as HTMLInputElement | null;
  if (am) am.onchange = () => {
    autoMouse = am.checked;
    if (kb) store.set(`autoMouse.${kb.id}`, autoMouse);
    invoke("set_auto_mouse", { enabled: autoMouse });
  };
  ($("m-keysonly") as HTMLInputElement).onchange = (e) => {
    keysOnly = (e.target as HTMLInputElement).checked;
    store.set("keysOnly", keysOnly);
    applyKeysOnly();
  };
  ($("m-switches") as HTMLInputElement).onchange = (e) => {
    showSwitches = (e.target as HTMLInputElement).checked;
    store.set("showSwitches", showSwitches);
    schedule();
  };
  ($("m-pause") as HTMLInputElement).onchange = (e) => {
    invoke("set_paused", { paused: (e.target as HTMLInputElement).checked });
  };
  const dev = document.getElementById("m-device") as HTMLSelectElement | null;
  if (dev) dev.onchange = () => invoke("select_device", { path: dev.value });
  const start = $("m-autostart") as HTMLInputElement;
  start.onchange = () => void autostart.change(start.checked);
  showAutostart(autostart.state, autostart.busy);
}

/** "Start with Windows": show the state read from the app side. While it
 * cannot be read, or a change is on its way, the switch cannot be touched. */
function showAutostart(state: AutostartState, busy: boolean) {
  const box = document.getElementById("m-autostart") as HTMLInputElement | null;
  if (!box) return;
  box.checked = state === true;
  box.disabled = busy || state === null;
}

/** Shrink or grow the window height so the board fills the width exactly. */
async function fitWindow() {
  if (!layout) return;
  const wrap = $("board-wrap");
  const wanted = wrap.clientWidth / boardAspect;
  const extra = window.innerHeight - wrap.clientHeight;
  const scale = await win.scaleFactor();
  const size = await win.innerSize();
  const height = Math.max(170, Math.round(wanted + extra));
  await win.setSize(new LogicalSize(size.width / scale, height));
}

// ---------------------------------------------------------------- zoom

/** Key size (px) at which the board exactly fits the window. */
function fitKeyPx(): number {
  const wrap = $("board-wrap");
  const w = wrap.clientWidth;
  const h = wrap.clientHeight;
  const lw = boardAspect * boardUnits;
  return Math.max(KEY_MIN, Math.min(w / lw, h / boardUnits));
}

function sizeBoard(aspect: number, units?: number) {
  boardAspect = aspect;
  if (units) boardUnits = units;
  const wrap = $("board-wrap");
  if (!wrap.clientWidth || !wrap.clientHeight) return;
  // First time with this keyboard: start a little smaller than the window,
  // so + grows the board inside the window before it ever overflows.
  if (!keyPx) keyPx = Math.max(KEY_MIN, Math.floor(fitKeyPx() * 0.8));
  const px = fitMode ? fitKeyPx() : keyPx;
  const svg = document.getElementById("board") as unknown as SVGSVGElement;
  svg.style.width = `${Math.floor(px * boardUnits * boardAspect)}px`;
  svg.style.height = `${Math.floor(px * boardUnits)}px`;
  $("zoom").querySelector<HTMLButtonElement>('[data-zoom="fit"]')?.classList.toggle("active", fitMode);
}

/** Change the key size, keeping the point under the cursor (or the centre) in place. */
function setKeyPx(next: number, anchor?: { x: number; y: number }) {
  const wrap = $("board-wrap");
  const ax = anchor?.x ?? wrap.clientWidth / 2;
  const ay = anchor?.y ?? wrap.clientHeight / 2;
  const fx = (wrap.scrollLeft + ax) / Math.max(1, wrap.scrollWidth);
  const fy = (wrap.scrollTop + ay) / Math.max(1, wrap.scrollHeight);
  // Stop once at the exact size that still fits before going past it.
  const cur = currentKeyPx();
  const fit = fitKeyPx();
  if ((cur < fit - 0.5 && next > fit) || (cur > fit + 0.5 && next < fit)) next = fit;
  keyPx = Math.round(Math.min(KEY_MAX, Math.max(KEY_MIN, next)) * 10) / 10;
  fitMode = false;
  if (kb) store.set(`keyPx.${kb.id}`, keyPx);
  store.set("fitMode", fitMode);
  sizeBoard(boardAspect);
  wrap.scrollLeft = fx * wrap.scrollWidth - ax;
  wrap.scrollTop = fy * wrap.scrollHeight - ay;
}

/** Start from what is on screen now (the fitted size when in fit mode). */
function currentKeyPx(): number {
  return fitMode ? fitKeyPx() : keyPx;
}

function toggleFit() {
  fitMode = !fitMode;
  if (!fitMode) keyPx = Math.round(fitKeyPx());
  store.set("fitMode", fitMode);
  if (kb) store.set(`keyPx.${kb.id}`, keyPx);
  sizeBoard(boardAspect);
}

function wireZoom() {
  const wrap = $("board-wrap");
  const buttons = $("zoom");
  const tips: Record<string, string> = { in: t("zoom.in"), out: t("zoom.out"), fit: t("zoom.fit") };
  buttons.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.title = tips[b.dataset.zoom!]));
  buttons.onclick = (e) => {
    const z = (e.target as HTMLElement).closest("button")?.dataset.zoom;
    if (z === "in") setKeyPx(currentKeyPx() * 1.15);
    if (z === "out") setKeyPx(currentKeyPx() / 1.15);
    if (z === "fit") toggleFit();
  };
  // Ctrl + wheel zooms around the cursor
  wrap.addEventListener(
    "wheel",
    (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      const r = wrap.getBoundingClientRect();
      setKeyPx(currentKeyPx() * (e.deltaY < 0 ? 1.08 : 1 / 1.08), { x: e.clientX - r.left, y: e.clientY - r.top });
    },
    { passive: false },
  );
  // drag to move around when zoomed in
  let drag: { x: number; y: number; left: number; top: number } | null = null;
  wrap.addEventListener("mousedown", (e) => {
    const overflowing = wrap.scrollWidth > wrap.clientWidth + 1 || wrap.scrollHeight > wrap.clientHeight + 1;
    if (e.button !== 0 || !overflowing || editingParts) return;
    drag = { x: e.clientX, y: e.clientY, left: wrap.scrollLeft, top: wrap.scrollTop };
    wrap.classList.add("panning");
  });
  window.addEventListener("mousemove", (e) => {
    if (!drag) return;
    wrap.scrollLeft = drag.left - (e.clientX - drag.x);
    wrap.scrollTop = drag.top - (e.clientY - drag.y);
  });
  window.addEventListener("mouseup", () => {
    drag = null;
    wrap.classList.remove("panning");
  });
  new ResizeObserver(() => sizeBoard(boardAspect)).observe(wrap);
}

/** Switch "highlight pressed keys" (from the settings menu or the tray) and remember it. */
function setHighlight(on: boolean) {
  highlight = on;
  saveHighlight(store, on);
  const box = document.getElementById("m-highlight") as HTMLInputElement | null;
  if (box) box.checked = on;
  schedule();
}

function applyKeysOnly() {
  $("hud").classList.toggle("keys-only", keysOnly);
  sizeBoard(boardAspect);
}

function applyPin() {
  const b = $("btn-pin");
  b.classList.toggle("active", alwaysOnTop);
  b.title = alwaysOnTop ? t("tip.pinOn") : t("tip.pinOff");
  win.setAlwaysOnTop(alwaysOnTop);
}

/** Full-size macro list that temporarily replaces the keyboard view. */
let macroView = false;

function toggleMacroView(force?: boolean) {
  macroView = force ?? !macroView;
  $("btn-macros").classList.toggle("active", macroView);
  schedule();
}

function renderMacroView() {
  const view = $("macro-view");
  $("layer-num").textContent = "M";
  $("layer-num").className = "layer-color-3";
  $("layer-name").textContent = t("macro.title");
  $("chips").innerHTML = "";
  $("status").textContent = t("macro.back");
  const texts = ctx.macroText ?? [];

  // where each macro is assigned: "L15 right ring N"
  const where = new Map<number, string[]>();
  if (kb && layout) {
    const names = new Map(layout.keys.map((k) => [`${k.row},${k.col}`, k.name]));
    for (let l = 0; l < kb.layers; l++) {
      for (let r = 0; r < kb.rows; r++) {
        for (let c = 0; c < kb.cols; c++) {
          const kc = code(l, r, c);
          if (kc < 0x7700 || kc > 0x777f) continue;
          const n = kc & 0x7f;
          const list = where.get(n) ?? [];
          list.push(`L${l} ${names.get(`${r},${c}`) ?? `R${r}C${c}`}`);
          where.set(n, list);
        }
      }
    }
  }

  const cards = texts
    .map((d, n) => ({ d, n }))
    .filter(({ d }) => d && d !== t("macro.empty"))
    .map(({ d, n }) => {
      const steps = d.split(" → ").map((p) => `<span class="step">${escapeHtml(p)}</span>`).join('<span class="arrow">→</span>');
      const used = where.get(n);
      return (
        `<article class="macro-card"><header><b>M${n}</b>` +
        `<span class="used">${used ? escapeHtml(used.join(" / ")) : t("macro.unassigned")}</span></header>` +
        `<div class="steps">${steps}</div></article>`
      );
    });
  view.innerHTML = cards.length ? cards.join("") : `<p class="none">${t("macro.none")}</p>`;
}

/** Leave menus, the macro list, part editing and layer preview. */
function closeTemporaryModes() {
  toggleMenu(false);
  if (macroView) toggleMacroView(false);
  if (editingParts) toggleEditParts(false);
  preview = null;
  schedule();
}

function toggleEditParts(force?: boolean) {
  editingParts = force ?? !editingParts;
  $("hud").classList.toggle("editing-parts", editingParts);
  const banner = $("edit-banner");
  banner.hidden = !editingParts;
  banner.innerHTML = `<span>${escapeHtml(t("parts.help"))}</span><button id="edit-done">${escapeHtml(t("parts.doneShort"))}</button>`;
  const done = document.getElementById("edit-done");
  if (done) done.onclick = () => toggleEditParts(false);
  schedule();
}

/** In edit mode a click cycles a key: shown -> hidden -> trackball -> shown. */
function cyclePart(key: string) {
  if (!kb) return;
  const next: Record<string, Part | undefined> = { undefined: "hidden", hidden: "trackball", trackball: undefined };
  const value = next[String(parts[key])];
  if (value) parts[key] = value;
  else delete parts[key];
  store.set(`parts.${kb.id}`, parts);
  schedule();
}

function toggleMenu(force?: boolean) {
  const menu = $("menu");
  const open = force ?? menu.hidden;
  if (open) buildMenu();
  menu.hidden = !open;
  // Task Manager can change it while the app runs, so read it each time.
  if (open) void autostart.refresh();
}

function startRename() {
  const el = $("layer-name");
  const layer = shownLayer();
  el.contentEditable = "true";
  el.textContent = layerName(layer);
  el.focus();
  const finish = (save: boolean) => {
    el.contentEditable = "false";
    if (save && kb) {
      const v = (el.textContent ?? "").trim();
      if (v) names[layer] = v;
      else delete names[layer];
      store.set(`names.${kb.id}`, names);
    }
    el.onkeydown = null;
    el.onblur = null;
    schedule();
  };
  el.onkeydown = (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true);
    } else if (e.key === "Escape") finish(false);
  };
  el.onblur = () => finish(true);
}

// ---------------------------------------------------------------- wiring

function wire() {
  $("btn-switch").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M3.5 14.5h17"/></svg>';
  $("btn-menu").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h8M16 7h4M4 17h4M12 17h8"/><circle cx="14" cy="7" r="2"/><circle cx="10" cy="17" r="2"/></svg>';
  $("btn-macros").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h11M4 11h11M4 16h6"/><path d="M15 14v6l5-3z"/></svg>';
  for (const z of ["in", "out", "fit"] as const) {
    const icon = { in: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M12 6v12M6 12h12"/></svg>', out: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h12"/></svg>', fit: '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5"/></svg>' }[z];
    $("zoom").querySelector(`[data-zoom="${z}"]`)!.innerHTML = icon;
  }
  $("btn-switch").title = t("tip.switch");
  $("btn-menu").title = t("tip.menu");
  $("btn-macros").title = t("tip.macros");
  $("btn-min").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h12"/></svg>';
  $("btn-close").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M7 7l10 10M17 7L7 17"/></svg>';
  $("btn-min").title = t("tip.minimize");
  $("btn-close").title = t("tip.close");
  $("btn-min").onclick = () => invoke("minimize_main");
  $("btn-close").onclick = () => invoke("hide_main_to_tray");
  $("btn-pin").innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z"/><path d="M12 14v6"/></svg>';
  $("btn-pin").onclick = () => {
    alwaysOnTop = !alwaysOnTop;
    store.set("alwaysOnTop", alwaysOnTop);
    applyPin();
  };
  $("btn-macros").onclick = (e) => {
    e.stopPropagation();
    toggleMenu(false);
    toggleMacroView();
  };
  $("grip").title = t("tip.grip");
  $("layer-name").title = t("menu.names");

  $("btn-switch").onclick = () => {
    showSwitches = !showSwitches;
    store.set("showSwitches", showSwitches);
    schedule();
  };
  $("btn-menu").onclick = (e) => {
    e.stopPropagation();
    toggleMenu();
  };
  $("layer-name").ondblclick = startRename;
  document.getElementById("board")!.addEventListener("click", (e) => {
    if (!editingParts) return;
    const g = (e.target as Element).closest("g[data-key]");
    const key = g?.getAttribute("data-key");
    if (key) cyclePart(key);
  });
  $("layer-num").onclick = () => {
    preview = null;
    schedule();
  };
  $("chips").onclick = (e) => {
    const b = (e.target as HTMLElement).closest("button[data-layer]") as HTMLElement | null;
    if (!b) return;
    const l = Number(b.dataset.layer);
    preview = preview === l || (preview === null && l === st.top) ? null : l;
    schedule();
  };
  $("menu").onclick = async (e) => {
    const act = (e.target as HTMLElement).closest("button")?.dataset.act;
    if (!act) return;
    toggleMenu(false);
    if (act === "fit") await fitWindow();
    if (act === "parts") toggleEditParts();
    if (act === "rename") startRename();
    if (act === "follow") preview = null;
    if (act === "reload") await invoke("reload");
    if (act === "reset") await invoke("reset_layers");
    if (act === "click") await invoke("set_click_through", { enabled: true });
    if (act === "hide") await invoke("hide_main_to_tray");
    schedule();
  };
  document.addEventListener("click", (e) => {
    if (!(e.target as HTMLElement).closest("#menu")) toggleMenu(false);
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeTemporaryModes();
  });

  // resize from every edge and corner
  const dirs = ["North", "South", "East", "West", "NorthEast", "NorthWest", "SouthEast", "SouthWest"] as const;
  for (const d of dirs) {
    const h = document.createElement("div");
    h.className = `edge edge-${d}`;
    h.onmousedown = (e) => {
      e.preventDefault();
      win.startResizeDragging(d);
    };
    $("hud").appendChild(h);
  }
  $("grip").ondblclick = () => fitWindow();
  $("grip").onmousedown = (e) => {
    e.preventDefault();
    win.startResizeDragging("SouthEast");
  };
}

async function main() {
  applyOpacity();
  wire();
  wireZoom();
  applyPin();
  applyKeysOnly();
  await listen<KeyboardInfo | null>("hud://keyboard", (e) => setKeyboard(e.payload));
  await listen<LiveState>("hud://state", (e) => {
    const prevTop = st.top;
    st = e.payload;
    if (preview !== null && st.top !== prevTop) preview = null;
    schedule();
  });
  await listen<DeviceInfo[]>("hud://devices", (e) => (devices = e.payload));
  await listen<boolean>("hud://typing", (e) => {
    typingOnTracked = e.payload;
    schedule();
  });
  await listen<number[]>("hud://other-keys", (e) => {
    otherKeys = new Set(e.payload);
    if (showingStandard()) schedule();
  });
  // Esc from any keyboard, even when the HUD window does not have focus.
  await listen("hud://escape", () => {
    if (editingParts || macroView || preview !== null || !$("menu").hidden) closeTemporaryModes();
  });
  await listen<boolean>("hud://click-through", (e) => {
    clickThrough = e.payload;
    schedule();
  });
  // Switched from the tray menu (and the echo of our own changes).
  await listen<boolean>("hud://highlight", (e) => setHighlight(e.payload === true));
  // Only the tray check mark depends on this; never hold up start-up for it.
  invoke("set_highlight", { enabled: highlight }).catch(() => {});
  await getCurrentWebview().onDragDropEvent(async (e) => {
    if (e.payload.type !== "drop") return;
    const file = e.payload.paths.find((p) => p.toLowerCase().endsWith(".json"));
    if (file) await invoke("import_definition", { path: file });
  });
  await listen<{ ok: boolean; id?: string; error?: string }>("hud://import", (e) => {
    // The reason is for the log only; the window says what to do.
    if (!e.payload.ok) console.error("import failed", e.payload.error);
    toast(e.payload.ok ? t("import.ok") : t("import.failed"));
  });
  if (ignoreConfigurators) await invoke("set_ignore_configurators", { ignore: true });
  const snap = await invoke<Snapshot>("snapshot");
  st = snap.state;
  devices = snap.devices;
  clickThrough = snap.clickThrough;
  setKeyboard(snap.keyboard);
}

main();
