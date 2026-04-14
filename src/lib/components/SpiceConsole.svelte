<script>
  /*
   * SPICE console rendered via capsaicin-client in the Rust backend.
   *
   * Event flow:
   *   Rust capsaicin SpiceClient -> `spice:event` Tauri events -> queue
   *   rAF tick drains the queue and issues one batched paint per frame.
   *
   * See capsaicin/inbox/virtmanager-rs-display-events.md for the full
   * handling checklist. TL;DR:
   *   - SurfaceCreated allocates a persistent framebuffer (with size cap)
   *   - Region{Raw} honours stride (rows may be padded)
   *   - Region{SolidColor} is a per-row fill
   *   - StreamFrame blits identically to Region{Raw}
   *   - CopyRect handles overlapping src/dest (canvas drawImage from
   *     same canvas is spec-guaranteed memmove-safe)
   *   - Mark is a present trigger (we rAF-batch instead)
   *   - Reset wipes the framebuffer
   */
  import { onMount, onDestroy, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  let { vmName, onClose } = $props();

  // Hostile-server guard: cap at 16384×16384 / 256 MiB (same as capsaicin-cli).
  const MAX_DIM = 16384;
  const MAX_PIXELS = 64 * 1024 * 1024; // 256 MiB of BGRA

  let canvasEl = $state(null);
  let wrapperEl = $state(null);
  let connected = $state(false);
  let error = $state(null);
  let surfaceInfo = $state(null);  // {width, height, format}
  let streamCount = $state(0);
  let ctx = null;
  let unlisten = null;
  let aborted = false;

  // Auth prompt state: shown when the SPICE server rejects our ticket.
  let needPassword = $state(false);
  let passwordInput = $state("");
  let attempted = $state(false);

  // Coalescing: events accumulate in this queue, a single rAF flushes.
  const eventQueue = [];
  let rafHandle = null;
  // Exposed for future debugging
  let debugLog = false;

  onMount(async () => {
    unlisten = await listen("spice:event", (e) => enqueue(e.payload));
    await connectSpice(null);
  });

  async function connectSpice(password) {
    attempted = true;
    error = null;
    try {
      await invoke("open_spice", { name: vmName, password });
      connected = true;
      needPassword = false;
    } catch (e) {
      // Backend error payload has shape { code, message, suggestion }.
      if (e && e.code === "spice_auth_required") {
        needPassword = true;
        // Keep any previously-typed password so "try again" after a typo is quick.
        error = passwordInput ? "Wrong password — try again." : null;
      } else {
        error = e?.message || JSON.stringify(e);
      }
      connected = false;
    }
  }

  async function submitPassword(ev) {
    ev?.preventDefault?.();
    await connectSpice(passwordInput);
    if (connected) passwordInput = ""; // don't retain the secret once accepted
  }

  onDestroy(async () => {
    aborted = true;
    if (rafHandle) cancelAnimationFrame(rafHandle);
    if (unlisten) unlisten();
    try { await invoke("close_spice"); } catch (_) {}
  });

  function enqueue(evt) {
    if (aborted) return;
    eventQueue.push(evt);
    if (rafHandle == null) {
      rafHandle = requestAnimationFrame(flush);
    }
  }

  async function flush() {
    rafHandle = null;
    const batch = eventQueue.splice(0, eventQueue.length);
    for (const evt of batch) {
      await handleEvent(evt);
    }
    // If events arrived while we were handling the batch, schedule next tick.
    if (eventQueue.length > 0 && rafHandle == null) {
      rafHandle = requestAnimationFrame(flush);
    }
  }

  async function handleEvent(e) {
    if (debugLog) console.debug("[spice]", e.kind, e);
    switch (e.kind) {
      case "surface_created":
        await handleSurfaceCreated(e);
        break;

      case "surface_destroyed":
        // Only matters for non-primary; nothing to do for now.
        break;

      case "region":
        if (!ctx) return;
        paintRegion(e.rect, e.pixels, e.format);
        break;

      case "copy_rect":
        if (!ctx) return;
        paintCopyRect(e);
        break;

      case "stream_created":
        streamCount++;
        break;

      case "stream_destroyed":
        streamCount = Math.max(0, streamCount - 1);
        break;

      case "stream_frame":
        if (!ctx) return;
        // Per capsaicin contract: BGRA, top-down, stride = width*4.
        paintRegion(e.destRect, e.pixels, "xrgb8888");
        break;

      case "reset":
        // Server dropped all cache state. Clear our framebuffer so we
        // don't composite stale pixels while waiting for fresh draws.
        if (ctx && canvasEl) {
          ctx.fillStyle = "#000";
          ctx.fillRect(0, 0, canvasEl.width, canvasEl.height);
        }
        break;

      case "mark":
        // Browser canvas presents lazily via rAF anyway; nothing to flush
        // beyond the batching we already do. Left as a hook.
        break;

      case "mode":
        // Legacy mode hint. SurfaceCreated is authoritative; ignore.
        break;

      case "closed":
        error = e.reason ?? "SPICE session closed";
        connected = false;
        break;
    }
  }

  async function handleSurfaceCreated(e) {
    if (!e.primary) {
      // Secondary surface: tolerated miss. capsaicin will still drive the
      // primary correctly; any draws into the secondary surface simply
      // won't show up on our canvas. Acceptable per the contract.
      return;
    }
    // Hostile-server guard.
    const pixels = e.width * e.height;
    if (e.width === 0 || e.height === 0 ||
        e.width > MAX_DIM || e.height > MAX_DIM ||
        pixels > MAX_PIXELS) {
      error = `Server reported unreasonable surface size ${e.width}×${e.height}`;
      return;
    }
    surfaceInfo = { width: e.width, height: e.height, format: e.format };
    await tick();
    if (!canvasEl) return;
    canvasEl.width = e.width;
    canvasEl.height = e.height;
    ctx = canvasEl.getContext("2d", { alpha: false });
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, e.width, e.height);
  }

  function decodeBase64(b64) {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }

  function paintRegion(rect, pixels, format) {
    const w = rect.width, h = rect.height;
    if (w <= 0 || h <= 0) return;

    if (pixels.kind === "solid_color") {
      // argb packed as 32-bit AARRGGBB. Alpha ignored for XRGB surfaces.
      const argb = pixels.argb >>> 0;
      const r = (argb >> 16) & 0xff;
      const g = (argb >> 8) & 0xff;
      const b = argb & 0xff;
      ctx.fillStyle = `rgb(${r},${g},${b})`;
      ctx.fillRect(rect.left, rect.top, w, h);
      return;
    }

    // Raw path. SPICE XRGB8888 wire format is little-endian 0x00RRGGBB →
    // bytes (B, G, R, X). Canvas ImageData expects RGBA bytes.
    const src = decodeBase64(pixels.data_b64);
    const stride = pixels.stride;
    const image = ctx.createImageData(w, h);
    const dst = image.data;

    if (format === "xrgb8888" || format === "argb8888") {
      const forceAlpha = format === "xrgb8888";
      for (let y = 0; y < h; y++) {
        const srcOff = y * stride;
        const dstOff = y * w * 4;
        for (let x = 0; x < w; x++) {
          const si = srcOff + x * 4;
          const di = dstOff + x * 4;
          dst[di] = src[si + 2];       // R
          dst[di + 1] = src[si + 1];   // G
          dst[di + 2] = src[si];       // B (SPICE wire is B,G,R,A little-endian)
          dst[di + 3] = forceAlpha ? 255 : src[si + 3];
        }
      }
    } else {
      // Unknown format: paint a red stripe so the bug is visible.
      for (let i = 0; i < dst.length; i += 4) {
        dst[i] = 255; dst[i+1] = 0; dst[i+2] = 0; dst[i+3] = 255;
      }
    }

    ctx.putImageData(image, rect.left, rect.top);
  }

  function paintCopyRect(e) {
    const { srcX, srcY, destRect } = e;
    const w = destRect.width, h = destRect.height;
    if (w <= 0 || h <= 0) return;
    // Canvas spec: drawImage from the same canvas reads through an
    // internal intermediate buffer, so overlapping src/dest is safe
    // (memmove semantics). Verified in Firefox/Chromium/WebKit.
    ctx.drawImage(canvasEl, srcX, srcY, w, h, destRect.left, destRect.top, w, h);
  }

  // ── Input: keyboard ───────────────────────────────────────────────────
  //
  // Map browser KeyboardEvent.code -> PC AT set-1 scancode. Extended keys
  // are prefixed with 0xE0 → packed as `(0xE0 << 8) | code`.

  const KEY_MAP = {
    Escape: 0x01, Digit1: 0x02, Digit2: 0x03, Digit3: 0x04, Digit4: 0x05,
    Digit5: 0x06, Digit6: 0x07, Digit7: 0x08, Digit8: 0x09, Digit9: 0x0a,
    Digit0: 0x0b, Minus: 0x0c, Equal: 0x0d, Backspace: 0x0e, Tab: 0x0f,
    KeyQ: 0x10, KeyW: 0x11, KeyE: 0x12, KeyR: 0x13, KeyT: 0x14, KeyY: 0x15,
    KeyU: 0x16, KeyI: 0x17, KeyO: 0x18, KeyP: 0x19, BracketLeft: 0x1a,
    BracketRight: 0x1b, Enter: 0x1c, ControlLeft: 0x1d,
    KeyA: 0x1e, KeyS: 0x1f, KeyD: 0x20, KeyF: 0x21, KeyG: 0x22, KeyH: 0x23,
    KeyJ: 0x24, KeyK: 0x25, KeyL: 0x26, Semicolon: 0x27, Quote: 0x28,
    Backquote: 0x29, ShiftLeft: 0x2a, Backslash: 0x2b,
    KeyZ: 0x2c, KeyX: 0x2d, KeyC: 0x2e, KeyV: 0x2f, KeyB: 0x30, KeyN: 0x31,
    KeyM: 0x32, Comma: 0x33, Period: 0x34, Slash: 0x35, ShiftRight: 0x36,
    NumpadMultiply: 0x37, AltLeft: 0x38, Space: 0x39, CapsLock: 0x3a,
    F1: 0x3b, F2: 0x3c, F3: 0x3d, F4: 0x3e, F5: 0x3f, F6: 0x40, F7: 0x41,
    F8: 0x42, F9: 0x43, F10: 0x44, NumLock: 0x45, ScrollLock: 0x46,
    Numpad7: 0x47, Numpad8: 0x48, Numpad9: 0x49, NumpadSubtract: 0x4a,
    Numpad4: 0x4b, Numpad5: 0x4c, Numpad6: 0x4d, NumpadAdd: 0x4e,
    Numpad1: 0x4f, Numpad2: 0x50, Numpad3: 0x51, Numpad0: 0x52,
    NumpadDecimal: 0x53, F11: 0x57, F12: 0x58,
    NumpadEnter: 0xe01c, ControlRight: 0xe01d, NumpadDivide: 0xe035,
    AltRight: 0xe038, Home: 0xe047, ArrowUp: 0xe048, PageUp: 0xe049,
    ArrowLeft: 0xe04b, ArrowRight: 0xe04d, End: 0xe04f, ArrowDown: 0xe050,
    PageDown: 0xe051, Insert: 0xe052, Delete: 0xe053,
    MetaLeft: 0xe05b, MetaRight: 0xe05c,
  };

  function keyHandler(down) {
    return async (ev) => {
      const code = KEY_MAP[ev.code];
      if (code == null) return;
      ev.preventDefault();
      try {
        await invoke("spice_input", {
          event: { kind: down ? "key_down" : "key_up", scancode: code },
        });
      } catch (_) {}
    };
  }

  let buttonsMask = 0;

  function browserToSpiceButton(b) {
    // DOM button: 0=left, 1=middle, 2=right
    return b === 0 ? 1 : b === 1 ? 2 : b === 2 ? 3 : 0;
  }

  function canvasCoords(ev) {
    if (!canvasEl || !surfaceInfo) return null;
    const rect = canvasEl.getBoundingClientRect();
    const scaleX = surfaceInfo.width / rect.width;
    const scaleY = surfaceInfo.height / rect.height;
    const x = Math.round((ev.clientX - rect.left) * scaleX);
    const y = Math.round((ev.clientY - rect.top) * scaleY);
    if (x < 0 || y < 0 || x >= surfaceInfo.width || y >= surfaceInfo.height) return null;
    return { x, y };
  }

  async function mouseMove(ev) {
    const pos = canvasCoords(ev);
    if (!pos) return;
    try {
      await invoke("spice_input", {
        event: { kind: "mouse_position", x: pos.x, y: pos.y, buttons: buttonsMask },
      });
    } catch (_) {}
  }

  async function mouseDown(ev) {
    const button = browserToSpiceButton(ev.button);
    if (button === 0) return;
    buttonsMask |= 1 << (button - 1);
    ev.preventDefault();
    try {
      await invoke("spice_input", {
        event: { kind: "mouse_press", button, buttons: buttonsMask },
      });
    } catch (_) {}
  }

  async function mouseUp(ev) {
    const button = browserToSpiceButton(ev.button);
    if (button === 0) return;
    buttonsMask &= ~(1 << (button - 1));
    try {
      await invoke("spice_input", {
        event: { kind: "mouse_release", button, buttons: buttonsMask },
      });
    } catch (_) {}
  }

  async function wheel(ev) {
    ev.preventDefault();
    const button = ev.deltaY < 0 ? 4 : 5;
    try {
      await invoke("spice_input", { event: { kind: "mouse_press", button, buttons: buttonsMask } });
      await invoke("spice_input", { event: { kind: "mouse_release", button, buttons: buttonsMask } });
    } catch (_) {}
  }

  function focus() { wrapperEl?.focus(); }
</script>

<div class="spice-container">
  <div class="toolbar">
    <span class="title">
      SPICE Console — {vmName}
      {#if connected && !error}
        <span class="badge connected">Connected</span>
      {:else if error}
        <span class="badge err">Error</span>
      {:else}
        <span class="badge connecting">Connecting...</span>
      {/if}
      {#if surfaceInfo}
        <span class="meta">{surfaceInfo.width}×{surfaceInfo.height} · {surfaceInfo.format}</span>
      {/if}
      {#if streamCount > 0}
        <span class="meta">{streamCount} stream{streamCount === 1 ? "" : "s"}</span>
      {/if}
    </span>
    <div class="actions">
      <button class="btn" onclick={focus}>Focus</button>
      <button class="btn btn-close" onclick={onClose}>Disconnect</button>
    </div>
  </div>

  {#if error}
    <div class="err-banner">{error}</div>
  {/if}

  {#if needPassword && !connected}
    <form class="password-prompt" onsubmit={submitPassword}>
      <label>
        <span>SPICE password</span>
        <input
          type="password"
          bind:value={passwordInput}
          placeholder="Enter the VM's SPICE password"
          autocomplete="off"
          autofocus
        />
      </label>
      <button type="submit" class="btn btn-primary" disabled={!passwordInput}>Connect</button>
    </form>
  {/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="canvas-wrap"
    bind:this={wrapperEl}
    tabindex="0"
    onkeydown={keyHandler(true)}
    onkeyup={keyHandler(false)}
    onmousemove={mouseMove}
    onmousedown={mouseDown}
    onmouseup={mouseUp}
    onwheel={wheel}
    oncontextmenu={(e) => e.preventDefault()}
  >
    <canvas bind:this={canvasEl}></canvas>
  </div>
</div>

<style>
  .spice-container { display: flex; flex-direction: column; height: 100%; background: #000; }
  .toolbar {
    display: flex; justify-content: space-between; align-items: center;
    padding: 8px 16px; background: var(--bg-surface);
    border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .title { font-size: 13px; font-weight: 500; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .meta { font-size: 11px; color: var(--text-muted); font-family: 'SF Mono', monospace; }

  .badge { display: inline-block; padding: 1px 8px; border-radius: 10px; font-size: 11px; }
  .connected { background: rgba(52, 211, 153, 0.15); color: #34d399; }
  .connecting { background: rgba(251, 191, 36, 0.15); color: #fbbf24; }
  .err { background: rgba(239, 68, 68, 0.15); color: #ef4444; }

  .actions { display: flex; gap: 6px; }
  .btn {
    padding: 4px 12px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); cursor: pointer;
    font-size: 12px; font-family: inherit;
  }
  .btn:hover { background: var(--bg-hover); }
  .btn-close:hover { background: #7f1d1d; color: #fca5a5; border-color: #7f1d1d; }

  .err-banner {
    padding: 8px 16px; background: rgba(239, 68, 68, 0.1);
    border-bottom: 1px solid rgba(239, 68, 68, 0.3);
    color: #ef4444; font-size: 12px; flex-shrink: 0;
  }

  .password-prompt {
    padding: 16px; background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    display: flex; gap: 12px; align-items: flex-end; flex-shrink: 0;
  }
  .password-prompt label {
    display: flex; flex-direction: column; gap: 4px; flex: 1;
  }
  .password-prompt label span {
    font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em;
  }
  .password-prompt input {
    padding: 7px 10px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-input); color: var(--text); font-size: 13px; font-family: inherit; outline: none;
  }
  .password-prompt input:focus {
    border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-dim);
  }
  .btn-primary {
    padding: 7px 14px; border: 1px solid var(--accent); border-radius: 6px;
    background: var(--accent); color: white; font-size: 13px; font-family: inherit; cursor: pointer;
  }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

  .canvas-wrap {
    flex: 1; overflow: hidden; background: #000;
    display: flex; align-items: center; justify-content: center;
    outline: none;
  }
  .canvas-wrap:focus { outline: 1px solid var(--accent-dim); outline-offset: -1px; }
  canvas {
    max-width: 100%; max-height: 100%;
    image-rendering: crisp-edges;
    cursor: crosshair;
  }
</style>
