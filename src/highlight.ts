import type { KeyView } from "./render";
import type { Store } from "./settings";

/**
 * "Highlight pressed keys": the board is dimmed and only the keys being
 * pressed are drawn at full brightness with a glow. The default view is left
 * exactly as it was; everything here only applies while #hud has GLOW_CLASS.
 */
export const HIGHLIGHT_SETTING = "highlightPressed";
export const GLOW_CLASS = "glow-mode";

/** Off unless the saved value is exactly `true` (old or broken values mean off). */
export function readHighlight(store: Store): boolean {
  return store.get<unknown>(HIGHLIGHT_SETTING, false) === true;
}

export function saveHighlight(store: Store, on: boolean): void {
  store.set(HIGHLIGHT_SETTING, on);
}

export interface GlowInput {
  /** The setting. */
  enabled: boolean;
  /** Looking at another layer: pressed keys are not shown then. */
  previewing: boolean;
  editingParts: boolean;
  /** The standard keyboard is shown (pressed keys come from any keyboard). */
  standard: boolean;
  connected: boolean;
  /** Showing the keymap remembered from the last connection. */
  cached: boolean;
  paused: boolean;
  notice: string | null;
}

/**
 * Whether to dim the board right now. Only while a pressed key can actually
 * light up: when tracking is stopped (paused, Vial open, keyboard locked or
 * not connected), while previewing a layer or editing parts, the board is
 * drawn normally so it stays readable.
 */
export function glowActive(i: GlowInput): boolean {
  if (!i.enabled || i.previewing || i.editingParts) return false;
  if (i.standard) return true;
  return i.connected && !i.cached && !i.paused && i.notice !== "configurator-running" && i.notice !== "matrix-locked";
}

/**
 * Pressed keys go last so their glow is drawn over the neighbouring keys
 * instead of under them. Order is unchanged when `on` is false.
 */
export function raisePressed(views: KeyView[], on: boolean): KeyView[] {
  if (!on) return views;
  return [...views.filter((v) => !v.pressed), ...views.filter((v) => v.pressed)];
}
