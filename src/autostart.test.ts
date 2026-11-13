import { describe, expect, it, vi } from "vitest";
import { readAutostart, switchAutostart, type Call } from "./autostart";

describe("start with Windows", () => {
  it("reads the state from the app side", async () => {
    const call = vi.fn<Call>(async () => true);
    expect(await readAutostart(call)).toBe(true);
    expect(call).toHaveBeenCalledWith("autostart_enabled");
  });

  it("shows off when the state cannot be read", async () => {
    expect(await readAutostart(async () => { throw new Error("x"); })).toBe(false);
    expect(await readAutostart(async () => "yes")).toBe(false);
  });

  it("sends the wanted state and shows what the app side read back", async () => {
    const call = vi.fn<Call>(async () => true);
    expect(await switchAutostart(call, true, false)).toBe(true);
    expect(call).toHaveBeenCalledWith("set_autostart", { enabled: true });
    // The app side may report a different state than asked for.
    expect(await switchAutostart(async () => false, true, false)).toBe(false);
  });

  it("goes back to the earlier state when the change fails", async () => {
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(await switchAutostart(async () => { throw new Error("denied"); }, true, false)).toBe(false);
    expect(await switchAutostart(async () => { throw new Error("denied"); }, false, true)).toBe(true);
    expect(await switchAutostart(async () => undefined, false, true)).toBe(true);
    err.mockRestore();
  });
});
