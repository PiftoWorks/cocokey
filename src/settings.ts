/** Settings kept in the window's local storage, as JSON under "cocokey.<key>". */
export interface Store {
  get<T>(key: string, fallback: T): T;
  set(key: string, value: unknown): void;
}

type Storage = Pick<globalThis.Storage, "getItem" | "setItem">;

const PREFIX = "cocokey.";

/**
 * `storage` is looked up on every call: reading `localStorage` itself can
 * throw (storage disabled), and then settings fall back to their defaults.
 */
export function createStore(storage: () => Storage): Store {
  return {
    get<T>(key: string, fallback: T): T {
      try {
        const v = storage().getItem(PREFIX + key);
        return v === null ? fallback : (JSON.parse(v) as T);
      } catch {
        return fallback;
      }
    },
    set(key: string, value: unknown) {
      try {
        storage().setItem(PREFIX + key, JSON.stringify(value));
      } catch {
        /* storage unavailable: settings just won't persist */
      }
    },
  };
}
