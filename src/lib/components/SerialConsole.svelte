<script>
  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";

  let { vmName, onClose } = $props();

  let containerEl = $state(null);
  let connected = $state(false);
  let error = $state(null);
  let unlisten = null;
  let term = null;
  let renderLoop = null;
  let blinkInterval = null;

  onMount(async () => {
    try {
      // Dynamic import — crytter ships as WASM + JS glue
      const crytter = await import("crytter-wasm");
      await crytter.default();

      term = new crytter.Terminal({
        fontFamily: "Menlo, Monaco, 'Courier New', monospace",
        fontSize: 14,
        cols: 120,
        rows: 30,
      });
      term.open(containerEl);
      term.fit();

      // Render loop (required — writes are batched via rAF)
      function frame() {
        if (term) term.render();
        renderLoop = requestAnimationFrame(frame);
      }
      renderLoop = requestAnimationFrame(frame);

      // Cursor blink
      blinkInterval = setInterval(() => { if (term) term.blinkCursor(); }, 530);

      // Keyboard input → send to VM
      const handleKey = async (e) => {
        if (!term || !connected) return;
        const data = term.handleKeyEvent(e);
        if (data != null) {
          e.preventDefault();
          // Convert string to bytes
          const bytes = Array.from(new TextEncoder().encode(data));
          try {
            await invoke("console_send", { data: bytes });
          } catch (err) {
            error = err?.message || String(err);
          }
        }
      };
      containerEl.addEventListener("keydown", handleKey);

      // Listen for data from the VM
      unlisten = await listen("console:data", (event) => {
        if (!term) return;
        const bytes = new Uint8Array(event.payload);
        // Use writeBytes for raw PTY data
        const response = term.writeBytes(bytes);
        // Send device query responses back to the VM
        if (response) {
          const respBytes = Array.from(new TextEncoder().encode(response));
          invoke("console_send", { data: respBytes }).catch(() => {});
        }
      });

      // Open the console
      await invoke("open_console", { name: vmName });
      connected = true;

      // Focus the terminal
      containerEl.focus();
    } catch (e) {
      console.error("Console error:", e);
      error = e?.message || JSON.stringify(e);
    }
  });

  onDestroy(async () => {
    if (renderLoop) cancelAnimationFrame(renderLoop);
    if (blinkInterval) clearInterval(blinkInterval);
    if (unlisten) unlisten();
    term = null;
    try {
      await invoke("close_console");
    } catch (_) {}
  });
</script>

<div class="console-container">
  <div class="console-toolbar">
    <span class="console-title">
      Serial Console — {vmName}
      {#if connected}
        <span class="badge-connected">Connected</span>
      {:else}
        <span class="badge-connecting">Connecting...</span>
      {/if}
    </span>
    <button class="btn-close" onclick={onClose}>Disconnect</button>
  </div>

  {#if error}
    <div class="console-error">{error}</div>
  {/if}

  <div
    class="terminal-container"
    bind:this={containerEl}
    tabindex="0"
    role="textbox"
    aria-label="Serial console terminal"
  ></div>
</div>

<style>
  .console-container {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #000;
  }

  .console-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 16px;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .console-title {
    font-size: 13px;
    font-weight: 500;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .badge-connected {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11px;
    background: rgba(52, 211, 153, 0.15);
    color: #34d399;
  }

  .badge-connecting {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11px;
    background: rgba(251, 191, 36, 0.15);
    color: #fbbf24;
  }

  .btn-close {
    padding: 4px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-button);
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
    font-family: inherit;
  }

  .btn-close:hover {
    background: #7f1d1d;
    color: #fca5a5;
    border-color: #7f1d1d;
  }

  .console-error {
    padding: 8px 16px;
    background: rgba(239, 68, 68, 0.1);
    border-bottom: 1px solid rgba(239, 68, 68, 0.3);
    color: #ef4444;
    font-size: 12px;
    flex-shrink: 0;
  }

  .terminal-container {
    flex: 1;
    overflow: hidden;
    outline: none;
  }
</style>
