/**
 * "Start with Windows". The registry (read by the app side) is the only
 * record, so the switch is read again each time the settings menu opens.
 */
export type Call = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** Current state; off when it cannot be read. */
export async function readAutostart(call: Call): Promise<boolean> {
  try {
    return (await call("autostart_enabled")) === true;
  } catch {
    return false;
  }
}

/**
 * Ask for on or off and return the state to show. If the change fails the
 * switch goes back to what it was before.
 */
export async function switchAutostart(call: Call, want: boolean, before: boolean): Promise<boolean> {
  try {
    const now = await call("set_autostart", { enabled: want });
    return typeof now === "boolean" ? now : before;
  } catch (e) {
    console.error("set_autostart failed", e);
    return before;
  }
}
