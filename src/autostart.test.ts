import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AutostartSwitch, readAutostart, switchAutostart, type AutostartState, type Call } from "./autostart";

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
});
afterEach(() => {
  vi.restoreAllMocks();
});

/** A call whose answers the test hands out one by one, in any order. */
function deferredCall() {
  const pending: { cmd: string; args?: Record<string, unknown>; resolve: (v: unknown) => void; reject: (e: unknown) => void }[] = [];
  const call: Call = (cmd, args) => new Promise((resolve, reject) => pending.push({ cmd, args, resolve, reject }));
  return { call, pending };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("start with Windows", () => {
  it("reads the state from the app side", async () => {
    const call = vi.fn<Call>(async () => true);
    expect(await readAutostart(call)).toBe(true);
    expect(call).toHaveBeenCalledWith("autostart_enabled");
  });

  it("a state that cannot be read is neither on nor off", async () => {
    expect(await readAutostart(async () => { throw new Error("x"); })).toBeNull();
    expect(await readAutostart(async () => "yes")).toBeNull();
  });

  it("shows what the app side read back after switching", async () => {
    const call = vi.fn<Call>(async () => true);
    expect(await switchAutostart(call, true)).toBe(true);
    expect(call).toHaveBeenCalledWith("set_autostart", { enabled: true });
    // One step failed on the app side: it reports the state it was left in.
    expect(await switchAutostart(async () => false, true)).toBe(false);
  });

  it("reads the state again when switching fails, instead of guessing", async () => {
    const calls: string[] = [];
    const call: Call = async (cmd) => {
      calls.push(cmd);
      if (cmd === "set_autostart") throw new Error("denied");
      return true; // Run was written before the failure
    };
    expect(await switchAutostart(call, true)).toBe(true);
    expect(calls).toEqual(["set_autostart", "autostart_enabled"]);
    expect(await switchAutostart(async () => { throw new Error("x"); }, true)).toBeNull();
  });
});

describe("the switch in the settings menu", () => {
  it("stays untouchable while the state cannot be read", async () => {
    const shown: [AutostartState, boolean][] = [];
    const call = vi.fn<Call>(async () => { throw new Error("x"); });
    const sw = new AutostartSwitch(call, (s, b) => shown.push([s, b]));
    await sw.refresh();
    expect(sw.state).toBeNull();
    await sw.change(true);
    expect(call).not.toHaveBeenCalledWith("set_autostart", expect.anything());
  });

  it("an older read that answers late does not overwrite a newer change", async () => {
    const { call, pending } = deferredCall();
    const shown: [AutostartState, boolean][] = [];
    const sw = new AutostartSwitch(call, (s, b) => shown.push([s, b]));
    sw.state = false;
    const read = sw.refresh(); // menu opened
    const change = sw.change(true); // switched before the read answered
    await tick();
    expect(pending.map((p) => p.cmd)).toEqual(["autostart_enabled", "set_autostart"]);
    pending[1].resolve(true); // the change answers first
    await change;
    pending[0].resolve(false); // then the old read
    await read;
    expect(sw.state).toBe(true);
    expect(shown.at(-1)).toEqual([true, false]);
  });

  it("does not send a second change while one is on its way", async () => {
    const { call, pending } = deferredCall();
    const sw = new AutostartSwitch(call, () => {});
    sw.state = false;
    const first = sw.change(true);
    await sw.change(false);
    await tick();
    expect(pending.length).toBe(1);
    pending[0].resolve(true);
    await first;
    expect(sw.state).toBe(true);
    expect(sw.busy).toBe(false);
  });
});
