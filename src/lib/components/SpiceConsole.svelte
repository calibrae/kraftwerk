<script>
  /*
   * SPICE console rendered via capsaicin-client in the Rust backend.
   *
   * Mouse: server-advertised mode drives the input path.
   *   - CLIENT mode: send absolute InputEvent::MousePosition in canvas
   *     coords. Host OS cursor is the real one; we still composite the
   *     guest cursor sprite for debugging parity? No — we skip it in
   *     CLIENT mode to avoid a double-cursor.
   *   - SERVER mode: pointer-lock grab + relative MouseMotion from
   *     movementX/Y. Cursor channel sprite is composited on top because
   *     the host cursor is locked/hidden.
   *
   * Release combo (SERVER mode only): Ctrl+Alt+Shift (three modifiers).
   * Browser-enforced ESC also releases but we don't advertise it.
   */
  import { onMount, onDestroy, tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  let { vmName, onClose } = $props();

  const MAX_DIM = 16384;
  const MAX_PIXELS = 64 * 1024 * 1024;

  let canvasEl = $state(null);     // framebuffer
  let cursorEl = $state(null);     // cursor overlay canvas
  let wrapperEl = $state(null);
  let connected = $state(false);
  let error = $state(null);
  let surfaceInfo = $state(null);
  let streamCount = $state(0);
  let ctx = null;
  let cursorCtx = null;
  let unlisten = null;
  let aborted = false;

  let needPassword = $state(false);
  let passwordInput = $state("");

  // Mouse mode drives the input strategy.
  let mouseMode = $state("server"); // "client" | "server"

  // Focus / grab (only meaningful in SERVER mode).
  let hasFocus = $state(false);
  let grabbed = $state(false);

  // Cursor state.
  const cursorCache = new Map(); // unique (string) -> { w, h, hotX, hotY, pixels: Uint8ClampedArray (RGBA) }
  let cursorShown = $state(false);
  let cursorHotX = $state(0);
  let cursorHotY = $state(0);
  let cursorX = $state(0);
  let cursorY = $state(0);

  // Diagnostics.
  let inputSent = $state(0);
  let displayRecv = $state(0);

  const eventQueue = [];
  let rafHandle = null;

  onMount(async () => {
    unlisten = await listen("spice:event", (e) => enqueue(e.payload));
    document.addEventListener("pointerlockchange", onPointerLockChange);
    document.addEventListener("pointerlockerror", onPointerLockError);
    await connectSpice(null);
  });

  onDestroy(async () => {
    aborted = true;
    if (rafHandle) cancelAnimationFrame(rafHandle);
    if (unlisten) unlisten();
    document.removeEventListener("pointerlockchange", onPointerLockChange);
    document.removeEventListener("pointerlockerror", onPointerLockError);
    if (document.pointerLockElement) document.exitPointerLock();
    try { await invoke("close_spice"); } catch (_) {}
  });

  async function connectSpice(password) {
    error = null;
    try {
      await invoke("open_spice", { name: vmName, password });
      connected = true;
      needPassword = false;
    } catch (e) {
      if (e && e.code === "spice_auth_required") {
        needPassword = true;
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
    if (connected) passwordInput = "";
  }

  function enqueue(evt) {
    if (aborted) return;
    eventQueue.push(evt);
    if (rafHandle == null) rafHandle = requestAnimationFrame(flush);
  }

  async function flush() {
    rafHandle = null;
    const batch = eventQueue.splice(0, eventQueue.length);
    for (const evt of batch) await handleEvent(evt);
    if (eventQueue.length > 0 && rafHandle == null) rafHandle = requestAnimationFrame(flush);
  }

  async function handleEvent(e) {
    switch (e.kind) {
      // ── Display ────────────────────────────────────────────────────
      case "surface_created": await handleSurfaceCreated(e); break;
      case "surface_destroyed": break;
      case "region":     if (ctx) { paintRegion(e.rect, e.pixels, e.format); displayRecv++; } break;
      case "copy_rect":  if (ctx) { paintCopyRect(e); displayRecv++; } break;
      case "stream_created":  streamCount++; break;
      case "stream_destroyed": streamCount = Math.max(0, streamCount - 1); break;
      case "stream_frame": if (ctx) { paintRegion(e.destRect, e.pixels, "xrgb8888"); displayRecv++; } break;
      case "reset":
        if (ctx && canvasEl) { ctx.fillStyle = "#000"; ctx.fillRect(0, 0, canvasEl.width, canvasEl.height); }
        break;
      case "mark": case "mode": break;

      // ── Mouse mode ─────────────────────────────────────────────────
      case "mouse_mode":
        mouseMode = e.mode;
        // When switching to CLIENT mode, release any pointer lock so
        // the host OS cursor comes back naturally.
        if (mouseMode === "client" && document.pointerLockElement) {
          document.exitPointerLock();
        }
        // And wipe our cursor overlay since we won't composite in client mode.
        if (mouseMode === "client") clearCursorOverlay();
        break;

      // ── Cursor channel ─────────────────────────────────────────────
      case "cursor_set": handleCursorSet(e); break;
      case "cursor_set_from_cache": handleCursorSetFromCache(e); break;
      case "cursor_move":
        cursorX = e.x; cursorY = e.y;
        repaintCursor();
        break;
      case "cursor_hide":
        cursorShown = false;
        clearCursorOverlay();
        break;
      case "cursor_invalidate_one":
        cursorCache.delete(String(e.unique));
        break;
      case "cursor_invalidate_all":
        cursorCache.clear();
        break;

      case "closed":
        error = e.reason ?? "SPICE session closed";
        connected = false;
        if (document.pointerLockElement) document.exitPointerLock();
        break;
    }
  }

  async function handleSurfaceCreated(e) {
    if (!e.primary) return;
    const pixels = e.width * e.height;
    if (e.width === 0 || e.height === 0 || e.width > MAX_DIM || e.height > MAX_DIM || pixels > MAX_PIXELS) {
      error = `Server reported unreasonable surface size ${e.width}×${e.height}`;
      return;
    }
    surfaceInfo = { width: e.width, height: e.height, format: e.format };
    await tick();
    if (!canvasEl || !cursorEl) return;
    canvasEl.width = e.width;
    canvasEl.height = e.height;
    cursorEl.width = e.width;
    cursorEl.height = e.height;
    ctx = canvasEl.getContext("2d", { alpha: false });
    cursorCtx = cursorEl.getContext("2d", { alpha: true });
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, e.width, e.height);
    cursorCtx.clearRect(0, 0, e.width, e.height);
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
      const argb = pixels.argb >>> 0;
      ctx.fillStyle = `rgb(${(argb >> 16) & 0xff},${(argb >> 8) & 0xff},${argb & 0xff})`;
      ctx.fillRect(rect.left, rect.top, w, h);
      return;
    }
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
          dst[di] = src[si + 2];
          dst[di + 1] = src[si + 1];
          dst[di + 2] = src[si];
          dst[di + 3] = forceAlpha ? 255 : src[si + 3];
        }
      }
    } else {
      for (let i = 0; i < dst.length; i += 4) { dst[i] = 255; dst[i+3] = 255; }
    }
    ctx.putImageData(image, rect.left, rect.top);
  }

  function paintCopyRect(e) {
    const { srcX, srcY, destRect } = e;
    const w = destRect.width, h = destRect.height;
    if (w <= 0 || h <= 0) return;
    ctx.drawImage(canvasEl, srcX, srcY, w, h, destRect.left, destRect.top, w, h);
  }

  // ── Cursor channel handling ──────────────────────────────────────────
  //
  // Capsaicin delivers sprite pixels as ARGB8888 top-down with
  // stride = width*4. We convert once into Canvas RGBA (byte-swap) and
  // store in the cache keyed by the unique id (string — u64 doesn't fit
  // in JS number). Rendering = putImageData at (x - hot_x, y - hot_y).

  function argbToCanvasRgba(src, width, height) {
    const dst = new Uint8ClampedArray(width * height * 4);
    for (let i = 0, n = dst.length; i < n; i += 4) {
      // src ARGB8888 little-endian on wire = bytes B,G,R,A
      dst[i]     = src[i + 2];
      dst[i + 1] = src[i + 1];
      dst[i + 2] = src[i];
      dst[i + 3] = src[i + 3];
    }
    return dst;
  }

  function handleCursorSet(e) {
    const unique = String(e.unique);
    let sprite = null;
    if (e.pixelsB64 && e.width > 0 && e.height > 0) {
      const raw = decodeBase64(e.pixelsB64);
      if (raw.length >= e.width * e.height * 4) {
        sprite = {
          w: e.width, h: e.height,
          hotX: e.hotX, hotY: e.hotY,
          pixels: argbToCanvasRgba(raw, e.width, e.height),
        };
        if (e.cacheable) cursorCache.set(unique, sprite);
      }
    }
    if (sprite) setActiveCursor(sprite, e.x, e.y, e.visible);
    else {
      cursorShown = e.visible && false; // no decodable sprite = hidden
      clearCursorOverlay();
    }
  }

  function handleCursorSetFromCache(e) {
    const sprite = cursorCache.get(String(e.unique));
    if (!sprite) return; // cache miss — swallow silently
    setActiveCursor(sprite, e.x, e.y, e.visible);
  }

  let activeSprite = null;

  function setActiveCursor(sprite, x, y, visible) {
    activeSprite = sprite;
    cursorHotX = sprite.hotX;
    cursorHotY = sprite.hotY;
    cursorX = x; cursorY = y;
    cursorShown = visible;
    repaintCursor();
  }

  function clearCursorOverlay() {
    if (cursorCtx && cursorEl) {
      cursorCtx.clearRect(0, 0, cursorEl.width, cursorEl.height);
    }
  }

  function repaintCursor() {
    if (!cursorCtx || !cursorEl) return;
    cursorCtx.clearRect(0, 0, cursorEl.width, cursorEl.height);
    if (mouseMode === "client") return; // host OS draws the real cursor
    if (!cursorShown || !activeSprite) return;
    const { w, h, pixels } = activeSprite;
    const dx = cursorX - cursorHotX;
    const dy = cursorY - cursorHotY;
    // Build a one-off ImageData and blit.
    const img = new ImageData(new Uint8ClampedArray(pixels), w, h);
    cursorCtx.putImageData(img, dx, dy);
  }

  // ── Focus / pointer lock ─────────────────────────────────────────────

  function onPointerLockChange() {
    grabbed = document.pointerLockElement === wrapperEl;
  }
  function onPointerLockError() {
    error = "Could not grab pointer — the window may need focus first.";
  }

  async function grab() {
    if (!wrapperEl) return;
    wrapperEl.focus();
    try { await wrapperEl.requestPointerLock(); } catch (_) {}
  }
  function release() {
    if (document.pointerLockElement) document.exitPointerLock();
  }

  // ── Input helpers ────────────────────────────────────────────────────

  async function sendInput(event) {
    inputSent++;
    try { await invoke("spice_input", { event }); } catch (_) {}
  }

  // ── Keyboard ─────────────────────────────────────────────────────────

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
      // SERVER-mode only: release combo intercept.
      if (down && mouseMode === "server" && grabbed &&
          ev.ctrlKey && ev.altKey && ev.shiftKey) {
        ev.preventDefault();
        release();
        return;
      }
      const code = KEY_MAP[ev.code];
      if (code == null) return;
      ev.preventDefault();
      await sendInput({ kind: down ? "key_down" : "key_up", scancode: code });
    };
  }

  // ── Mouse (mode-aware) ───────────────────────────────────────────────

  let buttonsMask = 0;

  function browserToSpiceButton(b) { return b === 0 ? 1 : b === 1 ? 2 : b === 2 ? 3 : 0; }

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
    if (mouseMode === "client") {
      const pos = canvasCoords(ev);
      if (!pos) return;
      await sendInput({ kind: "mouse_position", x: pos.x, y: pos.y, buttons: buttonsMask });
    } else {
      if (!grabbed) return;
      const dx = ev.movementX | 0;
      const dy = ev.movementY | 0;
      if (dx === 0 && dy === 0) return;
      await sendInput({ kind: "mouse_motion", dx, dy, buttons: buttonsMask });
    }
  }

  async function mouseDown(ev) {
    const button = browserToSpiceButton(ev.button);
    if (button === 0) return;
    if (mouseMode === "server" && connected && !grabbed) {
      ev.preventDefault();
      await grab();
      return;
    }
    buttonsMask |= 1 << (button - 1);
    ev.preventDefault();
    await sendInput({ kind: "mouse_press", button, buttons: buttonsMask });
  }

  async function mouseUp(ev) {
    if (mouseMode === "server" && !grabbed) return;
    const button = browserToSpiceButton(ev.button);
    if (button === 0) return;
    buttonsMask &= ~(1 << (button - 1));
    await sendInput({ kind: "mouse_release", button, buttons: buttonsMask });
  }

  async function wheel(ev) {
    if (mouseMode === "server" && !grabbed) return;
    ev.preventDefault();
    const button = ev.deltaY < 0 ? 4 : 5;
    await sendInput({ kind: "mouse_press", button, buttons: buttonsMask });
    await sendInput({ kind: "mouse_release", button, buttons: buttonsMask });
  }

  function onWrapperFocus() { hasFocus = true; }
  function onWrapperBlur()  { hasFocus = false; }

  let statusLabel = $derived(
    !connected ? "Disconnected" :
    mouseMode === "client" ? "Client mode · single cursor" :
    grabbed    ? "Grabbed · Ctrl+Alt+Shift to release" :
    hasFocus   ? "Focused · click to grab" :
                 "Not focused · click to interact"
  );
  let statusTone = $derived(
    !connected ? "err" :
    mouseMode === "client" ? "grabbed" :
    grabbed    ? "grabbed" :
    hasFocus   ? "focused" : "idle"
  );
</script>

<div class="spice-container">
  <div class="toolbar">
    <span class="title">
      SPICE Console — {vmName}
      <span class="badge {statusTone}">{statusLabel}</span>
      {#if surfaceInfo}
        <span class="meta">{surfaceInfo.width}×{surfaceInfo.height} · {surfaceInfo.format}</span>
      {/if}
      {#if streamCount > 0}
        <span class="meta">{streamCount} stream{streamCount === 1 ? "" : "s"}</span>
      {/if}
      {#if connected}
        <span class="meta" title="Input events sent · display events received">
          ↑{inputSent} · ↓{displayRecv}
        </span>
      {/if}
    </span>
    <div class="actions">
      {#if connected && mouseMode === "server"}
        {#if grabbed}
          <button class="btn" onclick={release} title="Release pointer grab">Release</button>
        {:else}
          <button class="btn" onclick={grab} title="Grab pointer + keyboard">Grab</button>
        {/if}
      {/if}
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
        <input type="password" bind:value={passwordInput} placeholder="Enter the VM's SPICE password" autocomplete="off" autofocus />
      </label>
      <button type="submit" class="btn btn-primary" disabled={!passwordInput}>Connect</button>
    </form>
  {/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div
    class="canvas-wrap"
    class:grabbed
    class:focused={hasFocus}
    class:client-mode={mouseMode === "client"}
    bind:this={wrapperEl}
    tabindex="0"
    onfocus={onWrapperFocus}
    onblur={onWrapperBlur}
    onkeydown={keyHandler(true)}
    onkeyup={keyHandler(false)}
    onmousemove={mouseMove}
    onmousedown={mouseDown}
    onmouseup={mouseUp}
    onwheel={wheel}
    oncontextmenu={(e) => e.preventDefault()}
  >
    <div class="canvas-stack">
      <canvas bind:this={canvasEl}></canvas>
      <canvas bind:this={cursorEl} class="cursor-overlay"></canvas>
    </div>
    {#if connected && mouseMode === "server" && !grabbed}
      <div class="grab-overlay">
        <div class="grab-hint">Click to grab input · Ctrl+Alt+Shift to release</div>
      </div>
    {/if}
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

  .badge { display: inline-block; padding: 1px 8px; border-radius: 10px; font-size: 11px; font-weight: 500; }
  .badge.idle     { background: var(--bg-button); color: var(--text-muted); }
  .badge.focused  { background: rgba(251, 191, 36, 0.15); color: #fbbf24; }
  .badge.grabbed  { background: rgba(52, 211, 153, 0.15); color: #34d399; }
  .badge.err      { background: rgba(239, 68, 68, 0.15); color: #ef4444; }

  .actions { display: flex; gap: 6px; }
  .btn {
    padding: 4px 12px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--bg-button); color: var(--text); cursor: pointer;
    font-size: 12px; font-family: inherit;
  }
  .btn:hover { background: var(--bg-hover); }
  .btn-close:hover { background: #7f1d1d; color: #fca5a5; border-color: #7f1d1d; }
  .btn-primary {
    padding: 7px 14px; border: 1px solid var(--accent); border-radius: 6px;
    background: var(--accent); color: white; font-size: 13px; cursor: pointer;
  }
  .btn-primary:hover:not(:disabled) { filter: brightness(1.1); }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }

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
  .password-prompt label { display: flex; flex-direction: column; gap: 4px; flex: 1; }
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

  .canvas-wrap {
    flex: 1; overflow: hidden; background: #000; position: relative;
    display: flex; align-items: center; justify-content: center;
    outline: none;
    cursor: default;
  }
  .canvas-wrap.focused { box-shadow: inset 0 0 0 1px rgba(251, 191, 36, 0.4); }
  .canvas-wrap.grabbed { box-shadow: inset 0 0 0 2px rgba(52, 211, 153, 0.7); }
  .canvas-wrap.client-mode { box-shadow: inset 0 0 0 1px rgba(52, 211, 153, 0.3); }

  .canvas-stack {
    position: relative;
    max-width: 100%; max-height: 100%;
    display: inline-block;
  }
  canvas {
    display: block;
    max-width: 100%; max-height: 100%;
    image-rendering: crisp-edges;
  }
  .cursor-overlay {
    position: absolute;
    top: 0; left: 0;
    width: 100%; height: 100%;
    pointer-events: none;
  }

  .grab-overlay {
    position: absolute; inset: 0; pointer-events: none;
    display: flex; align-items: flex-end; justify-content: center;
    padding-bottom: 24px;
  }
  .grab-hint {
    background: rgba(22, 22, 42, 0.85);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 6px 14px;
    border-radius: 20px;
    font-size: 12px;
    backdrop-filter: blur(6px);
  }
</style>
