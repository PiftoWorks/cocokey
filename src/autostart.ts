/**
 * "Start with Windows". The registry (read by the app side) is the only
 * record, so the switch is read again each time the settings menu opens.
 */
export type Call = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** On, off, or `null` when it cannot be read (the switch is then left alone). */
export type AutostartState = boolean | null;

export async function readAutostart(call: Call): Promise<AutostartState> {
  try {
    const on = await call("autostart_enabled");
    return typeof on === "boolean" ? on : null;
  } catch (e) {
    console.error("autostart_enabled failed", e);
    return null;
  }
}

/**
 * Ask for on or off and return the state to show. The app side answers with
 * the state it read back, also when one of its steps failed; if it cannot
 * answer, read the state again instead of guessing.
 */
export async function switchAutostart(call: Call, want: boolean): Promise<AutostartState> {
  try {
    const now = await call("set_autostart", { enabled: want });
    if (typeof now === "boolean") return now;
  } catch (e) {
    console.error("set_autostart failed", e);
  }
  return readAutostart(call);
}

/**
 * Keeps the switch in step with the answers. Reading (when the menu opens)
 * and switching can overlap; only the answer to the latest request is shown,
 * so an older read never overwrites a newer change.
 */
export class AutostartSwitch {
  state: AutostartState = null;
  busy = false;
  private latest = 0;

  constructor(
    private readonly call: Call,
    private readonly show: (state: AutostartState, busy: boolean) => void,
  ) {}

  async refresh(): Promise<void> {
    const ticket = ++this.latest;
    const state = await readAutostart(this.call);
    if (ticket === this.latest) this.apply(state);
  }

  async change(want: boolean): Promise<void> {
    if (this.busy || this.state === null) return;
    const ticket = ++this.latest;
    this.busy = true;
    this.show(this.state, true);
    const state = await switchAutostart(this.call, want);
    if (ticket === this.latest) this.apply(state);
  }

  private apply(state: AutostartState) {
    this.state = state;
    this.busy = false;
    this.show(state, false);
  }
}
