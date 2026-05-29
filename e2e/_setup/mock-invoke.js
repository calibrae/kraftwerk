/**
 * Tauri invoke + event shims for the E2E suite.
 *
 * Tauri v2 exposes `__TAURI_INTERNALS__` on `window` with the IPC
 * surface that `@tauri-apps/api/core::invoke` calls into. When we
 * run in plain chromium (no Tauri runtime present), that global is
 * undefined and every `invoke()` throws — which is exactly the
 * blank-UI failure mode we just shipped twice.
 *
 * This shim installs a controllable invoke. By default every command
 * returns a sensible empty value (`[]` for `list_*`, `null` for
 * everything else). Tests can override per-command behaviour via
 * `window.__kraftwerkMock.command(name, handler)` (set up by the
 * test fixtures).
 *
 * Event listeners (`@tauri-apps/api/event::listen`) likewise need to
 * be neutered to return a no-op unsubscriber.
 *
 * This file is loaded via `addInitScript` BEFORE the page navigates,
 * so the Tauri JS wrapper sees the shim instead of `undefined`.
 */

// eslint-disable-next-line no-undef
(() => {
  if (typeof window === "undefined") return;

  const handlers = new Map();
  const calls = [];

  function defaultReturn(cmd) {
    // Most list_* commands should return [] so .filter / .map work.
    if (cmd.startsWith("list_")) return [];
    // get_* commands typically return an object; null is the safest
    // default and the UI tends to render "loading…" or skip gracefully.
    return null;
  }

  async function invoke(cmd, args) {
    calls.push({ cmd, args });
    const h = handlers.get(cmd);
    if (h) return h(args ?? {});
    return defaultReturn(cmd);
  }

  // Tauri v2's IPC entry point.
  window.__TAURI_INTERNALS__ = {
    invoke,
    metadata: { plugins: {} },
    transformCallback: (cb) => cb,
    convertFileSrc: (p) => p,
  };

  // Event subscription shim — returns a no-op unsubscriber.
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: () => {},
  };

  // Public test API — tests poke at this from `page.evaluate`.
  window.__kraftwerkMock = {
    command(name, handler) { handlers.set(name, handler); },
    reset() { handlers.clear(); calls.length = 0; },
    calls() { return calls.slice(); },
    lastCall(name) {
      for (let i = calls.length - 1; i >= 0; i--) {
        if (calls[i].cmd === name) return calls[i];
      }
      return null;
    },
  };
})();
